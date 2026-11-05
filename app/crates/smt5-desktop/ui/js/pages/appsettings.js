// Uygulama Ayarları: what belongs to the app itself, not to a runtime —
// closing the window, the runtime folder, starting the setup over.

import { call, pickFolder } from '../api.js';
import { $, html, render, icon, toast } from '../dom.js';
import { state, refreshInfo } from '../store.js';
import { t } from '../i18n.js';

let el = null;

function draw() {
  if (!el) return;
  const ca = state.info?.close_action ?? null;
  render(el, html`
    <div class="head"><div><div class="crumbs">SiliconMetaTrader5 › <b>${t('Uygulama Ayarları')}</b></div><h1>${t('Uygulama Ayarları')}</h1>
      <div class="muted" style="margin-top:4px;max-width:760px">${t('Uygulamanın kendi ayarları; MT5 ve runtime ayarları Runtime Ayarları sayfasında.')}</div></div></div>
    <div class="grid" style="grid-template-columns:repeat(auto-fit,minmax(320px,1fr));align-items:start">
      <section class="card pad col">
        <h2>${t('Pencereyi kapatınca')}</h2>
        <div class="seg full" id="st-close" role="group" aria-label="${t('Pencereyi kapatınca')}">
          ${[[null, t('Sor')], ...(state.info?.tray ? [['tray', navigator.userAgent.includes('Mac') ? t('Menü çubuğu') : t('Tepsi')]] : []), ['quit', t('Tamamen kapat')]].map(([v, label]) =>
            html`<button data-v="${v ?? ''}" class="${ca === v ? 'sel' : ''}">${label}</button>`)}
        </div>
        <p class="muted" style="margin:0;font-size:12px">${ca === 'tray' ? t('Pencere kapanır, MT5 çalışmaya devam eder; menü çubuğu simgesinden yönetirsin.')
          : ca === 'quit' ? t('MT5 ve köprü durdurulur, sonra uygulama çıkar.')
          : t('Her kapatışta ne yapılacağı sorulur.')} ${t('Sunucular her durumda çalışmaya devam eder.')}</p>
      </section>
      <section class="card pad col">
        <h2>${t('Runtime klasörü')}</h2>
        <div class="mono muted" style="word-break:break-all">${state.info?.runtime_dir || '—'}</div>
        <div class="row"><button class="btn sm" id="st-folder">${icon('folder')}${t('Değiştir…')}</button><button class="btn sm" id="st-open">${icon('ext')}${navigator.userAgent.includes('Mac') ? t("Finder'da göster") : t('Klasörü aç')}</button></div>
      </section>
      <section class="card pad col">
        <h2>${t('Kurulumu baştan başlat')}</h2>
        <p class="muted" style="margin:0;font-size:13px">${t('Sihirbazı ilk adımdan açar: dil, sistem, mod, bileşenler, giriş ve veri testi. Kurulu olanlar ve sunucular korunur.')}</p>
        <div class="row"><button class="btn" id="st-wizard">${icon('refresh')}${t('Sihirbazı yeniden başlat')}</button></div>
      </section>
    </div>`);
  $('#st-close', el).onclick = async e => {
    const b = e.target.closest('button'); if (!b) return;
    try { await call('set_close_action', { action: b.dataset.v || null }); await refreshInfo(); draw(); } catch (err) { toast(String(err), true); }
  };
  $('#st-folder', el).onclick = async () => {
    const p = await pickFolder(); if (!p) return;
    try { await call('set_runtime_dir', { path: p }); await refreshInfo(); toast(t('Runtime klasörü değişti')); draw(); } catch (e) { toast(String(e), true); }
  };
  $('#st-open', el).onclick = () => call('open_external', { target: state.info.runtime_dir }).catch(e => toast(String(e), true));
  $('#st-wizard', el).onclick = async () => {
    try { await call('set_setup_complete', { done: false }); location.hash = '#/setup'; location.reload(); } catch (e) { toast(String(e), true); }
  };
}

export default {
  mount(node) { el = node; draw(); },
  update() {},
  unmount() { el = null; },
};
