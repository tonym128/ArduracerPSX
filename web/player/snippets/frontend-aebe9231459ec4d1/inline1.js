
const _img = new Map();
function _read(s, start, end) {
  if (s.blob) return s.blob.slice(start, end).arrayBuffer();
  return fetch(s.url, { headers: { Range: 'bytes=' + start + '-' + (end - 1) } }).then((r) => {
    if (r.status !== 206) throw new Error(s.url + ': HTTP ' + r.status + ' to a range request');
    return r.arrayBuffer();
  });
}
export function wdOpenBlob(id, blob) { _img.set(id, { blob, url: null, done: [] }); return blob.size; }
export async function wdOpenUrl(id, url) {
  const r = await fetch(url, { headers: { Range: 'bytes=0-0' } });
  if (r.status !== 206) return -1;
  const range = r.headers.get('content-range') || '';
  const total = Number(range.split('/')[1]);
  if (!(total > 0)) return -1;
  _img.set(id, { blob: null, url, done: [] });
  return total;
}
export function wdClose(id) { _img.delete(id); }
export function wdRead(id, start, end) {
  const s = _img.get(id);
  if (!s) return;
  _read(s, start, end).then(
    (b) => { s.done.push(start, new Uint8Array(b)); },
    (e) => { console.error('[psoxide] disc read', start, end, e); s.done.push(start, null); });
}
export function wdTake(id) {
  const s = _img.get(id);
  if (!s || s.done.length === 0) return null;
  const d = s.done;
  s.done = [];
  return d;
}
export async function wdReadNow(id, start, end) {
  const s = _img.get(id);
  if (!s) throw new Error('disc closed');
  return new Uint8Array(await _read(s, start, end));
}
