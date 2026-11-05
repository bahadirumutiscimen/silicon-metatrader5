// Small DOM + formatting helpers shared by every page.

import { t, locale } from './i18n.js';

export const $ = (sel, root = document) => root.querySelector(sel);
export const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];

export function esc(v) {
  return String(v ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
}

/** Tagged template that escapes interpolations unless wrapped with raw(). */
export function html(strings, ...vals) {
  let out = '';
  strings.forEach((s, i) => {
    out += s;
    if (i < vals.length) {
      const v = vals[i];
      if (v == null || v === false) return;
      if (Array.isArray(v)) out += v.map(x => (x && x.__raw ? x.s : esc(x))).join('');
      else out += v && v.__raw ? v.s : esc(v);
    }
  });
  return { __raw: true, s: out };
}
export const raw = s => ({ __raw: true, s: String(s) });
export function render(el, tpl) { el.innerHTML = tpl.__raw ? tpl.s : esc(tpl); }

// ---------- icons (stroke, 24 grid) ----------
const P = {
  grid: '<rect x="3" y="3" width="7" height="9" rx="1.5"/><rect x="14" y="3" width="7" height="5" rx="1.5"/><rect x="14" y="12" width="7" height="9" rx="1.5"/><rect x="3" y="16" width="7" height="5" rx="1.5"/>',
  screen: '<rect x="3" y="4" width="18" height="12" rx="2"/><path d="M8 20h8M12 16v4"/>',
  logs: '<path d="M4 6h16M4 12h16M4 18h10"/>',
  sliders: '<path d="M4 7h10M18 7h2M4 17h4M12 17h8"/><circle cx="16" cy="7" r="2"/><circle cx="10" cy="17" r="2"/>',
  db: '<ellipse cx="12" cy="6" rx="7" ry="3"/><path d="M5 6v12c0 1.7 3.1 3 7 3s7-1.3 7-3V6M5 12c0 1.7 3.1 3 7 3s7-1.3 7-3"/>',
  wand: '<circle cx="12" cy="12" r="9"/><path d="m8.5 12 2.5 2.5 4.5-5"/>',
  pulse: '<path d="M4 12h4l2-5 4 10 2-5h4"/>',
  wallet: '<rect x="3" y="6" width="18" height="13" rx="2"/><path d="M3 10h18M7 15h3"/>',
  trend: '<path d="M3 17l6-6 4 4 8-8"/><path d="M15 7h6v6"/>',
  clock: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>',
  refresh: '<path d="M20 12a8 8 0 1 1-2.3-5.6"/><path d="M20 4v5h-5"/>',
  play: '<path d="M7 5v14l11-7z"/>',
  stop: '<rect x="6" y="6" width="12" height="12" rx="2"/>',
  ext: '<path d="M14 4h6v6M20 4l-9 9"/><path d="M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5"/>',
  window: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M3 9h18"/>',
  full: '<path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5"/>',
  check: '<path d="m5 12 5 5 9-10"/>',
  x: '<path d="M6 6l12 12M18 6 6 18"/>',
  alert: '<path d="M12 3 2 20h20z"/><path d="M12 10v4M12 17v.5"/>',
  info: '<circle cx="12" cy="12" r="9"/><path d="M12 11v6M12 7.5v.5"/>',
  copy: '<rect x="8" y="8" width="12" height="12" rx="2"/><path d="M16 8V5a1 1 0 0 0-1-1H5a1 1 0 0 0-1 1v10a1 1 0 0 0 1 1h3"/>',
  trash: '<path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13"/>',
  eye: '<path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12z"/><circle cx="12" cy="12" r="3"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  search: '<circle cx="11" cy="11" r="6"/><path d="m20 20-4.5-4.5"/>',
  server: '<rect x="4" y="3" width="16" height="7" rx="1.5"/><rect x="4" y="14" width="16" height="7" rx="1.5"/><path d="M8 6.5h.01M8 17.5h.01M12 6.5h4M12 17.5h4"/>',
  plug: '<path d="M9 3v5M15 3v5M6 8h12v3a6 6 0 0 1-12 0V8ZM12 17v4"/>',
  key: '<circle cx="8" cy="15" r="4"/><path d="m11 12 9-9M17 6l3 3"/>',
  folder: '<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>',
  chip: '<rect x="6" y="6" width="12" height="12" rx="2"/><path d="M9 2v4M15 2v4M9 18v4M15 18v4M2 9h4M2 15h4M18 9h4M18 15h4"/>',
  cpu: '<rect x="4" y="4" width="16" height="16" rx="2"/><rect x="9" y="9" width="6" height="6"/>',
  spinner: '<path d="M12 3a9 9 0 1 0 9 9" />',
  bars: '<path d="M7 20V10M12 20V4M17 20v-7"/>',
  chev: '<path d="m9 6 6 6-6 6"/>',
  laptop: '<rect x="4" y="5" width="16" height="11" rx="1.5"/><path d="M2 19h20"/>',
  terminal: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="m7 9 3 3-3 3M12 15h5"/>',
  book: '<path d="M4 5a2 2 0 0 1 2-2h13v15H6a2 2 0 0 0-2 2z"/><path d="M4 20a2 2 0 0 0 2 2h13v-4M9 7h6"/>',
  min: '<path d="M6 12h12"/>',
};
export function icon(name, cls = '') {
  return raw(`<svg class="${cls}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${P[name] || ''}</svg>`);
}
export const okIcon = () => icon('check');

// ---------- formatting ----------
export function bytes(n) {
  if (n == null) return '—';
  const u = ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0, v = Number(n);
  while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
  return `${v >= 100 || i === 0 ? v.toFixed(0) : v.toFixed(1)} ${u[i]}`;
}
export function money(v, cur = 'USD') {
  if (v == null || Number.isNaN(v)) return '—';
  try { return new Intl.NumberFormat(locale(), { style: 'currency', currency: cur || 'USD', maximumFractionDigits: 2 }).format(v); }
  catch { return `${Number(v).toFixed(2)} ${cur || ''}`; }
}
export function num(v, d = 2) {
  if (v == null || Number.isNaN(v)) return '—';
  return new Intl.NumberFormat(locale(), { minimumFractionDigits: d, maximumFractionDigits: d }).format(v);
}
export function duration(sec) {
  if (sec == null || sec < 0) return '—';
  sec = Math.floor(sec);
  if (sec < 60) return t('{n} sn', { n: sec });
  const m = Math.floor(sec / 60), h = Math.floor(m / 60), d = Math.floor(h / 24);
  if (d) return t('{d} g {h} sa', { d, h: h % 24 });
  if (h) return t('{h} sa {m} dk', { h, m: m % 60 });
  return t('{m} dk', { m });
}
export function clock(ts) {
  if (!ts) return '—';
  return new Date(ts * 1000).toLocaleTimeString(locale(), { hour: '2-digit', minute: '2-digit', second: '2-digit' });
}

// ---------- feedback ----------
export function toast(msg, bad = false) {
  let box = $('.toasts');
  if (!box) { box = document.createElement('div'); box.className = 'toasts'; document.body.append(box); }
  const t = document.createElement('div');
  t.className = 'toast' + (bad ? ' bad' : '');
  t.textContent = msg;
  box.append(t);
  setTimeout(() => t.remove(), bad ? 7000 : 3500);
}

/** Modal dialog. `body` is an html`` template; resolves with the clicked button value. */
export function modal({ title, body, buttons = [{ label: t('Tamam'), value: true, primary: true }], onMount }) {
  return new Promise(resolve => {
    const bg = document.createElement('div');
    bg.className = 'modal-bg';
    render(bg, html`<div class="modal" role="dialog" aria-modal="true"><h2>${title}</h2><div class="col">${body}</div>
      <div class="row" style="justify-content:flex-end">${buttons.map((b, i) => html`<button class="btn ${b.primary ? 'p' : ''} ${b.danger ? 'danger' : ''}" data-i="${i}">${b.label}</button>`)}</div></div>`);
    const close = v => { bg.remove(); resolve(v); };
    bg.addEventListener('click', e => {
      if (e.target === bg) return close(null);
      const b = e.target.closest('button[data-i]');
      if (b && !b.disabled) close(buttons[+b.dataset.i].value);
    });
    document.addEventListener('keydown', function k(e) { if (e.key === 'Escape') { document.removeEventListener('keydown', k); close(null); } });
    document.body.append(bg);
    onMount?.(bg);
  });
}

export async function copyText(text) {
  try { await navigator.clipboard.writeText(text); toast(t('Panoya kopyalandı')); }
  catch { toast(t('Kopyalanamadı'), true); }
}
