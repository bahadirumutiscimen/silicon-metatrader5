// Bridge to the Rust commands. Without Tauri (plain browser, UI development)
// a mock backend with realistic data is loaded instead.

const T = window.__TAURI__;
let mock = null;
if (!T) mock = await import('./mock.js');

export const isTauri = !!T;

export async function call(cmd, args = {}) {
  if (mock) return mock.call(cmd, args);
  return T.core.invoke(cmd, args);
}

export async function on(event, fn) {
  if (mock) return mock.on(event, fn);
  return T.event.listen(event, e => fn(e.payload));
}

export async function pickFolder() {
  if (mock) return '/Users/demo/Documents/GitHub/silicon-metatrader5';
  const opts = { directory: true, multiple: false, title: 'SiliconMetaTrader5 klasörünü seç' };
  if (T.dialog?.open) return T.dialog.open(opts);
  return T.core.invoke('plugin:dialog|open', { options: opts });
}

export async function startDrag() {
  if (!T) return;
  try { await T.window.getCurrentWindow().startDragging(); } catch { /* not supported */ }
}
