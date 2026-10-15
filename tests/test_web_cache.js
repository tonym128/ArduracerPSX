/**
 * Tests for Web Arcade Cache: Byte-range slicing and cache invalidation.
 */

const assert = require('assert');

// 1. Emulate the Service Worker Range Request Handler logic
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
    const responseHeaders = new Headers(cachedResponse.headers);
    responseHeaders.set('Accept-Ranges', 'bytes');
    responseHeaders.set('X-Arduracer-Cache', 'HIT');
    return new Response(fullBlob, {
      status: 200,
      headers: responseHeaders
    });
  }

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

async function runTests() {
  console.log('Running Web Arcade ROM Cache Tests...');

  const sampleSize = 10000;
  const sampleBuf = Buffer.alloc(sampleSize);
  for (let i = 0; i < sampleSize; i++) sampleBuf[i] = i % 256;

  const mockCachedResponse = {
    headers: new Headers([
      ['Content-Type', 'application/octet-stream'],
      ['ETag', '"test-etag-123"']
    ]),
    blob: async () => new Blob([sampleBuf])
  };

  // Test 1: HEAD request
  {
    const req = {
      method: 'HEAD',
      headers: new Headers()
    };

    const res = await handleCachedRangeRequest(req, mockCachedResponse);
    assert.strictEqual(res.status, 200);
    assert.strictEqual(res.headers.get('Content-Length'), '10000');
    assert.strictEqual(res.headers.get('Accept-Ranges'), 'bytes');
    console.log('✓ Test 1: HEAD request returns 200 with total length & Accept-Ranges');
  }

  // Test 2: PSoXide disc probe: Range bytes=0-0
  {
    const req = {
      method: 'GET',
      headers: new Headers([['Range', 'bytes=0-0']])
    };

    const res = await handleCachedRangeRequest(req, mockCachedResponse);
    assert.strictEqual(res.status, 206);
    assert.strictEqual(res.headers.get('Content-Range'), 'bytes 0-0/10000');
    assert.strictEqual(res.headers.get('Content-Length'), '1');
    const buf = Buffer.from(await res.arrayBuffer());
    assert.strictEqual(buf.length, 1);
    assert.strictEqual(buf[0], 0);
    console.log('✓ Test 2: PSoXide disc probe (bytes=0-0) returns HTTP 206 with 1 byte and Content-Range');
  }

  // Test 3: CD Sector Read: Range bytes=2048-4095 (2048 bytes)
  {
    const req = {
      method: 'GET',
      headers: new Headers([['Range', 'bytes=2048-4095']])
    };

    const res = await handleCachedRangeRequest(req, mockCachedResponse);
    assert.strictEqual(res.status, 206);
    assert.strictEqual(res.headers.get('Content-Range'), 'bytes 2048-4095/10000');
    assert.strictEqual(res.headers.get('Content-Length'), '2048');
    const buf = Buffer.from(await res.arrayBuffer());
    assert.strictEqual(buf.length, 2048);
    assert.strictEqual(buf[0], sampleBuf[2048]);
    assert.strictEqual(buf[2047], sampleBuf[4095]);
    console.log('✓ Test 3: Sector read (bytes=2048-4095) returns 2048 bytes with accurate slice data');
  }

  // Test 4: Open-ended Range bytes=5000-
  {
    const req = {
      method: 'GET',
      headers: new Headers([['Range', 'bytes=5000-']])
    };

    const res = await handleCachedRangeRequest(req, mockCachedResponse);
    assert.strictEqual(res.status, 206);
    assert.strictEqual(res.headers.get('Content-Range'), 'bytes 5000-9999/10000');
    assert.strictEqual(res.headers.get('Content-Length'), '5000');
    console.log('✓ Test 4: Open-ended Range (bytes=5000-) returns remaining bytes to end of file');
  }

  // Test 5: Out of bounds Range bytes=20000-25000
  {
    const req = {
      method: 'GET',
      headers: new Headers([['Range', 'bytes=20000-25000']])
    };

    const res = await handleCachedRangeRequest(req, mockCachedResponse);
    assert.strictEqual(res.status, 416);
    assert.strictEqual(res.headers.get('Content-Range'), 'bytes */10000');
    console.log('✓ Test 5: Out of bounds Range returns HTTP 416 Range Not Satisfiable');
  }

  // Test 6: Invalidation Check Logic
  {
    function checkShouldFlush(localBuildId, remoteBuildId) {
      if (!localBuildId || !remoteBuildId) return false;
      return localBuildId !== remoteBuildId;
    }

    assert.strictEqual(checkShouldFlush('build-v1', 'build-v1'), false);
    assert.strictEqual(checkShouldFlush('build-v1', 'build-v2'), true);
    assert.strictEqual(checkShouldFlush(null, 'build-v1'), false);
    console.log('✓ Test 6: Cache flush logic triggers on build_id mismatch');
  }

  console.log('\nAll 6 Web Arcade ROM Cache tests passed successfully!\n');
}

runTests().catch((err) => {
  console.error('Test failed:', err);
  process.exit(1);
});
