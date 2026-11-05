import { call, on } from '../api.js';
import { $, html, render, icon, toast, copyText } from '../dom.js';
import { state, phaseOf, startJob, nativeWindow, targetName } from '../store.js';
import { t, th, locale } from '../i18n.js';
import { mountViewer } from '../viewer.js';

let el, loadedUrl = null, viewer = null;

/** The screen in `node` (noVNC inside the app; ⌘ → Ctrl when this computer is a Mac). */
function showScreen(node, url) {
  viewer?.destroy();
  viewer = node ? mountViewer(node, url, { mac: state.info?.os === 'macos', title: t('MetaTrader 5 masaüstü') }) : null;
}

function frame() {
  const s = state.snap;
  const box = $('#scr', el);
  if (!box || !s) return;
  const running = s.container?.status === 'running';
  const displayUp = running && s.status; // status API answers once the display stack runs
  // Windows server: the screen (TightVNC service + the Display task) runs without a
  // session and without the supervisor, e.g. the logon screen while lifecycle is no_session
  const winScreen = s.provider_kind === 'windows' && s.link?.state === 'connected' && !!s.display_url && phaseOf().group !== 'setup';
  if (winScreen && !displayUp) {
    const ph = phaseOf(), noSession = s.lifecycle === 'no_session';
    const key = `${s.display_url}|${noSession ? 'logon' : ph.key}|${!!state.job?.running}`;
    if (loadedUrl === key) return;
    loadedUrl = key;
    render(box, html`<div class="col" style="height:100%;gap:0">
      <div class="callout ${noSession ? 'info' : 'warn'}" id="scr-winnote" style="margin:0;border-radius:0">${icon(noSession ? 'info' : 'alert')}<div class="row wrap" style="gap:8px;align-items:center">${noSession
        ? t('Windows giriş ekranı: oturum aç, MT5 kendiliğinden başlar.')
        : html`${ph.label} · ${t('MT5 kurulumu ve broker girişi korunuyor.')}${ph.group === 'off' ? html` <button class="btn p sm" id="scr-start" ${state.job?.running ? 'disabled' : ''}>${icon('play')}${t('Başlat')}</button>` : ''}`}</div></div>
      <div class="vncbox" id="scr-vnc" style="flex:1;min-height:0"></div></div>`);
    showScreen($('#scr-vnc', el), s.display_url);
    $('#scr-start', el)?.addEventListener('click', () => startJob('start').catch(e => toast(String(e), true)));
    return;
  }
  if (!running || !displayUp) {
    loadedUrl = null;
    showScreen(null);
    const ph = phaseOf();
    render(box, html`<div class="over"><div><b>${running ? t('Görüntü sunucusu açılıyor…') : ph.group === 'setup' ? t('Kurulum tamamlanmadı') : t('Runtime durduruldu')}</b><br>
      <span style="opacity:.7;font-size:13px">${running ? ph.label : ph.group === 'setup' ? t('Kurulum sihirbazından devam et.') : ph.key === 'engine_down' && state.snap?.manages_vm ? t('x86_64 VM kapalı; Başlat önce VM\'i, sonra MT5\'i açar.') : t('MT5 kurulumu ve broker girişi korunuyor.')}</span><br><br>
      ${running ? '' : ph.group === 'setup' ? html`<a class="btn p" href="#/setup">${t('Kurulum sihirbazı')}</a>` : html`<button class="btn p" id="scr-start" ${state.job?.running ? 'disabled' : ''}>${icon('play')}${t('Başlat')}</button>`}</div></div>`);
    $('#scr-start', el)?.addEventListener('click', () => startJob('start').catch(e => toast(String(e), true)));
    return;
  }
  if (loadedUrl === s.display_url) return;
  loadedUrl = s.display_url;
  render(box, html`<div class="vncbox" id="scr-vnc"></div>`);
  showScreen($('#scr-vnc', el), s.display_url);
}

async function openWindow() {
  try { await call('open_display_window'); } catch (e) { toast(String(e), true); }
}

// Mac yerel mode: MT5 is an ordinary macOS window (Wine's Mac driver), so
// there is nothing to embed; copy-paste works like in any Mac app.
function drawNative() {
  const s = state.snap, ph = phaseOf();
  const running = s.container?.status === 'running';
  const busy = !!state.job?.running;
  let body;
  if (ph.group === 'setup') body = html`<b>${t('Kurulum tamamlanmadı')}</b><br><span class="muted">${t("Kurulum sihirbazı Wine 11'i indirir, MT5'i kurar (ya da Güvenli moddan kopyalar).")}</span><br><br><a class="btn p" href="#/setup">${t('Kurulum sihirbazı')}</a>`;
  else if (!running) body = html`<b>${t('Runtime durduruldu')}</b><br><span class="muted">${t('MT5 kurulumu ve broker girişi korunuyor.')}</span><br><br><button class="btn p" id="scr-start" ${busy ? 'disabled' : ''}>${icon('play')}${t('Başlat')}</button>`;
  else body = html`<b>${t('MT5 kendi macOS penceresinde çalışıyor')}</b><br><span class="muted">${ph.label} · ${t("Dock'ta MetaTrader 5 simgesi görünür. MT5 penceresini kapatırsan runtime tamamen durur; yeniden açmak için Başlat'ı kullan.")}</span><br><br>
      <button class="btn p" id="scr-focus">${icon('window')}${t('Pencereyi öne getir')}</button>`;
  render(el, html`
    <div class="head">
      <div><div class="crumbs">SiliconMetaTrader5 › ${t('Mac yerel')} › <b>${t('MT5 Ekranı')}</b></div><h1>${t('MT5 Ekranı')}</h1></div>
      <div class="row wrap"><span class="chip lg ${ph.tone}" id="scr-phase"><span class="d"></span>${ph.label}</span></div>
    </div>
    <div class="split">
      <div class="main"><section class="card pad" style="text-align:center;padding:48px 24px;line-height:1.6">${body}</section></div>
      <aside class="aside col" style="flex:0 1 300px">
        <section class="card pad col">
          <h2>${t('Mac yerel mod')}</h2>
          <div>
            <div class="kv"><span class="muted">${t('Görüntü')}</span><span>${t('macOS penceresi')}</span></div>
            <div class="kv"><span class="muted">${t('Köprü')}</span><span class="mono">127.0.0.1:${s.bridge_port}</span></div>
            <div class="kv"><span class="muted">${t('Pano')}</span><span>${t('Mac panosuyla ortak')}</span></div>
          </div>
          <p class="muted" style="margin:0;font-size:12px;line-height:1.5">${t('Dosyalar:')} <span class="mono">${s.native_home || ''}</span></p>
        </section>
        <section class="card pad col">
          <h2>${t('İlk giriş')}</h2>
          <p class="muted" style="margin:0;font-size:13px;line-height:1.55">${th("MT5 açıldığında <b>File › Open an Account</b> ile broker'ını bul ya da <b>File › Login to Trade Account</b> ile mevcut hesabına gir.")} ${t('MT5 Güvenli moddan kopyalandıysa giriş zaten hazırdır.')}</p>
          <a class="btn sm" href="#/datatest">${icon('pulse')}${t('Veri testine git')}</a>
        </section>
      </aside>
    </div>`);
  $('#scr-start', el)?.addEventListener('click', () => startJob('start').catch(e => toast(String(e), true)));
  $('#scr-focus', el)?.addEventListener('click', () => call('focus_mt5').catch(e => toast(String(e), true)));
}

function draw() {
  const s = state.snap;
  if (!s) return;
  if (nativeWindow(s)) return drawNative();
  const plain = `http://127.0.0.1:${s.display_port}/`;
  render(el, html`
    <div class="head">
      <div><div class="crumbs">SiliconMetaTrader5 › ${targetName() || t('Lokal runtime')} › <b>${t('MT5 Ekranı')}</b></div><h1>${t('MT5 Ekranı')}</h1></div>
      <div class="row wrap">
        <span class="chip lg ${phaseOf().tone}" id="scr-phase"><span class="d"></span>${phaseOf().label}</span>
        <button class="btn" id="scr-reload" ${s.status ? '' : 'disabled'}>${icon('refresh')}${t('Yeniden bağlan')}</button>
        <button class="btn" id="scr-full" ${s.status ? '' : 'disabled'}>${icon('full')}${t('Tam ekran')}</button>
        <button class="btn" id="scr-win" ${s.status ? '' : 'disabled'}>${icon('window')}${t('Ayrı pencere')}</button>
        <button class="btn" id="scr-browser" ${s.status ? '' : 'disabled'}>${icon('ext')}${t('Tarayıcıda aç')}</button>
      </div>
    </div>
    <div class="split">
      <div class="main"><div class="screen" id="scr"></div></div>
      <aside class="aside col" style="flex:0 1 300px">
        <section class="card pad col">
          <h2>${t('Bağlantı')}</h2>
          <div>
            <div class="kv"><span class="muted">${t('Altyapı')}</span><span>noVNC</span></div>
            <div class="kv"><span class="muted">${t('Adres')}</span><span class="mono">127.0.0.1:${s.display_port}</span></div>
            <div class="kv"><span class="muted">${t('Kimlik doğrulama')}</span><span>${s.has_password ? t('Şifreli') : html`<span class="neg">${t('Şifresiz')}</span>`}</span></div>
            <div class="kv"><span class="muted">${t('Erişim')}</span><span>${t('yalnızca bu bilgisayar')}</span></div>
          </div>
          ${s.has_password ? html`<button class="btn sm" id="scr-pw">${icon('key')}${t('Şifreyi kopyala')}</button>` : (s.target || 'local') === 'local' ? html`<a class="btn sm" href="#/settings">${t('Şifre oluştur')}</a>` : ''}
        </section>
        ${html`<section class="card pad col">
          <div class="row between"><h2>${t('Pano eşitleme')}</h2><label class="switch"><input type="checkbox" id="scr-clip" ${clipOn ? 'checked' : ''}><span></span></label></div>
          <p class="muted" style="margin:0;font-size:13px;line-height:1.55">${isMac ? th("Mac'te kopyaladığın metni MT5'te <b>Ctrl+V</b> ile yapıştırırsın; MT5'te <b>Ctrl+C</b> ile kopyaladığını Mac'te <b>⌘V</b> ile.") : th("Bilgisayarında kopyaladığın metni MT5'te <b>Ctrl+V</b> ile yapıştırırsın; MT5'te <b>Ctrl+C</b> ile kopyaladığını bilgisayarında <b>Ctrl+V</b> ile.")} ${t('Yalnızca bu ekran açıkken çalışır.')}</p>
          <div class="muted" style="font-size:12px" id="scr-clip-st">${s.lifecycle === 'no_session' ? t('Windows oturumu açık değil; oturum açılınca kendiliğinden başlar.') : clipStatus ?? t('Bekliyor')}</div>
        </section>`}
        <section class="card pad col">
          <h2>${t('İlk giriş')}</h2>
          <p class="muted" style="margin:0;font-size:13px;line-height:1.55">${th("MT5 açıldığında <b>File › Open an Account</b> ile broker'ını bul ya da <b>File › Login to Trade Account</b> ile mevcut hesabına gir.")} ${t('Geçmiş barlar arka planda iner; ilk 5–10 dakikada "No data" görmek normaldir.')}</p>
          <a class="btn sm" href="#/datatest">${icon('pulse')}${t('Veri testine git')}</a>
        </section>
      </aside>
    </div>`);
  loadedUrl = null;
  frame();
  $('#scr-reload', el).onclick = () => { loadedUrl = null; frame(); };
  $('#scr-full', el).onclick = () => $('#scr', el).requestFullscreen?.().catch(() => openWindow());
  $('#scr-win', el).onclick = openWindow;
  $('#scr-browser', el).onclick = () => call('open_external', { target: plain }).catch(e => toast(String(e), true));
  $('#scr-pw', el)?.addEventListener('click', async () => { const p = await call('reveal_password'); if (p) copyText(p); });
  $('#scr-clip', el)?.addEventListener('change', e => {
    clipOn = e.target.checked;
    try { localStorage.setItem('clip-sync', clipOn ? '1' : '0'); } catch { /* ignore */ }
    if (!clipOn) setClipStatus(t('Kapalı'));
    syncClipboard();
  });
}

// ---- clipboard sync (noVNC): the backend copies between the host clipboard
// and MT5's: the container's X CLIPBOARD here, or a server's clipboard
// (Windows clipboard / xclip) through its status API.
const isMac = navigator.userAgent.includes('Mac');
let clipOn = true, clipStatus = null, clipActive = false; // null = waiting
try { clipOn = localStorage.getItem('clip-sync') !== '0'; } catch { /* ignore */ }

function syncClipboard() {
  const s = state.snap;
  // needs the status API (servers: through the tunnel) — off while MT5's runtime is down
  const want = !!el && clipOn && !nativeWindow(s) && !!s?.status;
  if (want === clipActive) return;
  clipActive = want;
  call('clipboard_sync', { enabled: want }).catch(() => { clipActive = false; });
}

function setClipStatus(text) {
  clipStatus = text;
  const n = el && $('#scr-clip-st', el);
  if (n) n.textContent = text;
}

on('clipboard', ev => {
  const time = new Date().toLocaleTimeString(locale(), { hour: '2-digit', minute: '2-digit', second: '2-digit' });
  if (ev.error) setClipStatus(t('Hata: {error}', { error: ev.error }));
  else setClipStatus(t('{dir} · {n} karakter · {time}', { dir: ev.to === 'mt5' ? (isMac ? 'Mac → MT5' : t('Bilgisayar → MT5')) : (isMac ? 'MT5 → Mac' : t('MT5 → bilgisayar')), n: ev.chars, time }));
});

let lastKey = '';
export default {
  mount(node) { el = node; lastKey = ''; draw(); syncClipboard(); },
  update(what) {
    if ((what !== 'snap' && what !== 'job') || !el) return;
    const s = state.snap;
    // Rebuild (and reload the viewer) only when the connection itself changes.
    const key = [s?.native, state.job?.running, s?.display, s?.display_port, s?.has_password, s?.container?.status, !!s?.status, s?.status ? '' : `${s?.lifecycle}:${!!state.job?.running}`].join('|');
    syncClipboard();
    if (key !== lastKey) { lastKey = key; draw(); return; }
    const chip = $('#scr-phase', el);
    if (chip) { const ph = phaseOf(); chip.className = `chip lg ${ph.tone}`; chip.lastChild.textContent = ph.label; }
  },
  unmount() { showScreen(null); el = null; loadedUrl = null; syncClipboard(); },
};
