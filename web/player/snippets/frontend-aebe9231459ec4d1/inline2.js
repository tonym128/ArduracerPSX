
export function embedEmit(kind, message) {
  const f = globalThis.psoxideEmbedEvent;
  if (typeof f === 'function') f(kind, message);
}
