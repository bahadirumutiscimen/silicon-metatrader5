// Shell: sidebar, hash router, job drawer, theme.

import { call, on, pickFolder, startDrag } from './api.js';
import { $, $$, html, render, icon, toast, duration, esc, modal } from './dom.js';
import { state, subscribe, init, phaseOf, startJob, waitJob, refreshInfo, refreshSnapshot, nativeWindow } from './store.js';
import overview from './pages/overview.js';
import screen from './pages/screen.js';
import logs from './pages/logs.js';
import data from './pages/data.js';
import datatest from './pages/datatest.js';
import settings from './pages/settings.js';
import setup from './pages/setup.js';
import reset from './pages/reset.js';
import learn from './pages/learn.js';
import strategy from './pages/strategy.js';
import connect from './pages/connect.js';
import accounts from './pages/accounts.js';
import servers from './pages/servers.js';
import appsettings from './pages/appsettings.js';
import { latestRelease, updateCard, bindUpdateCard, copyCard, bindCopyCard } from './update.js';
import { t, th, setLang, getLang, setHost } from './i18n.js';

const PAGES = { overview, screen, logs, data, datatest, settings, setup, reset, learn, strategy, connect, accounts, servers, app: appsettings };
// labels go through t() when rendered
const NAV = [
  ['İZLEME', [['overview', 'Genel Bakış', 'grid'], ['screen', 'MT5 Ekranı', 'screen'], ['logs', 'Loglar', 'logs'], ['datatest', 'Veri Testi', 'pulse']]],
  ['YÖNETİM', [['servers', 'Sunucular', 'server'], ['accounts', 'Hesaplar', 'wallet'], ['settings', 'Runtime Ayarları', 'sliders'], ['data', 'Veri Yönetimi', 'db']]],
  ['UYGULAMA', [['app', 'Uygulama Ayarları', 'sliders'], ['setup', 'Kurulum Sihirbazı', 'wand'], ['reset', 'Sıfırlama ve Kaldırma', 'trash']]],
  ['EĞİTİM', [['connect', 'Bot bağlantısı', 'plug'], ['learn', 'Python API', 'book'], ['strategy', 'Supertrend örneği', 'trend']]],
];

let current = null; // { name, page, el }
// First run: until the wizard is finished only the wizard is shown.
let gated = false;

// ---------- theme ----------
const media = matchMedia('(prefers-color-scheme: dark)');
function themePref() { try { return localStorage.getItem('theme') || 'system'; } catch { return 'system'; } }
function applyTheme() {
  const p = themePref();
  const dark = p === 'dark' || (p === 'system' && media.matches);
  document.documentElement.dataset.theme = dark ? 'dark' : 'light';
  $$('.logo-img').forEach(img => { img.src = dark ? 'img/logo-dark.svg' : 'img/logo-light.svg'; });
  $$('#theme button').forEach(b => b.classList.toggle('sel', b.dataset.v === p));
}
media.addEventListener('change', applyTheme);

// ---------- shell ----------
function shell() {
  render(document.body, html`
    <div class="titlebar" data-tauri-drag-region></div>
    <div class="shell">
      <nav class="side" aria-label="${t('Ana menü')}">
        <div class="side-drag" data-tauri-drag-region></div>
        <div class="brand" data-tauri-drag-region>
          <img class="logo-img" src="img/logo-light.svg" alt="">
          <div><b>Silicon<span>MetaTrader5</span></b><div class="muted" style="font-size:12px" id="appmode">${modeLabel()}</div></div>
        </div>
        <div id="targetbox"></div>
        ${gated ? html`<div class="sec">${t('İLK KURULUM')}</div><a class="nav" data-nav="setup" href="#/setup">${icon('wand')}${t('Kurulum Sihirbazı')}</a>
          ${state.snap && !nativeWindow() ? html`<a class="nav" data-nav="screen" href="#/screen">${icon('screen')}${t('MT5 Ekranı')}</a>` : ''}
          <a class="nav" data-nav="logs" href="#/logs">${icon('logs')}${t('Loglar')}</a>
          <p class="muted" style="font-size:12px;line-height:1.5;margin:8px 10px 0">${t('Sistem hazır olunca diğer sayfalar açılır.')}</p>`
        : NAV.map(([sec, items]) => html`<div class="sec">${t(sec)}</div>${items.map(([id, label, ic]) =>
          html`<a class="nav" data-nav="${id}" href="#/${id}">${icon(ic)}${t(label)}${id === 'logs' ? html`<span class="count hidden" id="warncount"></span>` : ''}</a>`)}`)}
        <div style="flex:1;min-height:20px"></div>
        <div id="updbox"></div>
        ${gated ? '' : html`<div class="runbox" id="runbox"></div>`}
        <div class="seg full" id="theme" role="group" aria-label="${t('Tema')}" style="margin-top:8px">
          <button data-v="light">${t('Açık')}</button><button data-v="dark">${t('Koyu')}</button><button data-v="system">${t('Sistem')}</button>
        </div>
        <div class="seg full" id="lang" role="group" aria-label="Dil / Language" style="margin-top:6px">
          <button data-l="tr" class="${getLang() === 'tr' ? 'sel' : ''}">Türkçe</button><button data-l="en" class="${getLang() === 'en' ? 'sel' : ''}">English</button>
        </div>
      </nav>
      <main id="main"></main>
    </div>
    <div id="jobbox"></div>`);
  if (navigator.userAgent.includes('Mac')) document.documentElement.classList.add('mac');
  $('#theme').addEventListener('click', e => {
    const b = e.target.closest('button'); if (!b) return;
    try { localStorage.setItem('theme', b.dataset.v); } catch { /* private mode */ }
    applyTheme();
  });
  $('#lang').addEventListener('click', async e => {
    const b = e.target.closest('button'); if (!b || b.dataset.l === getLang()) return;
    try { await call('set_lang', { lang: b.dataset.l }); location.reload(); } catch (err) { toast(String(err), true); }
  });
  $('.titlebar').addEventListener('mousedown', e => { if (e.buttons === 1) startDrag(); });
  targetbox();
  applyTheme();
}

const modeLabel = () => `v${state.info?.version || ''} · ${hasServers() ? t('Sunucu modu') : t('Lokal')}`;

/** The sidebar's target switcher: redrawn whenever app_info changes (a server
 *  added, removed or made active), not only when the app starts. */
function targetbox() {
  const box = $('#targetbox'); if (!box) return;
  const m = $('#appmode'); if (m) m.textContent = modeLabel();
  const active = state.info?.active_target || 'local';
  render(box, hasServers() ? html`<label class="target"><span class="muted" style="font-size:11px">${t('Hedef')}</span>
    <select id="target" class="f">${[['local', t('Bu bilgisayar')], ...(state.info?.targets || [])].map(([id, name]) => html`<option value="${id}" ${id === active ? 'selected' : ''}>${name}</option>`)}</select></label>` : '');
  $('#target', box)?.addEventListener('change', async e => {
    try { await call('set_active_target', { id: e.target.value }); location.reload(); } catch (err) { toast(String(err), true); }
  });
}

/** Server mode is on once a server exists (menu item, target switcher). */
const hasServers = () => (state.info?.servers || 0) > 0;

function runbox() {
  const el = $('#runbox'); if (!el) return;
  const s = state.snap, ph = phaseOf();
  const job = state.job?.running ? state.job : null;
  const tone = { up: '', warn: 'mid', down: 'bad', acc: 'mid', neu: 'off' }[ph.tone] ?? 'off';
  const since = s?.status?.supervisor?.updated;
  const disp = s ? (nativeWindow(s) ? (state.info?.os === 'macos' ? t('MT5 macOS penceresinde') : t('MT5 masaüstü penceresinde')) : 'noVNC') : '';
  const srv = !!s?.target && s.target !== 'local';
  const note = {
    stopped: t('Veriler ve broker girişi korunuyor'),
    engine_down: s?.engine_label === 'Docker' ? t('Docker Engine çalışmıyor') : t('{engine} kapalı; Başlat önce onu açar', { engine: s?.engine_label }),
  }[ph.key];
  let action = '';
  const acct = s?.status?.bridge?.probe?.account;
  // a server is brought up to date with Güncelle (new runtime upload), not a local restart or image build
  const stale = !srv && !!s?.status && (s.status.api?.level ?? 0) < (state.info?.api_level ?? 0);
  const srvOld = srv && !!s?.update_needed && !['not_installed', 'connecting', 'unreachable'].includes(ph.key);
  if (job) action = html`<button class="btn sm" disabled>${icon('spinner', 'spin')}${t('Çalışıyor…')}</button>`;
  else if (srv && ['connecting', 'unreachable'].includes(ph.key)) action = html`<a class="btn sm" href="#/servers">${icon('server')}${t('Sunucular')}</a>`;
  else if (ph.key === 'no_session') action = html`<a class="btn sm p" href="#/screen">${icon('screen')}${t("MT5 Ekranı'ndan giriş yap")}</a>`;
  else if (ph.group === 'setup') action = html`<a class="btn sm" href="#/setup">${t('Kurulumu tamamla')}</a>`;
  else if (ph.group === 'off') action = html`<button class="btn sm p" data-run="start">${icon('play')}${t('Başlat')}</button>
      ${ph.key === 'stopped' && s?.manages_vm ? html`<button class="btn sm" data-run="vm_stop" title="${t("Colima VM'i kapatır, bellek ve CPU'yu serbest bırakır")}">${t("VM'i kapat")}</button>` : ''}`;
  else action = html`<button class="btn sm" data-run="stop">${icon('stop')}${t('Durdur')}</button>`;
  render(el, html`
    <div class="row"><span class="live ${tone}"></span><b style="font-size:13px">${job ? job.title : t('Runtime · {phase}', { phase: ph.label })}</b></div>
    <div class="muted small">${s ? s.provider : '—'}<br>${disp}${ph.group === 'on' && since ? ' · ' + duration(Date.now() / 1000 - since) : ''}${note ? html`<br>${note}` : ''}</div>
    ${acct ? html`<a class="muted small acctline" href="#/accounts" title="${t('Hesaplar')}">${icon('wallet')}${acct.server} · ${acct.login_masked || ''}</a>` : ''}
    ${stale ? html`<div class="callout warn small" style="padding:8px 10px">${s.native ? t('Runtime bu sürümün eski betikleriyle çalışıyor; yeni özellikler için yeniden başlat.') : t('Runtime imajı bu sürümden eski; yeni özellikler için imajı yeniden derle ya da çek.')}</div>` : ''}
    ${srvOld ? html`<div class="callout warn small" style="padding:8px 10px">${t('Sunucudaki runtime bu uygulamadan eski; Sunucular sayfasındaki Güncelle yeni sürümü yükler.')}</div>` : ''}
    <div class="col" style="gap:6px">${srvOld && !job ? html`<a class="btn sm p" href="#/servers">${icon('refresh')}${t('Güncelle')}</a>` : ''}${stale && s.native && !job ? html`<button class="btn sm p" id="rb-restart">${icon('refresh')}${t('Runtime\'ı yeniden başlat')}</button>` : ''}${action}</div>`);
  $('#rb-restart', el)?.addEventListener('click', restartRuntime);
  $$('[data-run]', el).forEach(b => b.addEventListener('click', async () => {
    try { await startJob(b.dataset.run); } catch (e) { toast(String(e), true); }
  }));
}

/** Stop + start: the native runtime copies the app's current scripts on start. */
export async function restartRuntime() {
  try {
    if (state.snap?.container?.status === 'running') await waitJob(await startJob('stop'));
    await waitJob(await startJob('start'));
  } catch (e) { toast(String(e), true); }
}

// ---------- job drawer ----------
let jobMin = false;
function jobbox(what) {
  const el = $('#jobbox'); if (!el) return;
  const j = state.job;
  const show = j && j.id && (j.running || (j.finished && Date.now() / 1000 - j.finished < 120 && !j.dismissed));
  if (!show) { el.innerHTML = ''; return; }
  if (what === 'job-line' && $('#jobterm')) {
    const term = $('#jobterm');
    const stick = term.scrollTop + term.clientHeight >= term.scrollHeight - 30;
    term.textContent = j.lines.slice(-600).join('\n');
    if (stick) term.scrollTop = term.scrollHeight;
    return;
  }
  const status = j.running ? html`<span class="dot run">${icon('spinner', 'spin')}</span>` : j.code === 0 ? html`<span class="dot ok">${icon('check')}</span>` : html`<span class="dot bad">${icon('x')}</span>`;
  render(el, html`<div class="job ${jobMin ? 'min' : ''}">
    <header>${status}<b class="grow" style="font-size:13px">${j.title}</b>
      <span class="muted" style="font-size:12px">${j.running ? duration(Date.now() / 1000 - j.started) : j.code === 0 ? t('tamamlandı') : t('çıkış {code}', { code: j.code })}</span>
      ${j.running ? html`<button class="btn sm" id="job-cancel">${t('Durdur')}</button>` : html`<button class="btn sm ib" id="job-close" aria-label="${t('Kapat')}">${icon('x')}</button>`}
      <button class="btn sm ib" id="job-min" aria-label="${t('Küçült')}">${icon('min')}</button></header>
    <pre class="term" id="jobterm"></pre></div>`);
  const term = $('#jobterm'); term.textContent = j.lines.slice(-600).join('\n'); term.scrollTop = term.scrollHeight;
  $('#job-min').onclick = () => { jobMin = !jobMin; jobbox(); };
  $('#job-close')?.addEventListener('click', () => { j.dismissed = true; jobbox(); });
  $('#job-cancel')?.addEventListener('click', async () => { await call('cancel_job'); toast(t('İşlem durduruluyor…')); });
}
setInterval(() => { if (state.job?.running) jobbox(); }, 1000);

// ---------- update notice ----------
async function checkUpdate() {
  const el = $('#updbox'); if (!el || gated) return;
  const tag = await latestRelease();
  let copy = null;
  if (state.info?.os === 'macos' && state.info?.packaged) { try { copy = await call('app_copy_status'); } catch { /* not Homebrew */ } }
  render(el, html`${copyCard(copy)}${updateCard(state.info, tag)}`);
  bindUpdateCard(el, tag);
  bindCopyCard(el, msg => toast(msg, true));
}

// ---------- router ----------
function route() {
  let name = (location.hash.replace(/^#\/?/, '').split('?')[0]) || 'overview';
  if (gated && !['setup', 'screen', 'logs'].includes(name)) { name = 'setup'; if (location.hash !== '#/setup') history.replaceState(null, '', '#/setup'); }
  const page = PAGES[name] || overview;
  if (current?.name === name) return;
  current?.page.unmount?.();
  const el = $('#main');
  el.scrollTop = 0;
  el.innerHTML = '<div class="page"></div>';
  current = { name, page, el: el.firstChild };
  $$('.nav').forEach(a => a.classList.toggle('on', a.dataset.nav === name));
  try { page.mount(current.el); page.update?.('snap'); }
  catch (e) { pageError(current.el, e); }
}

/** A page that throws shows its error in place; the menu and other pages keep working. */
function pageError(el, e) {
  console.error(e);
  render(el, html`<div class="callout bad">${icon('alert')}<div><b>${t('Bu sayfa açılamadı.')}</b> <span class="mono">${String(e)}</span><br>
    <a href="#/servers">${t('Sunucular')}</a> · <a href="#/logs">${t('Loglar')}</a></div></div>`);
}
export function go(name) { location.hash = '#/' + name; }

// ---------- runtime folder missing ----------
function needRuntime() {
  render($('#main'), html`<div class="page"><div class="card empty" style="margin-top:8vh">
    <img class="logo-img" src="img/logo-light.svg" alt="">
    <h1>${t('SiliconMetaTrader5 klasörünü seç')}</h1>
    <p>${th('Uygulama runtime\'ı (compose.yaml ve .env) repo içindeki <span class="mono">runtime/</span> klasöründen yönetir. Klasörü bir kez seçmen yeterli; sonraki açılışlarda hatırlanır.')}</p>
    <p class="mono" style="font-size:12px">${state.info?.runtime_error || ''}</p>
    <button class="btn p" id="pick">${icon('folder')}${t('Klasör seç…')}</button></div></div>`);
  applyTheme();
  $('#pick').onclick = async () => {
    const p = await pickFolder();
    if (!p) return;
    try { await call('set_runtime_dir', { path: p }); await refreshInfo(); start(); }
    catch (e) { toast(String(e), true); }
  };
}

let started = false;
function start() {
  if (!state.info?.runtime_dir) return needRuntime();
  if (started) return;
  started = true;
  window.addEventListener('hashchange', route);
  route();
  refreshSnapshot();
}

// MT5 decides the active account; remember each one the app sees.
let lastAcct = '';
function syncAccount() {
  const a = state.snap?.status?.bridge?.probe?.account;
  const key = a ? `${a.server}|${a.login_masked}` : '';
  if (!key || key === lastAcct) return;
  lastAcct = key;
  call('accounts').catch(() => { lastAcct = ''; });
}

let lastJobSeen = null;
subscribe(what => {
  if (what === 'info') targetbox();
  // a finished job may have removed a server (server_uninstall) or changed the active target
  if (what === 'job' && state.job && !state.job.running && state.job !== lastJobSeen) { lastJobSeen = state.job; refreshInfo().catch(() => {}); }
  if (what === 'snap' && !PANEL) syncAccount();
  if (what === 'snap' || what === 'job') runbox();
  if (what === 'job' || what === 'job-line') jobbox(what);
  if (current) { try { current.page.update?.(what); } catch (e) { pageError(current.el, e); } }
  const wc = $('#warncount');
  if (wc) { wc.textContent = state.logWarnings; wc.classList.toggle('hidden', !state.logWarnings); }
});

const PANEL = new URLSearchParams(location.search).get('panel') === '1';
// the separate MetaTrader 5 window (open_display_window)
const VIEWER = new URLSearchParams(location.search).get('viewer') === '1';

// ---------- closing the app ----------
let stopping = false;

/** Stop MT5 + bridge (and the Güvenli VM), then quit. */
async function stopEverything() {
  if (stopping) return;
  if (state.job?.running) { toast(t('Bir işlem sürüyor ({title}); bitmesini bekle ya da iş penceresinden durdur.', { title: state.job.title }), true); return; }
  stopping = true;
  try {
    // only this computer: servers keep running when the app quits
    const local = () => call('snapshot', { target: 'local' });
    const s = await local();
    if (s?.container?.status === 'running') {
      const code = await waitJob(await startJob('stop', { target: 'local' }));
      if (code !== 0) throw new Error(t('Runtime durdurulamadı (çıkış {code}); ayrıntı iş penceresinde.', { code }));
    }
    const s2 = await local();
    if (s2?.manages_vm && s2.lifecycle !== 'engine_down') {
      const code = await waitJob(await startJob('vm_stop'));
      if (code !== 0) throw new Error(t('Sanal makine kapatılamadı (çıkış {code}).', { code }));
    }
    await call('quit_app');
  } catch (e) { toast(String(e.message || e), true); }
  stopping = false;
}

/** kind "close": the window's close button; "quit": Cmd+Q / Dock › Quit. */
async function askClose(kind) {
  const tray = !!state.info?.tray, mac = navigator.userAgent.includes('Mac');
  // "Tamamen kapat" stops only this computer's runtime (stopEverything), so describe that one
  let localSnap = state.snap;
  if ((state.snap?.target || 'local') !== 'local') { try { localSnap = await call('snapshot', { target: 'local' }); } catch { localSnap = null; } }
  const srvNote = hasServers() ? ' ' + t('Sunuculardaki MT5 çalışmaya devam eder.') : '';
  const keepLabel = kind === 'close' && tray ? (mac ? t('Menü çubuğuna küçült') : t('Sistem tepsisine küçült')) : t('Uygulamadan çık, MT5 çalışsın');
  const keepText = kind === 'close' && tray
    ? (mac ? t('Pencere kapanır; MT5 ve Python köprüsü çalışmaya devam eder. Menü çubuğundaki simgeye tıklayınca durum paneli açılır, sağ tıkla menüden yönetirsin.')
      : t('Pencere kapanır; MT5 ve Python köprüsü çalışmaya devam eder. Tepsideki simgeye tıklayınca durum paneli açılır, sağ tıkla menüden yönetirsin.'))
    : t('Uygulama kapanır; MT5 ve Python köprüsü arka planda çalışmaya devam eder. Durdurmak için uygulamayı yeniden aç ya da terminalde smt5 down.');
  let remember = null;
  const choice = await modal({
    title: t('SiliconMetaTrader5 kapatılıyor'),
    body: html`<div class="closeopts">
      <div><b>${keepLabel}</b><div class="muted">${keepText}</div></div>
      <div><b>${t('Tamamen kapat')}</b><div class="muted">${localSnap?.manages_vm
        ? t('MT5 düzgünce kapatılır, köprü ve sanal makine durur, sonra uygulama çıkar. Python kodun bağlanamaz.')
        : t('MT5 düzgünce kapatılır, köprü durur, sonra uygulama çıkar. Python kodun bağlanamaz.')}${srvNote}</div></div></div>
      ${kind === 'close' ? html`<label class="row" style="font-size:13px;gap:8px"><input type="checkbox" id="cl-remember">${t('Bu seçimi hatırla (Uygulama Ayarları\'ndan değiştirebilirsin)')}</label>` : ''}`,
    buttons: [{ label: t('Vazgeç'), value: null }, { label: t('Tamamen kapat'), value: 'full' }, { label: keepLabel, value: 'keep', primary: true }],
    onMount: bg => { remember = bg.querySelector('#cl-remember'); },
  });
  if (!choice) return;
  const keepIsTray = kind === 'close' && tray;
  if (remember?.checked) {
    try { await call('set_close_action', { action: choice === 'full' ? 'quit' : keepIsTray ? 'tray' : null }); await refreshInfo(); } catch (e) { toast(String(e), true); }
  }
  if (choice === 'full') return stopEverything();
  if (keepIsTray) return call('hide_main');
  return call('quit_app');
}

if (!PANEL) {
  on('navigate', name => go(name));
  on('toast', msg => toast(msg));
  on('close-requested', () => askClose('close'));
  on('quit-requested', () => askClose('quit'));
  on('tray-action', async action => {
    try {
      if (action === 'start' || action === 'stop') await startJob(action);
      else if (action === 'screen') go('screen');
      else if (action === 'servers') go('servers');
      else if (action === 'quit_full') await stopEverything();
      else if (action === 'quit_keep') await call('quit_app');
    } catch (e) { toast(String(e), true); }
  });
}

/** Setups that already work (e.g. made before the gate existed) skip the wizard. */
async function autoComplete() {
  const s = state.snap;
  if (!gated || !(s?.lifecycle === 'ready' && s?.env_exists)) return;
  try {
    await call('set_setup_complete', { done: true });
    if (!state.info.lang_chosen) await call('set_lang', { lang: state.info.lang });
    await refreshInfo();
    gated = false;
  } catch { /* stay in the wizard */ }
}

(async () => {
  if (PANEL) { const { startPanel } = await import('./panel.js'); return startPanel(); }
  if (VIEWER) { const { startViewerWindow } = await import('./viewer.js'); return startViewerWindow(); }
  await refreshInfo();
  setLang(state.info?.lang);
  setHost(state.info?.os);
  gated = !!state.info?.runtime_dir && !state.info?.setup_complete;
  await init();
  await autoComplete();
  shell();
  runbox();
  jobbox();
  start();
  checkUpdate();
  setInterval(checkUpdate, 6 * 3600 * 1000);
})().catch(e => { document.body.textContent = t('SiliconMetaTrader5 başlatılamadı: {error}', { error: String(e) }); });
