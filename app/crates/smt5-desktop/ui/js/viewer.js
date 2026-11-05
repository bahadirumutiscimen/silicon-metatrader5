// The MT5 screen inside the app: noVNC's RFB client (ui/vendor/novnc, MPL-2.0,
// unmodified) draws on our own page instead of an iframe, so the app owns the
// keyboard. On a Mac host ⌘C/⌘V/… reach MT5 as Ctrl+C/Ctrl+V/…, and ⌘ alone
// never reaches the server (it would be the Windows / Super key there).
import { call, isTauri } from './api.js';

export const XK_Control_L = 0xffe3;
// ⌘ + these letters → Ctrl + the letter in MT5 (copy, paste, cut, select all, undo, redo, find, save)
const MAPPED = new Set(['KeyA', 'KeyC', 'KeyV', 'KeyX', 'KeyZ', 'KeyY', 'KeyF', 'KeyS']);

/** What to do with a key event on a Mac host. null: let noVNC handle it.
 *  {swallow}: drop it (⌘ itself) · {host}: leave it to the Mac (⌘Q, ⌘Tab, …)
 *  · {ctrl: 'c'}: send Ctrl+C to MT5 instead. */
export function macKey(ev) {
  if (ev.key === 'Meta' || ev.code === 'MetaLeft' || ev.code === 'MetaRight' || ev.code === 'OSLeft' || ev.code === 'OSRight') return { swallow: true };
  if (!ev.metaKey) return null;
  if (MAPPED.has(ev.code)) return { ctrl: ev.code.slice(3).toLowerCase() };
  return { host: true };
}

/** The websockify address and VNC password in a snapshot's display_url
 *  (http://127.0.0.1:PORT/vnc.html?…#password=…; the fragment never leaves the page). */
export function wsTarget(displayUrl) {
  let u;
  try { u = new URL(displayUrl); } catch { return { url: '', password: '' }; }
  if (!/^https?:$/.test(u.protocol) || !u.port) return { url: '', password: '' };
  const frag = new URLSearchParams(u.hash.replace(/^#/, ''));
  return { url: `ws://${u.hostname}:${u.port}/websockify`, password: frag.get('password') || u.searchParams.get('password') || '' };
}

/** Mount the screen in `box`. Returns {destroy}. `mac`: remap ⌘ to Ctrl. */
export function mountViewer(box, displayUrl, { mac = false, title = 'MetaTrader 5' } = {}) {
  const { url, password } = wsTarget(displayUrl);
  const target = document.createElement('div');
  target.className = 'vnc';
  target.setAttribute('role', 'application');
  target.setAttribute('aria-label', title);
  target.dataset.ws = url;
  box.replaceChildren(target);
  let rfb = null, stopped = false, timer = null;

  const onKey = async ev => {
    const a = macKey(ev);
    if (!a) return;
    ev.stopPropagation(); // before noVNC's own listener on its canvas
    if (a.host) return; // the Mac's shortcut (no preventDefault)
    ev.preventDefault();
    if (a.swallow || ev.type !== 'keydown') return;
    // the Mac clipboard reaches MT5 before the paste, not 1–2 s later
    if (a.ctrl === 'v') await call('clipboard_push').catch(() => {});
    if (!rfb) return;
    const ks = a.ctrl.charCodeAt(0), code = 'Key' + a.ctrl.toUpperCase();
    rfb.sendKey(XK_Control_L, 'ControlLeft', true);
    rfb.sendKey(ks, code, true);
    rfb.sendKey(ks, code, false);
    rfb.sendKey(XK_Control_L, 'ControlLeft', false);
  };
  if (mac) {
    target.addEventListener('keydown', onKey, true);
    target.addEventListener('keyup', onKey, true);
  }

  const connect = async () => {
    if (stopped || !url) return;
    if (!isTauri) { target.dataset.mock = '1'; return; } // browser UI development: no VNC server
    const { default: RFB } = await import('../vendor/novnc/core/rfb.js');
    if (stopped) return;
    rfb = new RFB(target, url, { credentials: { password } });
    rfb.scaleViewport = true;
    rfb.resizeSession = false;
    rfb.focusOnClick = true;
    rfb.addEventListener('connect', () => { target.dataset.state = 'connected'; rfb.focus(); });
    rfb.addEventListener('credentialsrequired', () => rfb.sendCredentials({ password }));
    rfb.addEventListener('disconnect', () => {
      target.dataset.state = 'disconnected';
      rfb = null;
      if (!stopped) timer = setTimeout(connect, 2000); // the display stack restarts / the tunnel reconnects
    });
  };
  connect().catch(e => { target.dataset.state = 'error'; target.dataset.error = String(e); });

  return {
    destroy() {
      stopped = true;
      clearTimeout(timer);
      try { rfb?.disconnect(); } catch { /* already closed */ }
      target.remove();
    },
  };
}

/** The separate "MetaTrader 5" window (index.html?viewer=1): only the screen,
 *  following the active target. */
export async function startViewerWindow() {
  document.title = 'SiliconMetaTrader5 — MetaTrader 5';
  document.body.style.margin = '0';
  document.body.style.background = '#101216';
  document.body.innerHTML = '<div class="vncbox" id="w-vnc" style="height:100vh"></div>';
  const box = document.getElementById('w-vnc');
  const info = await call('app_info').catch(() => null);
  let cur = null, v = null;
  const load = async () => {
    const s = await call('snapshot').catch(() => null);
    const url = s?.display_url || null;
    if (url === cur) return;
    cur = url;
    v?.destroy();
    v = url ? mountViewer(box, url, { mac: info?.os === 'macos' }) : null;
  };
  await load();
  setInterval(load, 5000); // another target, or the display came up
}
