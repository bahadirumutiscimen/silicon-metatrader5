// Tray panel: the small window under the menu-bar icon (left click).
// Runtime state, account, Start/Stop, MT5 window, open the app, quit.

import { call, on } from './api.js';
import { $, $$, html, render, icon, money, duration, toast } from './dom.js';
import { state, subscribe, phaseOf, account, refreshInfo, refreshSnapshot, nativeWindow } from './store.js';
import { t, setLang, setHost } from './i18n.js';

let srv = null; // server cards (Server mode), refreshed less often
const problems = () => (srv || []).filter(c => c.id !== 'local' && (c.link?.state === 'failed' || ['error', 'unreachable'].includes(c.lifecycle))).length;

function draw() {
  const s = state.snap, ph = phaseOf(), a = account(), job = state.job?.running ? state.job : null;
  const target = (state.info?.servers || 0) > 0 ? (s?.target_name || t('Bu bilgisayar')) : '';
  const tone = { up: '', warn: 'mid', down: 'bad', acc: 'mid', neu: 'off' }[ph.tone] ?? 'off';
  const since = s?.status?.supervisor?.updated;
  const active = ph.group === 'on' || ph.group === 'busy';
  render(document.body, html`<div class="panel">
    <div class="row between"><div class="row" style="gap:8px"><img class="logo-img" src="${document.documentElement.dataset.theme === 'dark' ? 'img/logo-dark.svg' : 'img/logo-light.svg'}" alt="" style="width:22px;height:22px"><b>Silicon<span style="color:var(--accent)">MetaTrader5</span></b></div>
      <button class="btn sm ib" id="p-open" title="${t('SiliconMetaTrader5\'i göster')}" aria-label="${t('SiliconMetaTrader5\'i göster')}">${icon('ext')}</button></div>
    ${target ? html`<div class="row between small"><span class="muted">${t('Hedef')}</span><b>${target}</b></div>` : ''}
    ${problems() ? html`<a class="callout bad small" id="p-srv" href="#">${icon('alert')}<div>${t('{n} sunucuda sorun', { n: problems() })}</div></a>` : ''}
    <div class="pstate"><div class="row"><span class="live ${tone}"></span><b>${job ? job.title : ph.label}</b></div>
      <div class="muted small">${s?.provider || '—'}${ph.group === 'on' && since ? ' · ' + duration(Date.now() / 1000 - since) : ''}</div></div>
    ${a ? html`<div class="pgrid"><div><span class="muted">${t('Sunucu')}</span><b>${a.server}</b></div>
      <div><span class="muted">${t('Bakiye')}</span><b class="num">${money(a.balance, a.currency)}</b></div>
      <div><span class="muted">${t('Varlık')}</span><b class="num">${money(a.equity, a.currency)}</b></div>
      <div><span class="muted">${t('Köprü')}</span><b class="num">127.0.0.1:${s?.bridge_port || 8001}</b></div></div>` : ''}
    <div class="col" style="gap:8px">
      ${job ? html`<button class="btn" disabled>${icon('spinner', 'spin')}${t('Çalışıyor…')}</button>`
        : active ? html`<button class="btn" data-act="stop">${icon('stop')}${t('Runtime\'ı durdur')}</button>`
        : ph.group === 'setup' ? html`<button class="btn p" id="p-setup">${icon('wand')}${t('Kurulumu tamamla')}</button>`
        : html`<button class="btn p" data-act="start">${icon('play')}${t('Runtime\'ı başlat')}</button>`}
      ${active ? html`<button class="btn" id="p-mt5">${icon(nativeWindow(s) ? 'window' : 'screen')}${nativeWindow(s) ? t('MT5 penceresini öne getir') : t('MT5 Ekranını aç')}</button>` : ''}
    </div>
    <div class="psep"></div>
    <div class="col" style="gap:6px">
      ${active ? html`<button class="btn ghost" data-act="quit_keep">${t('Uygulamadan çık (MT5 çalışmaya devam eder)')}</button>
      <button class="btn ghost danger" data-act="quit_full">${t('Tamamen kapat (MT5\'i de durdur)')}</button>`
        : html`<button class="btn ghost" data-act="quit_keep">${t('Uygulamadan çık')}</button>`}
    </div></div>`);
  $('#p-open').onclick = () => call('show_main_window');
  $('#p-srv')?.addEventListener('click', e => { e.preventDefault(); call('show_main_window'); call('tray_action', { action: 'servers' }); });
  $('#p-setup')?.addEventListener('click', () => call('show_main_window'));
  $('#p-mt5')?.addEventListener('click', async () => {
    try {
      if (nativeWindow(s)) await call('focus_mt5');
      else { await call('show_main_window'); await call('tray_action', { action: 'screen' }); }
    } catch (e) { toast(String(e), true); }
  });
  $$('[data-act]').forEach(b => b.onclick = () => call('tray_action', { action: b.dataset.act }));
}

export async function startPanel() {
  document.documentElement.classList.add('is-panel');
  await refreshInfo();
  setLang(state.info?.lang);
  setHost(state.info?.os);
  const media = matchMedia('(prefers-color-scheme: dark)');
  const theme = () => {
    let p = 'system'; try { p = localStorage.getItem('theme') || 'system'; } catch { /* private */ }
    document.documentElement.dataset.theme = p === 'dark' || (p === 'system' && media.matches) ? 'dark' : 'light';
  };
  theme(); media.addEventListener('change', theme);
  // poll only while the panel is open (it hides when it loses focus)
  let srvAt = 0;
  const refresh = async () => {
    try { state.job = await call('job_state'); } catch { /* none */ }
    if ((state.info?.servers || 0) > 0 && Date.now() - srvAt > 10000) { srvAt = Date.now(); try { srv = await call('servers'); } catch { srv = null; } }
    await refreshSnapshot();
  };
  subscribe(what => { if (what === 'snap' || what === 'job') draw(); });
  await refresh();
  setInterval(() => { if (document.hasFocus()) refresh(); }, 3000);
  on('job-done', () => refresh());
  on('panel-shown', async () => { theme(); try { await refreshInfo(); } catch { /* keep the old */ } refresh(); });
}
