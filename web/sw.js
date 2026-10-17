/**
 * Arduracer PSX Web Arcade - Service Worker
 * Handles offline caching, byte-range slicing for disc streaming,
 * and automatic cache flushing when new ROM images are deployed.
 */

const SW_VERSION = '1.0.0';
const ROM_CACHE_NAME = 'arduracer-roms-v1';
const SHELL_CACHE_NAME = 'arduracer-shell-v1';

const SHELL_ASSETS = [
  './',
  'index.html',
  'style.css',
  'app.js',
  'rom-cache.js',
  'player/index.html',
  'player/frontend-1a97a093078bd962.js',
  'player/frontend-1a97a093078bd962_bg.wasm',
  'player/psoxide-player-build.json',
  'player/snippets/frontend-aebe9231459ec4d1/inline0.js',
  'player/snippets/frontend-aebe9231459ec4d1/inline1.js',
  'player/snippets/frontend-aebe9231459ec4d1/inline2.js',
  'player/snippets/frontend-aebe9231459ec4d1/inline3.js'
];

// 1. Lifecycle Events
self.addEventListener('install', (event) => {
  self.skipWaiting();
  event.waitUntil(
    caches.open(SHELL_CACHE_NAME).then((cache) => {
      // Best effort precache of shell assets
      return Promise.allSettled(
        SHELL_ASSETS.map((url) =>
          fetch(url).then((res) => {
            if (res.ok) return cache.put(url, res);
          })
        )
      );
    })
  );
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    (async () => {
      await self.clients.claim();
      // Remove deprecated cache versions if any
      const keys = await caches.keys();
      for (const key of keys) {
        if (
          key.startsWith('arduracer-') &&
          key !== ROM_CACHE_NAME &&
          key !== SHELL_CACHE_NAME
        ) {
          console.log('[SW] Deleting old cache:', key);
          await caches.delete(key);
        }
      }
    })()
  );
});

// 2. Helper: Handle Range Requests from Cached Response
async function handleCachedRangeRequest(request, cachedResponse) {
  const rangeHeader = request.headers.get('Range');
  const fullBlob = await cachedResponse.blob();
  const totalSize = fullBlob.size;
  const mimeType = cachedResponse.headers.get('Content-Type') || 'application/octet-stream';
  const etag = cachedResponse.headers.get('ETag') || `"${totalSize}"`;

  if (request.method === 'HEAD') {
    return new Response(null, {
      status: 200,
      headers: {
        'Content-Type': mimeType,
        'Content-Length': String(totalSize),
        'Accept-Ranges': 'bytes',
        'ETag': etag,
        'X-Arduracer-Cache': 'HIT'
      }
    });
  }

  if (!rangeHeader) {
    // Normal full GET request
    const responseHeaders = new Headers(cachedResponse.headers);
    responseHeaders.set('Accept-Ranges', 'bytes');
    responseHeaders.set('X-Arduracer-Cache', 'HIT');
    return new Response(fullBlob, {
      status: 200,
      headers: responseHeaders
    });
  }

  // Parse Range: bytes=start-end
  const matches = rangeHeader.trim().match(/^bytes=(\d*)-(\d*)$/);
  if (!matches) {
    return new Response(fullBlob, {
      status: 200,
      headers: {
        'Content-Type': mimeType,
        'Content-Length': String(totalSize),
        'Accept-Ranges': 'bytes',
        'X-Arduracer-Cache': 'HIT'
      }
    });
  }

  const startStr = matches[1];
  const endStr = matches[2];

  let start = startStr ? parseInt(startStr, 10) : 0;
  let end = endStr ? parseInt(endStr, 10) : totalSize - 1;

  if (start >= totalSize || end >= totalSize || start > end) {
    return new Response('', {
      status: 416,
      statusText: 'Requested Range Not Satisfiable',
      headers: {
        'Content-Range': `bytes */${totalSize}`
      }
    });
  }

  const slice = fullBlob.slice(start, end + 1);
  const headers = new Headers(cachedResponse.headers);
  headers.set('Content-Type', mimeType);
  headers.set('Content-Range', `bytes ${start}-${end}/${totalSize}`);
  headers.set('Content-Length', String(slice.size));
  headers.set('Accept-Ranges', 'bytes');
  headers.set('ETag', etag);
  headers.set('X-Arduracer-Cache', 'HIT-PARTIAL');

  return new Response(slice, {
    status: 206,
    statusText: 'Partial Content',
    headers: headers
  });
}

// 3. Cache Flush Helper
async function flushRomCache() {
  console.log('[SW] Flushing ROM cache:', ROM_CACHE_NAME);
  const deleted = await caches.delete(ROM_CACHE_NAME);
  // Re-create empty ROM cache
  await caches.open(ROM_CACHE_NAME);
  return deleted;
}

// 4. Message Communication with Main Thread
self.addEventListener('message', (event) => {
  const data = event.data;
  if (!data || !data.type) return;

  event.waitUntil(
    (async () => {
      switch (data.type) {
        case 'SKIP_WAITING': {
          self.skipWaiting();
          break;
        }

        case 'FLUSH_ROM_CACHE': {
          await flushRomCache();
          const clients = await self.clients.matchAll();
          for (const client of clients) {
            client.postMessage({ type: 'ROM_CACHE_FLUSHED' });
          }
          break;
        }

        case 'GET_CACHE_STATUS': {
          const romCache = await caches.open(ROM_CACHE_NAME);
          const requests = await romCache.keys();
          const cachedFiles = [];
          for (const req of requests) {
            const res = await romCache.match(req);
            if (res) {
              const blob = await res.blob();
              cachedFiles.push({
                url: req.url,
                size: blob.size,
                etag: res.headers.get('ETag') || ''
              });
            }
          }
          if (event.source) {
            event.source.postMessage({
              type: 'CACHE_STATUS_RESULT',
              cachedFiles: cachedFiles
            });
          }
          break;
        }

        case 'CACHE_ROM_FILE': {
          // Pre-cache a specific ROM URL into ROM_CACHE
          const url = data.url;
          if (!url) break;
          const romCache = await caches.open(ROM_CACHE_NAME);
          try {
            const res = await fetch(url);
            if (res.ok) {
              await romCache.put(url, res.clone());
              if (event.source) {
                event.source.postMessage({
                  type: 'ROM_FILE_CACHED',
                  url: url,
                  status: 'ok'
                });
              }
            }
          } catch (err) {
            if (event.source) {
              event.source.postMessage({
                type: 'ROM_FILE_CACHED',
                url: url,
                status: 'error',
                error: String(err)
              });
            }
          }
          break;
        }
      }
    })()
  );
});

// 5. Fetch Event Handler
self.addEventListener('fetch', (event) => {
  const url = new URL(event.request.url);

  // Only handle same-origin HTTP(S) requests
  if (url.origin !== self.location.origin) return;

  // A. ROMs and Disc Images Handling
  if (url.pathname.includes('/roms/')) {
    // 1. Manifest is always network-first to ensure fresh detection
    if (url.pathname.endsWith('manifest.json')) {
      event.respondWith(
        fetch(event.request, { cache: 'no-store' }).catch(async () => {
          const cache = await caches.open(ROM_CACHE_NAME);
          return (await cache.match(event.request)) || new Response('{}', { status: 404 });
        })
      );
      return;
    }

    // 2. ROM Disc Files (.bin, .cue, .iso, .exe)
    event.respondWith(
      (async () => {
        const romCache = await caches.open(ROM_CACHE_NAME);
        const cached = await romCache.match(event.request.url, { ignoreSearch: true });

        if (cached) {
          // Serve from cache (supporting HTTP 206 Range slicing)
          return handleCachedRangeRequest(event.request, cached);
        }

        // If not cached and request is a Range request (e.g. streaming 710 MB bin or 278 MB iso):
        // Pass through to network so the game doesn't stall waiting for full download!
        if (event.request.headers.has('range')) {
          return fetch(event.request);
        }

        // For non-range GET requests (e.g. cue, exe):
        // Fetch from network and auto-cache smaller files (< 10 MB)
        const networkResponse = await fetch(event.request);
        if (networkResponse.ok && event.request.method === 'GET') {
          const contentLength = networkResponse.headers.get('content-length');
          const size = contentLength ? parseInt(contentLength, 10) : 0;
          if (size > 0 && size < 10 * 1024 * 1024) {
            romCache.put(event.request.url, networkResponse.clone());
          }
        }
        return networkResponse;
      })()
    );
    return;
  }

  // B. Shell & Emulator Assets Handling
  event.respondWith(
    (async () => {
      // Try cache first for player wasm/js/css
      const shellCache = await caches.open(SHELL_CACHE_NAME);
      const cached = await shellCache.match(event.request);
      if (cached) {
        // Fetch in background to revalidate
        fetch(event.request).then((res) => {
          if (res.ok) shellCache.put(event.request, res);
        }).catch(() => {});
        return cached;
      }

      // Network fallback
      try {
        const res = await fetch(event.request);
        if (res.ok && event.request.method === 'GET') {
          shellCache.put(event.request, res.clone());
        }
        return res;
      } catch (err) {
        return cached || new Response('Offline', { status: 503 });
      }
    })()
  );
});
