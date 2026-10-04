
let _slots = {};
export function wsFetch(id, url) {
  if (_slots[id]) return;
  const s = { state: 'running', received: 0, total: 0 };
  _slots[id] = s;
  const xhr = new XMLHttpRequest();
  xhr.open('GET', url);
  xhr.responseType = 'arraybuffer';
  xhr.onprogress = (e) => { s.received = e.loaded; if (e.lengthComputable) s.total = e.total; };
  xhr.onload = () => {
    if (xhr.status >= 200 && xhr.status < 300) { s.buf = new Uint8Array(xhr.response); s.state = 'done'; }
    else { s.msg = url + ': HTTP ' + xhr.status; s.state = 'error'; }
  };
  xhr.onerror = () => { s.msg = url + ': network error'; s.state = 'error'; };
  xhr.send();
}
export function wsGunzip(id) {
  const s = _slots[id];
  if (!s || s.state !== 'done' || s.unzipping) return;
  // Some CDNs (itch.io's, for one) serve a .gz file with Content-Encoding:
  // gzip, so the browser has already inflated it by the time XHR hands it
  // over. The magic bytes say which case this is; the FNV check downstream
  // vouches for the payload either way.
  if (!(s.buf.length > 2 && s.buf[0] === 0x1f && s.buf[1] === 0x8b)) return;
  s.unzipping = true;
  s.state = 'running';
  (async () => {
    try {
      const out = await new Response(
        new Blob([s.buf]).stream().pipeThrough(new DecompressionStream('gzip'))
      ).arrayBuffer();
      s.buf = new Uint8Array(out);
      s.state = 'done';
    } catch (e) { s.msg = 'gunzip: ' + String(e); s.state = 'error'; }
  })();
}
export function wsState(id) { const s = _slots[id]; return s ? s.state : 'idle'; }
export function wsReceived(id) { const s = _slots[id]; return s ? s.received : 0; }
export function wsTotal(id) { const s = _slots[id]; return s ? s.total : 0; }
export function wsTake(id) { const s = _slots[id]; const b = s.buf; delete _slots[id]; return b; }
export function wsError(id) { const s = _slots[id]; const m = s ? s.msg : 'no slot'; delete _slots[id]; return m || 'fetch failed'; }
