
let _dir = null;
function _idb() {
  return new Promise((res, rej) => {
    const r = indexedDB.open('psoxide-fs', 2);
    r.onupgradeneeded = () => {
      if (!r.result.objectStoreNames.contains('handles')) {
        r.result.createObjectStore('handles');
      }
      if (!r.result.objectStoreNames.contains('savestates')) {
        r.result.createObjectStore('savestates');
      }
    };
    r.onsuccess = () => res(r.result);
    r.onerror = () => rej(r.error);
  });
}
function _put(k, v) {
  return _idb().then(db => new Promise((res, rej) => {
    const t = db.transaction('handles', 'readwrite');
    t.objectStore('handles').put(v, k);
    t.oncomplete = () => res();
    t.onerror = () => rej(t.error);
  }));
}
function _get(k) {
  return _idb().then(db => new Promise((res, rej) => {
    const t = db.transaction('handles', 'readonly');
    const q = t.objectStore('handles').get(k);
    q.onsuccess = () => res(q.result);
    q.onerror = () => rej(q.error);
  }));
}
function _putState(k, v) {
  return _idb().then(db => new Promise((res, rej) => {
    const t = db.transaction('savestates', 'readwrite');
    t.objectStore('savestates').put(v, k);
    t.oncomplete = () => res();
    t.onerror = () => rej(t.error);
  }));
}
function _getState(k) {
  return _idb().then(db => new Promise((res, rej) => {
    const t = db.transaction('savestates', 'readonly');
    const q = t.objectStore('savestates').get(k);
    q.onsuccess = () => res(q.result);
    q.onerror = () => rej(q.error);
  }));
}
async function _list(dh) {
  const out = [];
  async function walk(h, pfx) {
    for await (const [name, ch] of h.entries()) {
      if (ch.kind === 'file') {
        const l = name.toLowerCase();
        if (l.endsWith('.bin') || l.endsWith('.exe')) out.push(pfx + name);
      } else if (ch.kind === 'directory') {
        await walk(ch, pfx + name + '/');
      }
    }
  }
  await walk(dh, '');
  return out.join('\n');
}
export function fsaSupported() {
  // The File System Access pickers throw SecurityError from cross-origin
  // iframes. itch embeds the player from html-classic.itch.zone, so use the
  // ordinary HTML file inputs there even when Chrome exposes these methods.
  return window.isSecureContext &&
         window.self === window.top &&
         ('showOpenFilePicker' in window) &&
         ('showDirectoryPicker' in window);
}
export async function pickFolder() {
  const dh = await window.showDirectoryPicker();
  await _put('folder', dh);
  _dir = dh;
  return await _list(dh);
}
// Gesture-free restore: only succeeds when the browser still reports the saved
// handle's permission as 'granted' (persistent permissions / installed PWA).
// Returns the data when granted, `false` when a handle exists but a user gesture
// is needed, `undefined` when there is no saved handle.
export async function autoFolder() {
  const dh = await _get('folder');
  if (!dh) return undefined;
  try {
    if ((await dh.queryPermission({ mode: 'read' })) === 'granted') {
      _dir = dh;
      return await _list(dh);
    }
  } catch (e) {}
  return false;
}
export async function reconnectFolder() {
  const dh = await _get('folder');
  if (!dh) return null;
  if ((await dh.requestPermission({ mode: 'read' })) !== 'granted') return null;
  _dir = dh;
  return await _list(dh);
}
export async function readGame(path) {
  let dh = _dir;
  if (!dh) {
    dh = await _get('folder');
    if (dh) { await dh.requestPermission({ mode: 'read' }); _dir = dh; }
  }
  if (!dh) return null;
  const parts = path.split('/');
  let h = dh;
  for (let i = 0; i < parts.length - 1; i++) h = await h.getDirectoryHandle(parts[i]);
  const fh = await h.getFileHandle(parts[parts.length - 1]);
  return await fh.getFile();
}
export async function saveQuickState(gameId, bytes, createdAt, cpuTick) {
  // Best effort: supporting browsers can exempt this origin from automatic
  // storage-pressure eviction. A denial does not prevent the normal save.
  try { if (navigator.storage?.persist) await navigator.storage.persist(); } catch (e) {}
  await _putState('quick:' + gameId, { bytes, createdAt, cpuTick });
}
export async function loadQuickState(gameId) {
  const state = await _getState('quick:' + gameId);
  return state ? state.bytes : undefined;
}
export async function inspectQuickState(gameId) {
  const state = await _getState('quick:' + gameId);
  return state ? state.createdAt + '\n' + state.cpuTick : undefined;
}
export async function fetchOpen(url) {
  const r = await fetch(url);
  if (!r.ok) throw new Error('HTTP ' + r.status);
  return [r.body.getReader(), Number(r.headers.get('content-length') || 0)];
}
export function readerNext(reader) { return reader.read(); }
export function downloadCsv(filenameStem, csv) {
  const safeStem = String(filenameStem || 'psoxide-input')
    .replace(/[^a-zA-Z0-9._-]+/g, '-')
    .replace(/^-+|-+$/g, '') || 'psoxide-input';
  const stamp = new Date().toISOString()
    .replace(/\.\d{3}Z$/, 'Z')
    .replace(/[:]/g, '-');
  const blob = new Blob([csv], { type: 'text/csv;charset=utf-8' });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = `${safeStem}-${stamp}.csv`;
  anchor.style.display = 'none';
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
  setTimeout(() => URL.revokeObjectURL(url), 0);
}
