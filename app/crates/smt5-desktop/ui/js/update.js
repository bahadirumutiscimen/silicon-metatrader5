// "A newer SiliconMetaTrader5 is available" — checks GitHub's latest release at most
// once a day (one anonymous request to api.github.com) and tells the user how
// to update for the way the app was installed. Nothing is downloaded.

import { call } from './api.js';
import { html, icon, copyText } from './dom.js';
import { t } from './i18n.js';

const REPO = 'bahadirumutiscimen/silicon-metatrader5';
const DAY = 24 * 3600 * 1000;
const KEY = 'update-check';

const parse = v => (String(v || '').replace(/^v/, '').split('-')[0].split('.').map(n => parseInt(n, 10) || 0).concat([0, 0, 0])).slice(0, 3);
export function newer(latest, current) {
  const a = parse(latest), b = parse(current);
  for (let i = 0; i < 3; i++) if (a[i] !== b[i]) return a[i] > b[i];
  return false;
}

function load() { try { return JSON.parse(localStorage.getItem(KEY) || 'null'); } catch { return null; } }
function save(v) { try { localStorage.setItem(KEY, JSON.stringify(v)); } catch { /* private mode */ } }

/** Latest release tag (cached for a day), or null when offline / no release yet. */
export async function latestRelease() {
  const c = load();
  if (c && Date.now() - c.at < DAY) return c.tag;
  try {
    const r = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`, { headers: { Accept: 'application/vnd.github+json' } });
    const tag = r.ok ? (await r.json()).tag_name || null : null;
    save({ at: Date.now(), tag });
    return tag;
  } catch {
    return c?.tag ?? null;
  }
}

/** Small sidebar card, or '' when up to date. */
export function updateCard(info, tag) {
  if (!tag || !newer(tag, info?.version)) return '';
  const mac = info?.os === 'macos';
  const how = info?.packaged
    ? (mac ? html`<code class="mono selectable">brew upgrade siliconmt5</code>` : html`${t('İndirme sayfasından yeni .deb / AppImage')}`)
    : html`<code class="mono selectable">git pull</code>${t(', sonra yeniden derle')}`;
  return html`<div class="updatebox">
    <div class="row" style="gap:6px">${icon('info')}<b style="font-size:12px">${t('Yeni sürüm: {tag}', { tag })}</b></div>
    <div class="muted" style="font-size:12px">${how}</div>
    <div class="row" style="gap:6px">
      ${info?.packaged && mac ? html`<button class="btn sm" data-upd="copy">${icon('copy')}${t('Kopyala')}</button>` : ''}
      <button class="btn sm" data-upd="open">${icon('ext')}${t('Notlar')}</button></div>
  </div>`;
}

export function bindUpdateCard(el, tag) {
  el.querySelector('[data-upd="copy"]')?.addEventListener('click', () => copyText('brew upgrade siliconmt5'));
  el.querySelector('[data-upd="open"]')?.addEventListener('click', () =>
    call('open_external', { target: `https://github.com/${REPO}/releases/tag/${tag}` }).catch(() => {}));
}

/** macOS + Homebrew: the copy in Applications is missing, a link, or older. */
export function copyCard(st) {
  if (!st?.brew || !st.needs_install) return '';
  const old = st.copy && !st.copy_is_link && st.copy_version;
  return html`<div class="updatebox">
    <div class="row" style="gap:6px">${icon('info')}<b style="font-size:12px">${old ? t("Uygulamalar'daki kopya eski ({from} → {to})", { from: st.copy_version, to: st.brew_version }) : t("Uygulamalar'a ekle")}</b></div>
    <div class="muted" style="font-size:12px">${old ? t('brew upgrade yeni sürümü kurdu; kopyayı güncelle.') : t("Spotlight ve Launchpad yalnız Uygulamalar'daki gerçek kopyayı bulur.")}</div>
    <button class="btn sm" data-copyapp>${icon('folder')}${old ? t('Güncelle ve yeniden aç') : t('Kopyala ve oradan aç')}</button>
  </div>`;
}

export function bindCopyCard(el, onError) {
  el.querySelector('[data-copyapp]')?.addEventListener('click', async e => {
    e.currentTarget.disabled = true;
    try { await call('app_copy_install', { relaunch: true }); }
    catch (err) { e.currentTarget.disabled = false; onError?.(String(err)); }
  });
}
