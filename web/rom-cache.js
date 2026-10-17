/**
 * Arduracer PSX Web Arcade - ROM Cache Manager
 * Manages Service Worker registration, ROM caching in CacheStorage,
 * progress tracking for disc downloads, and automatic flushing when new images are deployed.
 */

class RomCacheManager {
  constructor() {
    this.ROM_CACHE_NAME = 'arduracer-roms-v1';
    this.STORAGE_KEY_BUILD = 'arduracer_rom_build_id';
    this.STORAGE_KEY_TIMESTAMP = 'arduracer_rom_cached_at';
    this.STORAGE_KEY_MANIFEST = 'arduracer_rom_manifest';

    this.swRegistration = null;
    this.listeners = new Set();
    this.isSupported = 'serviceWorker' in navigator && 'caches' in window;
    this.activeDownloads = new Map(); // url -> AbortController
  }

  /**
   * Initialize Service Worker and check for ROM updates.
   */
  async init() {
    if (!this.isSupported) {
      console.warn('[RomCache] ServiceWorker or CacheStorage not supported in this browser.');
      return;
    }

    try {
      this.swRegistration = await navigator.serviceWorker.register('./sw.js', {
        scope: './'
      });
      console.log('[RomCache] Service Worker registered with scope:', this.swRegistration.scope);

      // Listen for messages from Service Worker
      navigator.serviceWorker.addEventListener('message', (event) => {
        if (!event.data) return;
        if (event.data.type === 'ROM_CACHE_FLUSHED') {
          this._notify('cache-flushed', { reason: 'sw_message' });
        }
      });

      // Check for updates on startup
      await this.checkForUpdates();
    } catch (err) {
      console.error('[RomCache] Failed to register Service Worker:', err);
    }
  }

  /**
   * Subscribe to cache events (e.g. 'new-image', 'cache-flushed', 'progress', 'status-change')
   */
  subscribe(callback) {
    this.listeners.add(callback);
    return () => this.listeners.delete(callback);
  }

  _notify(event, data) {
    for (const cb of this.listeners) {
      try {
        cb(event, data);
      } catch (e) {
        console.error('[RomCache] Listener error:', e);
      }
    }
  }

  /**
   * Check if a new ROM image build has been deployed.
   * If detected, automatically flushes outdated caches.
   */
  async checkForUpdates() {
    if (!this.isSupported) return false;

    try {
      // 1. Fetch remote manifest with cache-busting timestamp
      const manifestUrl = `roms/manifest.json?_t=${Date.now()}`;
      const response = await fetch(manifestUrl, { cache: 'no-store' });

      if (response.ok) {
        const remoteManifest = await response.json();
        const localBuildId = localStorage.getItem(this.STORAGE_KEY_BUILD);

        if (localBuildId && remoteManifest.build_id && localBuildId !== remoteManifest.build_id) {
          console.warn(
            `[RomCache] New ROM image detected! Local: ${localBuildId}, Remote: ${remoteManifest.build_id}. Flushing outdated cache...`
          );

          // Flush outdated cache!
          await this.flushCache();

          // Store new build ID and manifest
          localStorage.setItem(this.STORAGE_KEY_BUILD, remoteManifest.build_id);
          localStorage.setItem(this.STORAGE_KEY_MANIFEST, JSON.stringify(remoteManifest));
          localStorage.setItem(this.STORAGE_KEY_TIMESTAMP, new Date().toISOString());

          this._notify('new-image', {
            oldBuildId: localBuildId,
            newBuildId: remoteManifest.build_id,
            manifest: remoteManifest
          });

          return true; // New image detected & flushed
        }

        // First run or unchanged
        if (!localBuildId && remoteManifest.build_id) {
          localStorage.setItem(this.STORAGE_KEY_BUILD, remoteManifest.build_id);
          localStorage.setItem(this.STORAGE_KEY_MANIFEST, JSON.stringify(remoteManifest));
          localStorage.setItem(this.STORAGE_KEY_TIMESTAMP, new Date().toISOString());
        }

        return false;
      }
    } catch (err) {
      console.log('[RomCache] Manifest check failed or offline, trying HEAD check:', err);
    }

    // 2. Fallback: Check ETag or Last-Modified of arduracer.cue / arduracer.bin
    try {
      const headRes = await fetch('roms/arduracer.cue', { method: 'HEAD', cache: 'no-store' });
      if (headRes.ok) {
        const currentEtag = headRes.headers.get('ETag') || headRes.headers.get('Last-Modified');
        const storedEtag = localStorage.getItem('arduracer_rom_etag');
        if (storedEtag && currentEtag && storedEtag !== currentEtag) {
          console.warn('[RomCache] ETag mismatch detected! Flushing stale cache...');
          await this.flushCache();
          localStorage.setItem('arduracer_rom_etag', currentEtag);
          this._notify('new-image', { reason: 'etag_change', etag: currentEtag });
          return true;
        }
        if (currentEtag) {
          localStorage.setItem('arduracer_rom_etag', currentEtag);
        }
      }
    } catch (_) {}

    return false;
  }

  /**
   * Get the parsed manifest if available.
   */
  async getManifest() {
    try {
      const stored = localStorage.getItem(this.STORAGE_KEY_MANIFEST);
      if (stored) {
        return JSON.parse(stored);
      }
    } catch (_) {}

    try {
      const res = await fetch(`roms/manifest.json?_t=${Date.now()}`);
      if (res.ok) {
        const manifest = await res.json();
        if (manifest && manifest.build_id) {
          localStorage.setItem(this.STORAGE_KEY_MANIFEST, JSON.stringify(manifest));
        }
        return manifest;
      }
    } catch (_) {}

    return null;
  }

  /**
   * Flush all cached ROM disc images.
   */
  async flushCache() {
    if (!('caches' in window)) return false;

    console.log('[RomCache] Flushing ROM cache...');
    let deleted = false;
    try {
      deleted = await caches.delete(this.ROM_CACHE_NAME);
    } catch (e) {
      console.error('[RomCache] Error deleting cache:', e);
    }

    // Inform Service Worker
    if (navigator.serviceWorker && navigator.serviceWorker.controller) {
      navigator.serviceWorker.controller.postMessage({ type: 'FLUSH_ROM_CACHE' });
    }

    // Reopen clean cache
    await caches.open(this.ROM_CACHE_NAME);

    // Clear active downloads
    for (const [url, controller] of this.activeDownloads.entries()) {
      controller.abort();
    }
    this.activeDownloads.clear();

    this._notify('cache-flushed', { deleted: deleted });
    return deleted;
  }

  /**
   * Query status of all cached files in the ROM cache.
   */
  async getStatus() {
    if (!this.isSupported) {
      return { isSupported: false, totalBytes: 0, files: [] };
    }

    try {
      const cache = await caches.open(this.ROM_CACHE_NAME);
      const requests = await cache.keys();
      const files = [];
      let totalBytes = 0;

      for (const req of requests) {
        const res = await cache.match(req);
        if (res) {
          const blob = await res.blob();
          const filename = req.url.split('/').pop().split('?')[0];
          files.push({
            url: req.url,
            filename: filename,
            size: blob.size,
            type: blob.type
          });
          totalBytes += blob.size;
        }
      }

      const buildId = localStorage.getItem(this.STORAGE_KEY_BUILD) || 'unknown';
      const cachedAt = localStorage.getItem(this.STORAGE_KEY_TIMESTAMP);

      return {
        isSupported: true,
        buildId: buildId,
        cachedAt: cachedAt,
        totalBytes: totalBytes,
        files: files
      };
    } catch (e) {
      console.error('[RomCache] getStatus error:', e);
      return { isSupported: true, totalBytes: 0, files: [] };
    }
  }

  /**
   * Check if a specific ROM URL or filename is cached.
   */
  async isCached(urlOrFilename) {
    if (!this.isSupported) return false;
    try {
      const cache = await caches.open(this.ROM_CACHE_NAME);
      const requests = await cache.keys();
      return requests.some((req) => req.url.includes(urlOrFilename));
    } catch (_) {
      return false;
    }
  }

  /**
   * Pre-cache a ROM image with live progress tracking.
   * Can be aborted via cancelDownload().
   */
  async cacheRom(url, onProgress) {
    if (!this.isSupported) throw new Error('CacheStorage not supported');

    // Cancel existing download for this URL if any
    if (this.activeDownloads.has(url)) {
      this.activeDownloads.get(url).abort();
      this.activeDownloads.delete(url);
    }

    const controller = new AbortController();
    this.activeDownloads.set(url, controller);

    try {
      const response = await fetch(url, {
        signal: controller.signal
      });

      if (!response.ok) {
        throw new Error(`Failed to fetch ${url}: HTTP ${response.status}`);
      }

      const contentLength = response.headers.get('content-length');
      const total = contentLength ? parseInt(contentLength, 10) : 0;
      let loaded = 0;

      const reader = response.body.getReader();
      const chunks = [];

      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        chunks.push(value);
        loaded += value.byteLength;

        const percent = total > 0 ? Math.min(100, Math.round((loaded / total) * 100)) : 0;
        if (onProgress) {
          onProgress({ loaded, total, percent });
        }
        this._notify('progress', { url, loaded, total, percent });
      }

      // Assemble complete blob and put into cache
      const mimeType = response.headers.get('content-type') || 'application/octet-stream';
      const blob = new Blob(chunks, { type: mimeType });
      const cache = await caches.open(this.ROM_CACHE_NAME);

      const cacheHeaders = new Headers(response.headers);
      cacheHeaders.set('Content-Length', String(blob.size));
      cacheHeaders.set('Accept-Ranges', 'bytes');

      const fullUrl = new URL(url, window.location.href).href;
      await cache.put(
        fullUrl,
        new Response(blob, {
          status: 200,
          statusText: 'OK',
          headers: cacheHeaders
        })
      );

      this.activeDownloads.delete(url);
      this._notify('status-change', { url, status: 'cached', size: blob.size });
      return { success: true, size: blob.size };
    } catch (err) {
      this.activeDownloads.delete(url);
      if (err.name === 'AbortError') {
        console.log(`[RomCache] Download aborted: ${url}`);
        return { success: false, aborted: true };
      }
      throw err;
    }
  }

  /**
   * Cancel an in-progress ROM download.
   */
  cancelDownload(url) {
    if (this.activeDownloads.has(url)) {
      this.activeDownloads.get(url).abort();
      this.activeDownloads.delete(url);
      this._notify('status-change', { url, status: 'cancelled' });
    }
  }

  /**
   * Format bytes into human-readable string (KB, MB, GB).
   */
  static formatBytes(bytes, decimals = 1) {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const dm = decimals < 0 ? 0 : decimals;
    const sizes = ['B', 'KB', 'MB', 'GB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(dm)) + ' ' + sizes[i];
  }
}

// Global singleton instance
window.romCache = new RomCacheManager();
