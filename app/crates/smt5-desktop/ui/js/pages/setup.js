// Setup wizard, also the first-run screen: language → system → mode → what is
// installed / what will be installed → start → broker login → data test.
// Server path (Sunucu · SSH): connection → mode → precheck → install on the
// server, then the same login / data test steps on that server.
// Until it is finished the app shows nothing else. Nothing runs without a
// click; every command is shown in the job drawer.

import { call } from '../api.js';
import { $, $$, html, render, icon, toast, copyText, duration, raw } from '../dom.js';
import { state, phaseOf, startJob, waitJob, refreshSnapshot, refreshInfo, nativeWindow } from '../store.js';
import { datatestView } from './datatest.js';
import { modeCards } from '../modes.js';
import { t, th, getLang } from '../i18n.js';
import { SSH_GUIDE } from '../learn-data.js';

// labels go through t() when rendered
const ALL_STEPS = [
  ['lang', 'Dil'],
  ['os', 'Sistem'],
  ['mode', 'Çalışma modu'],
  ['prepare', 'Bileşenler'],
  ['start', 'Başlatma'],
  ['login', 'Broker girişi'],
  ['test', 'Veri testi'],
  ['done', 'Hazır'],
];
const SRV_STEPS = [
  ['lang', 'Dil'],
  ['os', 'Sistem'],
  ['srv_conn', 'Bağlantı'],
  ['srv_mode', 'Çalışma modu'],
  ['srv_check', 'Ön kontrol'],
  ['srv_install', 'Sunucuya kurulum'],
  ['srv_autologon', 'Otomatik oturum'],
  ['srv_reboot', 'Yeniden başlatma'],
  ['login', 'Broker girişi'],
  ['test', 'Veri testi'],
  ['done', 'Hazır'],
];
const nat = () => !!state.snap?.native;
const mac = () => state.info?.os === 'macos';
const linuxX86 = () => state.info?.os === 'linux' && state.info?.arch === 'x86_64';
const store = (k, v) => { try { localStorage.setItem(k, v); } catch { /* private mode */ } };
const load = k => { try { return localStorage.getItem(k); } catch { return null; } };
let osSeen = load('smt5.osSeen') === '1';
let osPick = load('smt5.osPick') || 'local';
const srvPath = () => osPick === 'server';
// which server the user picked on the first step: 'linux' | 'windows' (the SSH banner still decides when it can tell)
let srvOs = load('smt5.srvOs') === 'windows' ? 'windows' : 'linux';
// Windows server: autologon + one reboot after the install (Linux path has neither)
const steps = () => srvPath() ? SRV_STEPS.filter(([k]) => isWin() || !['srv_autologon', 'srv_reboot'].includes(k)) : ALL_STEPS.filter(([k]) => k !== 'mode' || mac() || linuxX86());
// "Sunucu ekle" (#/setup?add=1): a new server even while another one is active
const addMode = () => /[?&]add=1/.test(location.hash);
let addedId = null;
const activeId = () => (addMode() ? addedId : state.info?.active_target && state.info.active_target !== 'local' ? state.info.active_target : null);
// server path state
let form = { name: '', ssh: '', key: '' }; // the connection form survives redraws
let authMethod = 'password';
let fixWaiting = 0; // > 0 while the sudo command runs in Terminal (a run id)
let waiting = 0; // > 0 while we wait for the password typed in Terminal (a run id) // how the user logs in: password (typed in Terminal) or a key file
let probe = null, srvTest = null, srvMode = null, srvModeSeen = false, pc = null, checking = false, srvBusy = false, srvErr = null;
let SEQ = 0; // module-level run ids for the polling loops: never reused, so a remount cannot collide with an old loop
let autoOn = false, autoWaiting = 0, rebootDone = false, rebooting = 0, instSeen = false, keyLine = null, rebootNow = 0, autoHint = null;
// after the reboot: Windows up but no session yet (autologon still on) is waited for this long
const LOGON_GRACE_MS = 90000;
// what the server says wins (SSH banner, then the key login test, then the saved type); the first step's choice otherwise
const isWin = () => {
  if (probe?.os) return probe.os === 'windows';
  if (srvTest?.os) return srvTest.os === 'windows';
  if (activeId() && state.snap?.provider_kind) return state.snap.provider_kind === 'windows';
  return srvOs === 'windows';
};
// the banner names the other OS than the card the user picked
const osDiffers = () => !!probe?.os && probe.os !== srvOs;
const winInstalled = () => !!state.snap && !['not_installed', 'connecting', 'unreachable', 'installing', 'unknown'].includes(state.snap.lifecycle || 'unknown');
const srvInstalled = () => !!activeId() && (!!state.snap?.status || (isWin() && (winInstalled() || instSeen)));
const hasSession = () => ['waiting_login', 'ready'].includes(state.snap?.lifecycle);

let el, step = null, comps = null, scanning = false, testResult = null, dt = null, modes = [], pickLang = null;

async function scan() {
  scanning = true; draw();
  if (mac() || linuxX86()) { try { modes = await call('modes'); } catch { modes = []; } }
  try { comps = await call('components'); } catch (e) { toast(String(e), true); }
  scanning = false;
  draw();
}

function done() {
  const s = state.snap, ph = s?.status?.phase;
  const running = s?.container?.status === 'running';
  const installed = srvInstalled();
  if (installed && isWin()) instSeen = true;
  return {
    lang: !!state.info?.lang_chosen,
    os: osSeen || (!srvPath() && !!s?.env_exists),
    srv_conn: !!activeId() && (!!srvTest?.ok || s?.link?.state === 'connected'),
    srv_mode: srvModeSeen || installed,
    srv_check: installed || (!!pc && !pc.problems.some(p => p.blocking)),
    srv_install: installed,
    srv_autologon: autoOn, // only when server_autologon_status said so (an open RDP/console session does not configure it)
    srv_reboot: rebootDone || (hasSession() && autoOn), // session already open and autologon on: no restart needed
    mode: !(mac() || linuxX86()) || !!s?.env_exists,
    prepare: !!comps && comps.every(c => c.state === 'ok'),
    start: running && ['waiting_login', 'ready'].includes(ph),
    login: ph === 'ready',
    test: !!testResult && ['ok', 'market_closed'].includes(testResult.verdict),
    done: ph === 'ready' && !!testResult,
  };
}

/** A step can be opened when every step before it is done. */
function reachable(k) {
  const d = done(), list = steps();
  const i = list.findIndex(([x]) => x === k);
  return list.slice(0, i).every(([x]) => d[x]);
}
function firstOpen() {
  const d = done();
  return (steps().find(([k]) => !d[k]) || steps()[steps().length - 1])[0];
}

function dot(st) {
  return st === 'ok' ? html`<span class="dot ok">${icon('check')}</span>`
    : st === 'user' ? html`<span class="dot warn">!</span>`
    : st === 'run' ? html`<span class="dot run">${icon('spinner', 'spin')}</span>`
    : st === 'bad' ? html`<span class="dot bad">${icon('x')}</span>`
    : html`<span class="dot wait">↓</span>`;
}

// Windows: install + start OpenSSH Server, allow port 22 in Windows Firewall, then check it listens
const WIN_SSH_ON = "Add-WindowsCapability -Online -Name OpenSSH.Server~~~~0.0.1.0; Start-Service sshd; Set-Service sshd -StartupType Automatic; if (-not (Get-NetFirewallRule -Name 'OpenSSH-Server-In-TCP' -ErrorAction SilentlyContinue)) { New-NetFirewallRule -Name 'OpenSSH-Server-In-TCP' -DisplayName 'OpenSSH Server (sshd)' -Enabled True -Direction Inbound -Protocol TCP -Action Allow -LocalPort 22 }; (Test-NetConnection localhost -Port 22).TcpTestSucceeded";

// Windows: remove the pre-logon notice (it stops autologon at "OK")
const WIN_NO_BANNER = "$k = 'HKLM:\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Policies\\System'; Set-ItemProperty $k -Name legalnoticecaption -Value ''; Set-ItemProperty $k -Name legalnoticetext -Value ''";

function fixLine(cmd) {
  return html`<div class="fix"><code class="selectable">${cmd}</code><button class="btn sm ib" data-copy="${cmd}" aria-label="${t('Komutu kopyala')}">${icon('copy')}</button></div>`;
}

/** What to do about a Windows precheck problem (ids as the backend sends them). */
function winHint(id) {
  return {
    win_version: t('Windows 10 1809 (build 17763), Server 2019 ya da daha yeni bir sürüm gerekir.'),
    arch: t('Yalnız 64 bit Intel/AMD (AMD64) desteklenir; ARM sunucular desteklenmez.'),
    not_admin: t('SSH kullanıcısı yönetici olmalı. Yönetici bir kullanıcıyla sunucuyu yeniden ekle.'),
    disk: t('Sistem sürücüsünde en az 3 GB boş yer aç.'),
    mem: t('Sunucuda en az 2 GB bellek gerekir.'),
    no_tar: t("tar.exe bulunamadı (Windows 10 1803 ve sonrasında gelir). Windows'u güncelle."),
    network: t('Sunucu indirme adreslerine (python.org, tightvnc.com, mql5.com) ulaşamıyor. İnterneti ve güvenlik duvarını kontrol et.'),
    pending_reboot: t("Windows'un bekleyen bir yeniden başlatması var. Kurulumdan önce yeniden başlatman önerilir."),
    logon_banner: t('Windows oturum açmadan önce bir uyarı metni gösteriyor (genelde sağlayıcı ekler). Otomatik oturum bu ekranda takılır ve yeniden başlatınca MT5 açılmaz. Kaldırmak için sunucuda yönetici PowerShell\'de şunu çalıştır, sonra Tekrar kontrol et:'),
    foreign_mt5: t("Program Files içinde bu uygulamanın kurmadığı bir MetaTrader 5 var. Önce onu kaldır ya da başka bir sunucu kullan."),
  }[id] || id;
}

function nextBtn(ok, label = t('Devam')) {
  return html`<button class="btn p" data-next ${ok ? '' : 'disabled'}>${label}</button>`;
}

function body() {
  const s = state.snap, d = done(), busy = !!state.job?.running;
  switch (step) {
    case 'lang': {
      const cur = pickLang || getLang();
      const opt = (id, name, sub) => html`<button class="choice mode ${cur === id ? 'sel' : ''}" data-lang="${id}" style="flex:1">
        <div class="grow col" style="gap:4px"><b class="mt">${name}</b><div class="muted" style="font-size:12px">${sub}</div></div>
        ${cur === id ? html`<span class="chip up">${icon('check')}</span>` : ''}</button>`;
      return html`<section class="card pad col">
        <h2>Dil / Language</h2>
        <p class="muted" style="margin:0;font-size:13px">Uygulamanın dilini seç. · Choose the app's language.</p>
        <div class="row wrap" style="gap:12px;align-items:stretch">${opt('tr', 'Türkçe', 'Arayüz, bildirimler ve menü Türkçe')}${opt('en', 'English', 'Interface, notifications and menu in English')}</div>
        <div class="row" style="justify-content:flex-end"><button class="btn p" id="w-lang">${cur === 'en' ? 'Continue' : 'Devam'}</button></div>
      </section>`;
    }

    case 'os': {
      const os = state.info?.os, arch = state.info?.arch;
      const archName = os === 'macos' ? (arch === 'aarch64' ? t('Apple Silicon (arm64)') : t('Intel (x86_64)')) : arch;
      const card = (id, ic, name, sub) => {
        const here = os === id, sel = here && !srvPath();
        return html`<button class="choice mode oscard ${sel ? 'sel' : here ? '' : 'off'}" data-os="${here ? 'local' : ''}" ${here ? '' : 'disabled'} aria-pressed="${sel ? 'true' : 'false'}">
          <div class="osico">${icon(ic)}</div>
          <div class="grow col" style="gap:4px"><div class="row between"><b class="mt">${name}</b>${here ? html`<span class="chip up">${t('Bu bilgisayar')}</span>` : ''}</div>
            <div class="muted" style="font-size:12px">${here ? archName : sub}</div></div></button>`;
      };
      const srvCard = (id, name, sub) => {
        const sel = srvPath() && srvOs === id;
        return html`<button class="choice mode oscard ${sel ? 'sel' : ''}" data-os="server" data-srvos="${id}" aria-pressed="${sel ? 'true' : 'false'}">
          <div class="osico">${icon('server')}</div>
          <div class="grow col" style="gap:4px"><b class="mt">${name}</b>
            <div class="muted" style="font-size:12px">${sub}</div></div></button>`;
      };
      const unsupported = os === 'linux' && arch !== 'x86_64' && !srvPath();
      if (addMode()) return html`<section class="card pad col">
        <div class="row between wrap"><h2>${t('Sunucunun sistemi')}</h2><button class="btn p" id="w-os">${t('Devam')}</button></div>
        <p class="muted" style="margin:0;font-size:13px">${t('Yeni sunucu hangi sistemle çalışıyor? Kurulum SSH ile ona göre yapılır; bu bilgisayardaki kurulum ve diğer sunucular olduğu gibi kalır.')}</p>
        <div class="row wrap" style="gap:12px;align-items:stretch">
          ${srvCard('linux', t('Linux sunucu (SSH)'), t('x86_64 VPS · Linux yerel ya da Docker · SSH anahtarıyla kurulur ve buradan yönetilir'))}
          ${srvCard('windows', t('Windows sunucu (SSH)'), t('Windows 10/11 ya da Server 2019+ x64 · MT5 Wine\'sız çalışır · SSH ile kurulur ve buradan yönetilir'))}</div>
      </section>`;
      return html`<section class="card pad col">
        <div class="row between wrap"><h2>${t('İşletim sistemi')}</h2><button class="btn p" id="w-os" ${unsupported ? 'disabled' : ''}>${t('Devam')}</button></div>
        <p class="muted" style="margin:0;font-size:13px">${t('SiliconMetaTrader5 işletim sistemini kendisi algılar; kurulum bu bilgisayara göre yapılır. Diğer sistemde kullanmak için uygulamayı orada kur.')} ${t('MT5\'i bir sunucuda (VPS) çalıştırmak için sunucunun sistemini seç (Linux ya da Windows): kurulum SSH ile oraya yapılır. Sunucu bu bilgisayardaki kurulumla birlikte çalışır; yerel kurulum varsa olduğu gibi kalır.')}</p>
        <div class="row wrap" style="gap:12px;align-items:stretch">
          ${card('macos', 'laptop', 'macOS', t('Apple Silicon ve Intel Mac · Homebrew ile kurulur'))}
          ${card('linux', 'terminal', 'Linux', t('x86_64 · .deb, AppImage ya da Docker'))}</div>
        <span class="muted" style="font-size:12px;margin-top:4px">${t('Sunucuda (SSH)')}</span>
        <div class="row wrap" style="gap:12px;align-items:stretch">
          ${srvCard('linux', t('Linux sunucu (SSH)'), t('x86_64 VPS · Linux yerel ya da Docker · SSH anahtarıyla kurulur ve buradan yönetilir'))}
          ${srvCard('windows', t('Windows sunucu (SSH)'), t('Windows 10/11 ya da Server 2019+ x64 · MT5 Wine\'sız çalışır · SSH ile kurulur ve buradan yönetilir'))}</div>
        ${unsupported ? html`<div class="callout bad">${icon('alert')}<div>${t('Linux ARM64 desteklenmiyor: MT5 x86_64 bir Windows programıdır. x86_64 bir Linux ya da Mac kullan.')}</div></div>` : ''}
        ${os === 'linux' && !unsupported && !srvPath() ? html`<div class="callout info">${icon('info')}<div>${t('Linux x86_64\'te Wine doğrudan işlemcinde çalışır, emülasyon yoktur. Sonraki adımda Linux yerel (Docker\'sız, masaüstü penceresi) ya da Docker (container, sunucular için) seçersin; ikisi de aynı hızdadır.')}</div></div>` : ''}
      </section>`;
    }

    case 'mode': {
      // Before the configuration exists nothing is chosen yet.
      const shown = modes.map(m => ({ ...m, current: !!s?.env_exists && m.current }));
      return html`<section class="card pad col">
        <div class="row between wrap"><h2>${t('Çalışma modu')}</h2>${nextBtn(d.mode)}</div>
        <p class="muted" style="margin:0;font-size:13px">${mac() ? t("Mac'te MT5'i iki yoldan çalıştırabilirsin. Sonradan Runtime Ayarları'ndan değiştirebilirsin; her modun verisi ayrı kalır.")
          : t("Linux'ta MT5'i iki yoldan çalıştırabilirsin: masaüstünde Linux yerel, sunucuda Docker önerilir. Sonradan Runtime Ayarları'ndan değiştirebilirsin; her modun verisi ayrı kalır.")}</p>
        ${modes.length ? modeCards(shown, { disabled: busy }) : html`<p class="muted">${t('Taranıyor…')}</p>`}
      </section>`;
    }

    case 'prepare': {
      const n = st => (comps || []).filter(c => c.state === st).length;
      const user = n('user'), app = n('app');
      return html`<section class="card pad col">
        <div class="row between wrap"><div><h2>${t('Kurulu olanlar ve kurulacaklar')}</h2>
          <div class="muted" style="font-size:13px;margin-top:2px">${s?.provider || ''}</div></div>
          <div class="row"><button class="btn" id="w-rescan" ${scanning || busy ? 'disabled' : ''}>${icon('refresh', scanning ? 'spin' : '')}${t('Yeniden tara')}</button>${nextBtn(d.prepare)}</div></div>
        ${comps ? html`<div class="row wrap" style="gap:8px">
            <span class="chip up">${t('{n} kurulu', { n: n('ok') })}</span>
            ${app ? html`<span class="chip neu">${t('{n} kurulacak', { n: app })}</span>` : ''}
            ${user ? html`<span class="chip warn">${t('{n} senin yapman gereken', { n: user })}</span>` : ''}</div>
          <div>${comps.map(c => html`<div class="check">${dot(c.state)}<div class="grow"><div class="t">${c.name}</div><div class="s">${c.detail}</div>
              ${c.state === 'user' && c.fix ? fixLine(c.fix) : ''}</div>
            <span class="chip ${c.state === 'ok' ? 'up' : c.state === 'user' ? 'warn' : 'neu'}">${c.state === 'ok' ? t('kurulu') : c.state === 'user' ? t('terminalde') : t('kurulacak')}</span></div>`)}</div>`
        : html`<p class="muted">${t('Taranıyor…')}</p>`}
        ${user ? html`<div class="callout warn">${icon('alert')}<div>${t('İşaretli komutları Terminal\'de çalıştır (kopyala düğmesiyle), sonra Yeniden tara. Bunlar sistem bileşenleri; uygulama onları kendisi kurmaz.')}</div></div>` : ''}
        ${comps && !d.prepare ? html`<div class="row wrap">
          <button class="btn p" id="w-prepare" ${user || busy || scanning ? 'disabled' : ''}>${state.job?.running && ['prepare', 'install'].includes(state.job.kind) ? icon('spinner', 'spin') : icon('play')}${t('Eksikleri kur')}</button>
          <span class="muted" style="font-size:12px">${nat() ? (mac() ? t('Wine 11, Windows Python ve MT5 uygulamanın kendi klasörüne iner (~215 MB, birkaç dakika); sisteme bir şey kurulmaz.') : t('Wine 11, Windows Python ve MT5 uygulamanın kendi klasörüne iner (~100 MB, birkaç dakika); sisteme bir şey kurulmaz.'))
            : mac() ? t('Homebrew paketleri, x86_64 sanal makine ve runtime imajı. İmaj derlenirse 30–90 dk sürer; Mac bu sırada uyumaz.')
            : t('Runtime imajı ve yapılandırma.')}</span></div>` : ''}
      </section>`;
    }

    case 'start': {
      const ph = phaseOf(), st = s?.status, running = s?.container?.status === 'running';
      const ORDER = [['preparing_prefix', 'Wine prefix hazırlanıyor'], ['installing_mt5', 'MT5 kuruluyor'], ['starting_terminal', 'MT5 terminali açılıyor'], ['starting_bridge', 'Python köprüsü açılıyor'], ['waiting_login', 'Broker girişi bekleniyor']]
        .filter(([k]) => !(nat() && k === 'installing_mt5'));
      const idx = ORDER.findIndex(([k]) => k === st?.phase);
      const elapsed = st?.supervisor?.updated ? Date.now() / 1000 - st.supervisor.updated : null;
      return html`<section class="card pad col">
        <div class="row between wrap"><h2>${t('MT5\'i başlat')}</h2>${nextBtn(d.start)}</div>
        <div class="check">${dot(running ? 'ok' : null)}<div class="grow"><div class="t">${nat() ? t('Runtime (launchd)') : t('Runtime (container)')}</div>
          <div class="s">${running ? t('çalışıyor') : ph.key === 'engine_down' ? (s?.manages_vm ? t("VM kapalı · Başlat önce VM'i açar") : t('Docker Engine çalışmıyor')) : t('henüz başlatılmadı')}</div></div>
          ${running ? html`<button class="btn" data-job="stop" ${busy ? 'disabled' : ''}>${icon('stop')}${t('Durdur')}</button>` : html`<button class="btn p" data-job="start" ${busy ? 'disabled' : ''}>${state.job?.running && state.job.kind === 'start' ? icon('spinner', 'spin') : icon('play')}${t('Başlat')}</button>`}</div>
        ${running ? html`<div class="col" style="gap:10px;margin-top:4px">${ORDER.map(([k, label], i) => {
          const s_ = st?.phase === 'ready' || (idx >= 0 && i < idx) ? 'ok' : i === idx ? (k === 'waiting_login' ? 'ok' : 'run') : st?.phase === 'error' && i === Math.max(0, idx) ? 'bad' : 'wait';
          return html`<div class="step"><span class="dot ${s_}">${s_ === 'ok' ? icon('check') : s_ === 'run' ? icon('spinner', 'spin') : s_ === 'bad' ? icon('x') : ''}</span>${t(label)}${i === idx && k !== 'waiting_login' && elapsed ? html`<span class="muted" style="font-weight:400">· ${duration(elapsed)}${st?.detail ? ' · ' + st.detail : ''}</span>` : ''}</div>`;
        })}</div>` : ''}
        ${ph.key === 'error' ? html`<div class="callout bad">${icon('alert')}<div>${st?.detail || t('Runtime hata verdi.')} <a href="#/logs">${t('Loglar')}</a></div></div>` : ''}
        <p class="muted" style="margin:0;font-size:13px">${nat()
          ? t("MT5'in ilk açılışı yaklaşık bir dakika sürer; sonrakiler daha hızlıdır. Uygulamayı kapatabilirsin, runtime arka planda (launchd) devam eder.")
          : t("İlk açılışta Wine prefix'i kopyalanır ve MT5 kurulur; emülasyon altında birkaç dakika ile yarım saat arası sürebilir. Bu sırada uygulamayı kapatabilirsin, runtime arka planda devam eder.")}</p>
      </section>`;
    }

    case 'srv_conn': {
      const id = activeId(), keys = probe?.keys || [];
      const head = html`<div class="row between wrap"><h2>${t('Sunucuya bağlan')}</h2>${nextBtn(d.srv_conn)}</div>
        <p class="muted" style="margin:0;font-size:13px">${t('Uygulama sistemin ssh programını ve SSH anahtarlarını kullanır; hiçbir şifre istemez ve saklamaz. Sunucuda dışarı açık port gerekmez: her şey SSH tünelinden geçer.')}</p>`;
      if (!id) return html`<section class="card pad col">${head}
        <div class="grid g2" style="gap:12px">
          <label class="col" style="gap:4px"><span class="muted" style="font-size:12px">${t('Ad')}</span><input class="f" id="srv-name" data-form="name" value="${form.name}" placeholder="${t('ör. Frankfurt VPS')}" maxlength="40"></label>
          <label class="col" style="gap:4px"><span class="muted" style="font-size:12px">${t('Sunucu adresi')}</span><input class="f mono" id="srv-ssh" data-form="ssh" value="${form.ssh}" placeholder="root@203.0.113.10" spellcheck="false" autocapitalize="off">
            <span class="muted" style="font-size:11px">${th('<b>kullanıcı adı</b> @ <b>sunucunun IP\'si</b>. Sağlayıcı farklı bir port verdiyse sonuna ekle: <span class="mono">root@203.0.113.10:2222</span>')}</span></label>
          <div class="col" style="gap:6px;grid-column:1/-1"><span class="muted" style="font-size:12px">${t('Giriş yöntemi')}</span>
            <div class="seg" role="group" style="align-self:flex-start"><button data-auth="password" class="${authMethod === 'password' ? 'sel' : ''}">${t('Şifre')}</button><button data-auth="key" class="${authMethod === 'key' ? 'sel' : ''}">${t('Anahtar dosyası')}</button></div>
            ${authMethod === 'key' ? html`<input class="f mono" id="srv-key" data-form="key" value="${form.key}" placeholder="~/Downloads/sunucum.pem" spellcheck="false" autocapitalize="off">
              <span class="muted" style="font-size:11px">${t('Sağlayıcının verdiği .pem ya da özel anahtar dosyasının yolu.')}</span>`
            : html`<span class="muted" style="font-size:11px">${t('Şifreyi buraya yazmazsın: bağlantıyı denedikten sonra açılan Terminal penceresine bir kez yazarsın. Uygulama bu sırada bu bilgisayar için bir anahtar kurar; sonraki bağlantılar şifresiz olur.')}</span>`}</div></div>
        <details class="fn" id="srv-guide"><summary><b>${t('Sunucuya SSH ile bağlanma')}</b><span class="muted grow fdesc">${t('7 kısa adım')}</span>${icon('chev')}</summary>
          <ol class="rules" style="margin:4px 0 8px">${SSH_GUIDE.map(r => html`<li>${raw(r[getLang()] || r.tr)}</li>`)}</ol></details>
        <div class="row"><button class="btn p" id="srv-probe" ${srvBusy ? 'disabled' : ''}>${icon(srvBusy ? 'spinner' : 'plug', srvBusy ? 'spin' : '')}${t('Bağlantıyı dene')}</button></div>
        ${srvErr ? html`<div class="callout bad">${icon('alert')}<div>${srvErr}</div></div>` : ''}
        ${srvErr && !probe && srvOs === 'windows' ? html`<div class="callout warn" id="srv-win-ssh">${icon('info')}<div class="col" style="gap:8px">
            <div>${th('Windows sunucularda SSH kendiliğinden açık değildir. RDP (Windows App) ile sunucuya gir, <b>yönetici PowerShell</b> aç ve şunu çalıştır:')}</div>
            ${fixLine(WIN_SSH_ON)}
            <div class="muted" style="font-size:12px">${t('Son satır True yazmalı. Yazıyorsa ama burada hâlâ bağlanamıyorsan, sağlayıcının panelindeki güvenlik duvarında (firewall / security group) TCP 22 girişine izin ver.')}</div></div></div>` : ''}
        ${keys.length ? html`<div class="callout info" id="srv-fp">${icon('info')}<div class="col" style="gap:6px">
            <div>${th('<b>{host}</b> sunucusu bu kimlikle yanıt verdi. Emin olmak için parmak izini sunucu sağlayıcının paneliyle ya da sunucuda <span class="mono">ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub</span> çıktısıyla karşılaştır.', { host: `${probe.host}:${probe.port}` })}</div>
            ${keys.map(k => html`<div class="mono selectable" style="font-size:12px">${k.kind} · ${k.sha256}</div>`)}
            ${osDiffers() ? html`<div class="callout warn" id="srv-os-differs">${icon('info')}<div>${probe.os === 'windows' ? t('Linux sunucu seçtin ama bu sunucu Windows olarak yanıt verdi; kurulum Windows\'a göre yapılacak.') : t('Windows sunucu seçtin ama bu sunucu Linux olarak yanıt verdi; kurulum Linux\'a göre yapılacak.')}</div></div>` : ''}
            <div><button class="btn p sm" id="srv-accept" ${srvBusy ? 'disabled' : ''}>${icon('check')}${t('Parmak izini onayla ve ekle')}</button></div></div></div>` : ''}
      </section>`;
      const k = srvTest?.kind;
      return html`<section class="card pad col">${head}
        <div class="check">${dot(srvTest?.ok ? 'ok' : srvTest ? 'bad' : 'run')}<div class="grow"><div class="t">${s?.target_name || id}</div><div class="s mono">${probe?.dest || ''}</div></div></div>
        ${!srvTest?.ok && waiting ? html`<div class="callout info" id="srv-wait">${icon('spinner', 'spin')}<div class="col" style="gap:8px"><div>${t("Terminal'de şifreni bekliyorum. Şifreyi yazınca burası birkaç saniye içinde kendiliğinden ilerler.")}</div>
              ${isWin() && keyLine ? html`<div id="srv-keyline">${fixLine(keyLine)}</div>` : ''}
              <div><button class="btn sm" id="srv-retest">${icon('refresh')}${t('Tekrar dene')}</button></div></div></div>`
          : srvTest?.ok ? html`<div class="callout info" id="srv-conn-ok">${icon('check')}<div>${t('Bağlantı tamam: SSH anahtarıyla girildi. Sıradaki adımda çalışma modunu seç.')}</div></div>
            ${srvTest.os_mismatch ? html`<div class="callout warn" id="srv-os-mismatch">${icon('alert')}<div>${t('Sunucu {seen} olduğunu bildiriyor ama {saved} olarak kayıtlı. Sunucuyu Sunucular sayfasından kaldırıp doğru türle yeniden ekle.', { seen: srvTest.os === 'windows' ? 'Windows' : 'Linux', saved: srvTest.os === 'windows' ? 'Linux' : 'Windows' })}</div></div>` : ''}`
          : k === 'key_required' && authMethod === 'password' && isWin() ? html`<div class="callout warn">${icon('key')}<div class="col" style="gap:8px"><div>${t('Şimdi Windows şifrenle bir kez giriş yap: Terminal açılır, sunucu şifreni orada yazarsın. Uygulama bu bilgisayarın anahtarını sunucuda yöneticiler için administrators_authorized_keys dosyasına ekler (yönetici olmayan kullanıcı için authorized_keys). Bitince Tekrar dene.')}</div>
              ${keyLine ? html`<div id="srv-keyline">${fixLine(keyLine)}</div>` : ''}
              <div class="row wrap"><button class="btn p sm" id="srv-keygen">${icon('terminal')}${t("Terminal'de şifremi gir")}</button><button class="btn sm" id="srv-retest">${icon('refresh')}${t('Tekrar dene')}</button></div></div></div>`
          : k === 'key_required' && authMethod === 'password' ? html`<div class="callout warn">${icon('key')}<div class="col" style="gap:8px"><div>${t('Şimdi şifreyle bir kez giriş yap: Terminal açılır, sunucu şifreni orada yazarsın. Uygulama bu sırada bu bilgisayar için bir anahtar kurar; bitince Tekrar dene.')}</div>
              <div class="row wrap"><button class="btn p sm" id="srv-keygen">${icon('terminal')}${t("Terminal'de şifremi gir")}</button><button class="btn sm" id="srv-retest">${icon('refresh')}${t('Tekrar dene')}</button></div></div></div>`
          : k === 'key_required' ? html`<div class="callout warn">${icon('alert')}<div class="col" style="gap:8px"><div>${t('Bu anahtar dosyasıyla girilemedi: dosya doğru mu ve sunucu bu anahtarı tanıyor mu? Şifren de varsa, şifreyle bir kez girip bu bilgisayar için bir anahtar kurabilirsin.')}</div>
              <div class="row wrap"><button class="btn sm" id="srv-retest">${icon('refresh')}${t('Tekrar dene')}</button><button class="btn sm" id="srv-keygen">${icon('terminal')}${t("Terminal'de şifremi gir")}</button></div></div></div>`
          : k === 'host_key_changed' ? html`<div class="callout bad">${icon('alert')}<div class="col" style="gap:8px"><div>${t('Sunucu kimliği değişti. Sunucuyu yeniden kurmadıysan bu bir saldırı olabilir; bağlantı reddedildi.')}</div><div><button class="btn sm" id="srv-retest">${icon('refresh')}${t('Tekrar dene')}</button></div></div></div>`
          : k ? html`<div class="callout bad">${icon('alert')}<div class="col" style="gap:8px"><div>${srvTest.detail || k}</div><div><button class="btn sm" id="srv-retest">${icon('refresh')}${t('Tekrar dene')}</button></div></div></div>`
          : html`<p class="muted">${t('Bağlantı deneniyor…')}</p>`}
      </section>`;
    }

    case 'srv_mode': {
      const cur = srvMode || s?.provider_kind || 'linux-native';
      const card = (id, name, sub, badge) => html`<button class="choice mode big ${srvModeSeen && cur === id ? 'sel' : ''}" data-smode="${id}" style="flex:1">
        <div class="grow col" style="gap:6px"><div class="row between"><b class="mt">${name}</b>${badge ? html`<span class="chip up">${badge}</span>` : ''}</div><div class="muted" style="font-size:13px">${sub}</div></div></button>`;
      return html`<section class="card pad col">
        <div class="row between wrap"><h2>${t('Sunucuda çalışma modu')}</h2>${nextBtn(d.srv_mode)}</div>
        <div class="row wrap" style="gap:12px;align-items:stretch">
          ${isWin() ? card('windows', 'Windows', t("MT5 doğrudan Windows'ta çalışır, Wine yok. Python, MT5, TightVNC ve noVNC kullanıcının klasörüne kurulur; ekran noVNC ile buraya gelir."), t('Önerilen')) : html`
          ${card('linux-native', t('Linux yerel'), t("Docker gerekmez, daha hafif. Wine 11 ve MT5 kullanıcının ev klasörüne kurulur; ekran noVNC ile buraya gelir. Kurulum ~2–3 dk."), t('Önerilen'))}
          ${card('docker', 'Docker', t('İzole container. Sunucuda Docker yoksa paketleriyle kurulur. İlk imaj derlemesi 10–25 dk sürer.'), '')}`}</div>
      </section>`;
    }

    case 'srv_check': {
      const P = new Set((pc?.problems || []).map(p => p.id)), pf = pc?.preflight, mode = srvMode || s?.provider_kind || 'linux-native';
      const gb = mb => (mb / 1024).toFixed(1).replace(/\.0$/, '') + ' GB';
      const win = pc?.win, blocking = new Set((pc?.problems || []).filter(p => p.blocking).map(p => p.id));
      const rows = win ? [
        ['win_version', t('Windows 10 1809 / Server 2019 ya da yenisi'), `${win.os_caption} · build ${win.build}`],
        ['arch', t('İşlemci AMD64'), win.arch],
        ['not_admin', t('Yönetici hakları'), '', t('Yönetici hakları yok')],
        ['disk', t('Boş disk en az 3 GB'), gb(win.disk_free_mb)],
        ['mem', t('Bellek en az 2 GB'), gb(win.mem_mb)],
        ['no_tar', 'tar.exe', '', t('tar.exe bulunamadı')],
        ['network', t('İndirme adreslerine erişim'), '', t('İndirme adreslerine erişilemiyor')],
        ['pending_reboot', t('Bekleyen yeniden başlatma yok'), '', t('Bekleyen bir yeniden başlatma var')],
        ['logon_banner', t('Giriş öncesi uyarı ekranı yok'), '', t('Giriş öncesi uyarı ekranı var')],
        ['foreign_mt5', t('Başka bir MT5 kurulumu yok'), '', t('Başka bir MT5 kurulumu var')],
      ] : pf ? [
        ['arch', t('İşlemci x86_64'), pf.arch],
        ['disk', t('Boş disk en az 3 GB'), gb(pf.disk_free_mb)],
        ['mem', t('Bellek en az 2 GB'), gb(pf.mem_mb)],
        ['packages', t('Gerekli paketler'), mode === 'docker' ? t('docker, docker compose') : t('Xvfb, x11vnc, websockify, noVNC, python3-venv, curl, xz, xclip')],
        mode === 'docker' ? ['docker_group', t('Docker erişimi (docker grubu)'), ''] : ['linger', t("SSH'tan çıkınca da çalışma (linger)"), t('Kapalıysa SSH oturumu kapanınca MT5 de durur')],
      ] : [];
      return html`<section class="card pad col">
        <div class="row between wrap"><div><h2>${t('Ön kontrol')}</h2><div class="muted" style="font-size:13px;margin-top:2px">${pf?.os_pretty || pc?.win?.os_caption || ''}</div></div>
          <div class="row"><button class="btn" id="srv-recheck" ${checking ? 'disabled' : ''}>${icon('refresh', checking ? 'spin' : '')}${t('Tekrar kontrol et')}</button>${nextBtn(d.srv_check)}</div></div>
        ${srvErr ? html`<div class="callout bad">${icon('alert')}<div>${srvErr}</div></div>` : ''}
        ${pf || win ? html`<div id="srv-check-list">${rows.map(([id, okName, detail, badName]) => {
            const st = !P.has(id) ? 'ok' : blocking.has(id) ? 'bad' : 'warn';
            const name = P.has(id) && badName ? badName : okName;
            return html`<div class="check ${st}" data-problem="${id}">${dot(st === 'warn' ? 'user' : st)}<div class="grow"><div class="t">${name}</div><div class="s">${P.has(id) ? winHint(id) : detail}</div>
              ${id === 'logon_banner' && P.has(id) && win ? html`<div style="margin-top:6px">${fixLine(WIN_NO_BANNER)}</div>` : ''}
              ${id === 'pending_reboot' && P.has(id) ? html`<div style="margin-top:6px"><button class="btn sm" id="srv-reboot-now" ${rebootNow ? 'disabled' : ''}>${icon(rebootNow ? 'spinner' : 'refresh', rebootNow ? 'spin' : '')}${t('Şimdi yeniden başlat')}</button></div>` : ''}</div></div>`;
          })}</div>`
          : html`<p class="muted">${checking ? t('Sunucu kontrol ediliyor…') : ''}</p>`}
        ${P.has('no_apt') ? html`<div class="callout bad">${icon('alert')}<div>${t('Bu sunucuda apt yok: paketleri dağıtımının paket yöneticisiyle kendin kur, sonra Tekrar kontrol et.')}</div></div>` : ''}
        ${pc?.fix && fixWaiting ? html`<div class="callout info" id="srv-fix-wait">${icon('spinner', 'spin')}<div>${t("Terminal'deki komutun bitmesini bekliyorum (sudo şifresi, paket kurulumu). Bitince bu liste kendiliğinden güncellenir.")}</div></div>` : ''}
        ${pc?.fix ? html`<div class="callout warn">${icon('alert')}<div class="col" style="gap:8px"><div>${t('Bu komut sudo ister. Terminal\'de açılır; şifreyi orada gir, bitince Tekrar kontrol et.')}</div>${fixLine(pc.fix)}
            <div><button class="btn p sm" id="srv-fix">${icon('terminal')}${t("Terminal'de kur")}</button></div></div></div>` : ''}
      </section>`;
    }

    case 'srv_install': {
      const ph = phaseOf(), running = state.job?.running && state.job.kind === 'server_install', failed = state.job?.kind === 'server_install' && state.job?.code && !state.job.running;
      return html`<section class="card pad col">
        <div class="row between wrap"><h2>${t('Sunucuya kur ve başlat')}</h2>${nextBtn(d.srv_install)}</div>
        <div class="check">${dot(d.srv_install ? 'ok' : running ? 'run' : failed ? 'bad' : null)}<div class="grow"><div class="t">${s?.target_name || ''}</div><div class="s">${ph.label}</div></div>
          ${d.srv_install ? '' : html`<button class="btn p" id="srv-install" ${busy ? 'disabled' : ''}>${running ? icon('spinner', 'spin') : icon('play')}${failed ? t('Tekrar dene') : t('Kur ve başlat')}</button>`}</div>
        <p class="muted" style="margin:0;font-size:13px">${t('Runtime sunucuya gönderilir, sunucunun kendi şifresi ve token\'ı orada üretilir, MT5 kurulup başlatılır. Çalışan her komut aşağıdaki pencerede görünür. Uygulamayı kapatsan da sunucu çalışmaya devam eder.')}</p>
      </section>`;
    }

    case 'srv_autologon': return html`<section class="card pad col" id="srv-autologon">
        <div class="row between wrap"><h2>${t('Otomatik Windows oturumu')}</h2>${nextBtn(d.srv_autologon)}</div>
        <p class="muted" style="margin:0;font-size:13px">${t("MT5 bir Windows oturumunda çalışır. Sunucu yeniden başladığında MT5'in kendiliğinden açılması için Windows'un oturumu kendiliğinden açması gerekir.")}</p>
        <div class="callout warn">${icon('alert')}<div>${t("Ödünleşim: Windows şifren sunucuda Windows'un gizli deposunda saklanır ve açılışta oturum şifresiz açılır; sağlayıcının konsolunda masaüstü görünür. Şifreyi uygulama hiç görmez: onu yalnız Terminal'e, doğrudan sunucuya yazarsın.")}</div></div>
        ${autoHint && !d.srv_autologon ? html`<div class="callout bad" id="srv-autologon-hint">${icon('alert')}<div>${autoHint}</div></div>` : ''}
        <div class="check">${dot(d.srv_autologon ? 'ok' : autoWaiting ? 'run' : null)}<div class="grow"><div class="t">${t('Otomatik oturum açma')}</div><div class="s">${d.srv_autologon ? t('açık') : autoWaiting ? t("Terminal'de şifreni bekliyorum.") : t('kapalı')}</div></div>
          ${d.srv_autologon ? '' : html`<button class="btn p" id="srv-autologon-open">${icon('terminal')}${t("Terminal'de aç")}</button>`}</div>
        ${autoWaiting && !d.srv_autologon ? html`<div class="callout info" id="srv-autologon-wait">${icon('spinner', 'spin')}<div>${t("Terminal'de Windows şifreni yaz. Doğrulanınca burası birkaç saniye içinde kendiliğinden ilerler.")}</div></div>` : ''}
      </section>`;

    case 'srv_reboot': return html`<section class="card pad col" id="srv-reboot">
        <div class="row between wrap"><h2>${t('Sunucuyu yeniden başlat')}</h2>${nextBtn(d.srv_reboot)}</div>
        <p class="muted" style="margin:0;font-size:13px">${t('Sunucu bir kez yeniden başlatılacak (~1–2 dk). Tünel kendiliğinden yeniden bağlanır; Windows oturumu açılınca MT5 de kendiliğinden başlar.')}</p>
        <div class="check">${dot(d.srv_reboot ? 'ok' : rebooting ? 'run' : null)}<div class="grow"><div class="t">${s?.target_name || ''}</div><div class="s">${d.srv_reboot ? (rebootDone ? t('yeniden başlatıldı, oturum açık') : t('oturum zaten açık, yeniden başlatma gerekmez')) : rebooting ? phaseOf().label : t('henüz başlatılmadı')}</div></div>
          ${d.srv_reboot ? '' : html`<button class="btn p" id="srv-reboot-go" ${rebooting ? 'disabled' : ''}>${icon(rebooting ? 'spinner' : 'refresh', rebooting ? 'spin' : '')}${t('Şimdi yeniden başlat')}</button>`}</div>
        ${rebooting ? html`<div class="callout info" id="srv-reboot-wait">${icon('spinner', 'spin')}<div>${t('Sunucu yeniden başlıyor; açılınca burası kendiliğinden ilerler. 10 dakikaya kadar beklenir.')}</div></div>` : ''}
        ${srvErr ? html`<div class="callout bad">${icon('alert')}<div>${srvErr}</div></div>` : ''}
      </section>`;

    case 'login': return html`<section class="card pad col">
        <div class="row between wrap"><h2>${t('Broker hesabına giriş')}</h2>${nextBtn(d.login)}</div>
        <ol class="muted" style="margin:0;padding-left:18px;font-size:13px;line-height:1.8">
          <li>${nativeWindow() ? t("MT5 kendi macOS penceresinde açılır (Dock'ta simgesi görünür).") : th("<b>MT5 Ekranı</b>'nı aç (şifre otomatik girilir).")}</li>
          <li>${th("MT5'te <b>File › Open an Account</b> ile broker'ını bul veya <b>File › Login to Trade Account</b> ile giriş yap.")}</li>
          <li>${th('Giriş yapılınca durum <b>Hazır</b> olur ve bu adım kendiliğinden tamamlanır.')}</li></ol>
        <div class="callout info">${icon('info')}<div>${th("Durum, MT5 broker sunucusuna gerçekten bağlanınca <b>Hazır</b> olur. Bunu MT5 penceresinin <b>sağ alt köşesinde</b> görürsün: yeşil sinyal çubukları ve <span class=\"mono\">5305 / 24 Kb</span> gibi bir değer. Orada <b>No connection</b> ya da <b>Invalid account</b> yazıyorsa sunucu adını, hesap numarasını ve şifreyi kontrol et. Bağlantı geldikten sonra burası birkaç saniye içinde güncellenir.")}</div></div>
        <div class="row">${nativeWindow() ? html`<button class="btn p" id="w-focus">${icon('window')}${t('MT5 penceresini öne getir')}</button>` : html`<a class="btn p" href="#/screen" id="w-screen">${icon('screen')}${t('MT5 Ekranını aç')}</a>`}
          <span class="chip lg ${phaseOf().tone}"><span class="d"></span>${phaseOf().label}${s?.status?.bridge?.probe?.account?.server ? ' · ' + s.status.bridge.probe.account.server : ''}</span></div>
      </section>`;

    case 'test': return html`<div class="row between"><div class="muted" style="font-size:13px">${t('Python istemcisiyle aynı yoldan köprüye bağlanıp bar ve tick çekiyoruz.')}</div>${nextBtn(d.test)}</div><div class="col" id="w-dt"></div>`;

    case 'done': {
      const a = s?.status?.bridge?.probe?.account;
      return html`<section class="card empty"><img class="logo-img" src="${document.documentElement.dataset.theme === 'dark' ? 'img/logo-dark.svg' : 'img/logo-light.svg'}" alt="">
        <h2 style="font-size:20px">${t('SiliconMetaTrader5 hazır')}</h2>
        <p>${a?.server ? t('{server} hesabı bağlı.', { server: a.server }) + ' ' : ''}${t("Python'dan bağlanmak için:")}</p>
        <pre class="code selectable" style="text-align:left">from siliconmetatrader5 import MetaTrader5\nmt5 = MetaTrader5(host="127.0.0.1", port=${s?.bridge_port || 8001})\nmt5.initialize()\nprint(mt5.copy_rates_from_pos("EURUSD", mt5.TIMEFRAME_M5, 0, 10))</pre>
        <div class="row"><button class="btn p" id="w-finish" ${d.done ? '' : 'disabled'}>${t('Uygulamaya geç')}</button></div></section>`;
    }
  }
  return html``;
}

function draw() {
  if (!el) return;
  const d = done(), list = steps();
  if (!step || !reachable(step)) step = firstOpen();
  const keepDt = step === 'test' && dt && $('#w-dt', el);
  if (keepDt) { // don't wipe a running data test on every status poll
    $$('.wstep', el).forEach(b => b.classList.toggle('on', b.dataset.s === step));
    const n = $('[data-next]', el); if (n) n.disabled = !d.test;
    return;
  }
  render(el, html`
    <div class="head"><div><div class="crumbs">SiliconMetaTrader5 › <b>${t('Kurulum Sihirbazı')}</b></div><h1>${t('Sistemini hazırlayalım')}</h1>
      <div class="muted" style="margin-top:4px;max-width:720px">${t('Uygulama neyin kurulu olduğunu bulur, eksikleri onayınla kurar ve MT5\'i başlatır. Hiçbir komut sana sorulmadan çalışmaz; çalışan her komut aşağıdaki pencerede görünür.')}</div></div></div>
    <div class="steps">${list.map(([k, label], i) => html`<button class="wstep ${k === step ? 'on' : ''}" data-s="${k}" ${reachable(k) ? '' : 'disabled'}><span class="dot ${d[k] ? 'ok' : k === step ? 'run' : 'wait'}">${d[k] ? icon('check') : i + 1}</span>${k === 'lang' ? 'Dil / Language' : t(label)}</button>`)}</div>
    <div class="col">${body()}</div>`);

  $$('.wstep', el).forEach(b => b.onclick = () => { if (reachable(b.dataset.s)) { step = b.dataset.s; dt = null; draw(); } });
  $$('[data-next]', el).forEach(b => b.onclick = () => { const i = list.findIndex(([k]) => k === step); step = list[Math.min(list.length - 1, i + 1)][0]; dt = null; draw(); });
  $$('[data-copy]', el).forEach(b => b.onclick = () => copyText(b.dataset.copy));
  $('#w-rescan', el)?.addEventListener('click', scan);
  $$('[data-lang]', el).forEach(b => b.onclick = () => { pickLang = b.dataset.lang; draw(); });
  $('#w-os', el)?.addEventListener('click', () => {
    osSeen = true; try { localStorage.setItem('smt5.osSeen', '1'); } catch { /* private mode */ }
    const list = steps(), i = list.findIndex(([k]) => k === 'os');
    step = list[i + 1][0]; draw();
  });
  $('#w-lang', el)?.addEventListener('click', async () => {
    try {
      await call('set_lang', { lang: pickLang || getLang() });
      location.hash = '#/setup';
      location.reload(); // every text, the tray and the backend messages switch together
    } catch (e) { toast(String(e), true); }
  });
  $$('[data-mode]', el).forEach(b => b.onclick = async () => {
    try {
      if (state.snap?.container?.status === 'running') await waitJob(await startJob('stop'));
      await call('set_mode', { mode: b.dataset.mode });
      await refreshSnapshot();
      comps = null;
      await scan();
    } catch (e) { toast(String(e), true); }
  });
  $('#w-prepare', el)?.addEventListener('click', async () => {
    try {
      await call('init_env');
      await refreshSnapshot();
      comps = await call('components');
      if (comps.some(c => c.state === 'app')) {
        const id = await startJob('prepare');
        draw();
        await waitJob(id);
        await refreshSnapshot();
      }
      await scan();
    } catch (e) { toast(String(e), true); scan(); }
  });
  $$('[data-job]', el).forEach(b => b.onclick = async () => {
    try {
      const id = await startJob(b.dataset.job);
      draw();
      await waitJob(id);
      await refreshSnapshot();
      draw();
    } catch (e) { toast(String(e), true); }
  });
  $$('[data-form]', el).forEach(i => i.oninput = () => { form[i.dataset.form] = i.value; });
  $$('[data-auth]', el).forEach(b => b.onclick = () => { authMethod = b.dataset.auth; srvErr = null; draw(); });
  $$('[data-os]:not([data-os=""])', el).forEach(b => b.onclick = () => {
    osPick = b.dataset.os; store('smt5.osPick', osPick);
    if (b.dataset.srvos) { srvOs = b.dataset.srvos; store('smt5.srvOs', srvOs); }
    draw();
  });
  $('#srv-probe', el)?.addEventListener('click', async () => {
    const ssh = form.ssh.trim();
    srvForm = { name: form.name.trim() || ssh, ssh, identity: authMethod === 'key' ? (form.key.trim() || null) : null };
    if (authMethod === 'key' && !srvForm.identity) { srvErr = t('Anahtar dosyasının yolunu yaz ya da Şifre\'yi seç.'); draw(); return; }
    srvBusy = true; srvErr = null; probe = null; draw();
    try { probe = await call('server_probe', { ssh }); } catch (e) { srvErr = String(e); }
    srvBusy = false; draw();
  });
  $('#srv-accept', el)?.addEventListener('click', async () => {
    const key = (probe.keys.find(k => k.kind === 'ED25519') || probe.keys[0]).sha256;
    srvBusy = true; draw();
    try {
      const sv = await call('server_add', { name: srvForm.name, ssh: srvForm.ssh, identity: srvForm.identity, sha256: key });
      if (isWin()) await call('server_set_mode', { id: sv.id, mode: 'windows' }); // key setup and later steps depend on it
      await call('set_active_target', { id: sv.id });
      addedId = sv.id;
      await refreshInfo(); await refreshSnapshot();
      await retest();
    } catch (e) { srvErr = String(e); }
    srvBusy = false; draw();
  });
  $('#srv-retest', el)?.addEventListener('click', retest);
  $('#srv-keygen', el)?.addEventListener('click', async () => {
    try { const r = await call('server_keygen', { id: activeId() }); if (isWin()) keyLine = r?.line || null; waitForKey(); } catch (e) { toast(String(e), true); }
  });
  $$('[data-smode]', el).forEach(b => b.onclick = async () => {
    try { await call('server_set_mode', { id: activeId(), mode: b.dataset.smode }); srvMode = b.dataset.smode; srvModeSeen = true; pc = null; draw(); } catch (e) { toast(String(e), true); }
  });
  $('#srv-recheck', el)?.addEventListener('click', precheck);
  $('#srv-autologon-open', el)?.addEventListener('click', async () => {
    try { autoHint = null; await call('server_autologon', { id: activeId() }); waitForAutologon(); } catch (e) { toast(String(e), true); }
  });
  $('#srv-reboot-go', el)?.addEventListener('click', async () => {
    try { srvErr = null; await call('server_reboot', { id: activeId() }); waitForSession(); } catch (e) { srvErr = String(e); draw(); }
  });
  $('#srv-reboot-now', el)?.addEventListener('click', async () => {
    try { await call('server_reboot', { id: activeId() }); waitForRestart(); } catch (e) { toast(String(e), true); }
  });
  $('#srv-fix', el)?.addEventListener('click', async () => {
    try { await call('server_fix', { id: activeId(), mode: srvMode || state.snap?.provider_kind || 'linux-native' }); waitForFix(); } catch (e) { toast(String(e), true); }
  });
  $('#srv-install', el)?.addEventListener('click', async () => {
    try {
      const id = await startJob('server_install');
      draw();
      await waitJob(id);
      await refreshSnapshot();
      draw();
    } catch (e) { toast(String(e), true); }
  });
  if (step === 'srv_check' && !pc && !checking && !srvErr) precheck();
  if (step === 'srv_autologon' && !autoOn && !autoWaiting && !autoChecked) checkAutologon();
  if (step === 'srv_conn' && activeId() && !srvTest && !srvBusy) retest();
  $('#w-focus', el)?.addEventListener('click', () => call('focus_mt5').catch(e => toast(String(e), true)));
  $('#w-finish', el)?.addEventListener('click', async () => {
    try {
      await call('set_setup_complete', { done: true });
      await refreshInfo();
      location.hash = '#/overview';
      location.reload();
    } catch (e) { toast(String(e), true); }
  });
  if (step === 'test') dt = datatestView($('#w-dt', el), { compact: true, onResult: r => { testResult = r; draw(); } });
}

let srvForm = null, autoChecked = false;
const sleep = ms => new Promise(r => setTimeout(r, ms));
/** Entering the autologon step: it may already be on (wizard reopened). */
async function checkAutologon() {
  autoChecked = true;
  try { if (await call('server_autologon_status', { id: activeId() })) { autoOn = true; draw(); } } catch { /* the user can still open Terminal */ }
}
/** After Terminal opened: ask every 3 s (up to 10 min) until autologon is on, then go on by ourselves. */
async function waitForAutologon() {
  const run = autoWaiting = ++SEQ;
  draw();
  const until = Date.now() + 600000;
  while (autoWaiting === run && el && Date.now() < until) {
    await sleep(3000);
    if (autoWaiting !== run || !el || !activeId()) return;
    if (step !== 'srv_autologon') { autoWaiting = 0; autoChecked = false; return; }
    try { if (await call('server_autologon_status', { id: activeId() })) { autoOn = true; break; } } catch { /* keep waiting */ }
  }
  if (autoWaiting === run) {
    autoWaiting = 0;
    if (autoOn && step === 'srv_autologon') step = 'srv_reboot';
    draw();
  }
}
/** After the restart was requested: every 5 s (up to 10 min) until the Windows session is open.
 *  no_session after the server was down: Windows is up but nobody logged on yet. Back to the
 *  autologon step at once when autologon is off; otherwise wait (logon can be slow) and only
 *  after LOGON_GRACE_MS go back with a hint. */
async function waitForSession() {
  const run = rebooting = ++SEQ, t0 = Date.now();
  let sawDown = false, noSessSince = 0;
  const back = hint => { autoOn = false; autoChecked = true; autoHint = hint; rebooting = 0; if (step === 'srv_reboot') step = 'srv_autologon'; draw(); };
  draw();
  while (rebooting === run && el && Date.now() - t0 < 600000) {
    await sleep(5000);
    if (rebooting !== run || !el) return;
    if (step !== 'srv_reboot') { rebooting = 0; return; }
    await refreshSnapshot();
    const k = state.snap?.lifecycle;
    if (['unreachable', 'connecting'].includes(k)) sawDown = true;
    if (hasSession()) { rebootDone = true; break; }
    if (k === 'no_session' && (sawDown || Date.now() - t0 > 120000)) {
      if (!noSessSince) noSessSince = Date.now();
      let on = null;
      try { on = await call('server_autologon_status', { id: activeId() }); } catch { /* still starting: keep waiting */ }
      if (rebooting !== run || !el) return;
      if (on === false) { back(null); return; }
      if (Date.now() - noSessSince > LOGON_GRACE_MS) { back(t('Windows açıldı ama oturum açılmadı. Otomatik oturumu Terminal\'de yeniden aç (şifre doğru mu?), sonra yeniden başlat.')); return; }
    } else noSessSince = 0;
    draw();
  }
  if (rebooting === run) {
    rebooting = 0;
    if (rebootDone) { if (step === 'srv_reboot') step = 'login'; } else srvErr = t('Sunucu 10 dakikada Windows oturumuna ulaşmadı. Sağlayıcının konsolundan bakıp tekrar dene.');
    draw();
  }
}
/** "Şimdi yeniden başlat" on the precheck: check again every 5 s until the pending restart is gone. */
async function waitForRestart() {
  const run = rebootNow = ++SEQ;
  draw();
  const until = Date.now() + 600000;
  while (rebootNow === run && el && Date.now() < until) {
    await sleep(5000);
    if (rebootNow !== run || !el) return;
    if (step !== 'srv_check') { rebootNow = 0; return; }
    try {
      const r = await call('server_precheck', { id: activeId(), mode: 'windows' });
      if (!r.problems.some(p => p.id === 'pending_reboot')) { pc = r; break; }
    } catch { /* the server is restarting */ }
  }
  if (rebootNow === run) { rebootNow = 0; draw(); }
}
/** After Terminal opened: try the key every 3 s (up to 3 min) and move on by
 *  ourselves once the password was typed there and the key is installed. */
async function waitForKey() {
  const run = ++waiting;
  draw();
  const until = Date.now() + 180000;
  while (waiting === run && el && Date.now() < until) {
    await new Promise(r => setTimeout(r, 3000));
    if (waiting !== run || !el || !activeId()) return;
    if (srvTest?.ok) break; // "Tekrar dene" got there first
    try {
      const r = await call('server_test', { id: activeId() });
      if (r.ok) { srvTest = r; break; }
    } catch { /* keep waiting */ }
  }
  if (waiting === run) { waiting = 0; draw(); }
}

/** After the sudo command opened in Terminal: check again every 5 s (up to
 *  15 min, apt can be slow) until nothing blocks any more. */
async function waitForFix() {
  const run = ++fixWaiting;
  draw();
  const until = Date.now() + 15 * 60000;
  while (fixWaiting === run && el && Date.now() < until) {
    await new Promise(r => setTimeout(r, 5000));
    if (fixWaiting !== run || !el || step !== 'srv_check') return;
    try {
      const r = await call('server_precheck', { id: activeId(), mode: srvMode || state.snap?.provider_kind || 'linux-native' });
      pc = r;
      if (!r.problems.some(p => p.blocking)) break;
      draw();
    } catch { /* keep waiting */ }
  }
  if (fixWaiting === run) { fixWaiting = 0; draw(); }
}

async function retest() {
  srvBusy = true; srvTest = null; draw();
  try { srvTest = await call('server_test', { id: activeId() }); } catch (e) { srvTest = { ok: false, kind: 'network', detail: String(e) }; }
  srvBusy = false; draw();
}
async function precheck() {
  checking = true; srvErr = null; draw();
  try { pc = await call('server_precheck', { id: activeId(), mode: srvMode || state.snap?.provider_kind || 'linux-native' }); } catch (e) { srvErr = String(e); pc = null; }
  checking = false; draw();
}

let lastKey = '';
export default {
  mount(node) {
    el = node; step = null; dt = null; lastKey = ''; comps = null; pickLang = null;
    probe = null; srvTest = null; pc = null; srvErr = null; srvBusy = false; checking = false; addedId = null;
    form = { name: '', ssh: '', key: '' };
    waiting = 0; fixWaiting = 0;
    autoOn = false; autoWaiting = 0; autoChecked = false; rebootDone = false; rebooting = 0; rebootNow = 0; instSeen = false; keyLine = null; autoHint = null;
    // "Sunucu ekle": ask which system the new server runs (Linux or Windows) before connecting
    if (addMode()) { osPick = 'server'; osSeen = false; srvMode = null; srvModeSeen = false; }
    draw();
    scan();
  },
  update(what) {
    if (!el) return;
    if (step === 'srv_conn' && !activeId() && !srvBusy) return; // typing in the form: no periodic redraw
    if (what === 'snap' || what === 'res' || what === 'job') {
      const s = state.snap;
      const key = [s?.lifecycle, s?.env_exists, s?.container?.status, s?.status?.phase, s?.status?.detail, state.job?.running, Math.floor(Date.now() / 10000)].join('|');
      if (key !== lastKey) { lastKey = key; draw(); }
    }
  },
  unmount() { el = null; dt = null; waiting = 0; fixWaiting = 0; autoWaiting = 0; rebooting = 0; rebootNow = 0; },
};
