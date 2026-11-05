// Development backend: realistic responses so the UI can be built and
// screenshotted in a plain browser (no Tauri, no Docker). Never used inside
// the app. Scenario via URL: ?mock=ready|login|booting|error|stopped|vmoff|none
// plus &native=1 for the Mac yerel mode, &lang=en, &fresh=1 (first run: no
// language chosen, wizard not finished). Wizard choices survive a reload in
// sessionStorage.

const scenario = new URLSearchParams(location.search).get('mock') || 'ready';
const native = new URLSearchParams(location.search).get('native') === '1';
const HOME = '/Users/demo/Library/Application Support/SiliconMT5';
const now = () => Date.now() / 1000;
const t0 = now() - 2 * 3600 - 14 * 60;
const listeners = {};
let job = { id: 0, kind: '', title: '', running: false, code: null, started: 0, finished: null, lines: [] };
let equity = 10012.25;

const phase = { ready: 'ready', login: 'waiting_login', booting: native ? 'starting_terminal' : 'installing_mt5', error: 'error' }[scenario] || null;
const lifecycle = phase || { stopped: 'stopped', vmoff: native ? 'not_installed' : 'engine_down', none: 'no_config' }[scenario] || 'no_config';
const installed = scenario !== 'none';
const q = new URLSearchParams(location.search);
const mem = (k, d) => { try { const v = sessionStorage.getItem('mock.' + k); return v == null ? d : JSON.parse(v); } catch { return d; } };
const keep = (k, v) => { try { sessionStorage.setItem('mock.' + k, JSON.stringify(v)); } catch { /* none */ } };
const fresh = q.get('fresh') === '1';
const app = () => ({ lang: mem('lang', q.get('lang') || 'tr'), lang_chosen: mem('lang_chosen', !fresh), setup_complete: mem('setup_complete', !fresh), env: mem('env', installed), prepared: mem('prepared', installed) });

// Job titles and output as the real backend / runtime scripts print them.
function jobTitle(kind) {
  const L = (tr, e) => (app().lang === 'en' ? e : tr);
  if (native && ['build', 'pull', 'install', 'prepare'].includes(kind)) return L('Yerel mod kuruluyor (Wine 11, Windows Python, MT5)', 'Installing the native mode (Wine 11, Windows Python, MT5)');
  return { build: L('İmaj derleniyor', 'Building the image'), prepare: L('Kurulum hazırlanıyor', 'Preparing the installation'), up: L('Runtime başlatılıyor', 'Starting the runtime'), start: L('Runtime başlatılıyor', 'Starting the runtime'),
    vm_start: L('x86_64 VM başlatılıyor', 'Starting the x86_64 VM'), server_install: L('Sunucuya kuruluyor', 'Installing on the server'), server_update: L('Sunucu güncelleniyor', 'Updating the server') }[kind] || kind;
}
function jobLines(kind) {
  const en = app().lang === 'en';
  const L = (tr, e) => (en ? e : tr);
  const ts = i => new Date(Date.now() + i * 900).toTimeString().slice(0, 8);
  if (native && ['build', 'pull', 'install', 'prepare'].includes(kind)) return [
    L('Wine 11.21 indiriliyor (Gcenx macOS derlemesi)', 'Downloading Wine 11.21 (Gcenx macOS build)'), L('Wine hazır: wine-11.21', 'Wine ready: wine-11.21'), L('Wine prefix oluşturuluyor', 'Creating the Wine prefix'),
    L('Windows Python 3.13 indiriliyor (NuGet paketi, kurulum programı yok)', 'Downloading Windows Python 3.13 (NuGet package, no installer)'), L('Windows Python paketleri kuruluyor (MetaTrader5, siliconmetatrader5)', 'Installing Windows Python packages (MetaTrader5, siliconmetatrader5)'),
    L('MetaTrader 5 kurulum programı çalıştırılıyor (birkaç dakika)', 'Running the MetaTrader 5 installer (a few minutes)'), L('MT5 kuruldu', 'MT5 installed'), L("Durum API'si için Python ortamı oluşturuluyor", 'Creating the Python environment for the status API')].map((l, i) => `[${ts(i)}] ${l}`);
  if (native && ['start', 'up'].includes(kind)) return [L('başlatıldı: MT5 penceresi birazdan açılır', 'started: the MT5 window opens shortly'), 'phase=starting_terminal', 'phase=ready'].map((l, i) => `[${ts(i)}] ${l}`);
  if (kind.startsWith('server_')) return ['$ scp smt5-runtime.tgz', '$ smt5 install', L('Python ortamı ve paketler kuruluyor', 'Installing the Python environment and packages'), L('MetaTrader 5 kuruluyor', 'Installing MetaTrader 5'), '$ smt5 start'];
  return ['$ docker compose up -d', ' Network siliconmt5_default  Created', ' Volume "siliconmt5_config"  Created', ' Container siliconmt5  Started'];
}

function components() {
  const a = app(), p = a.prepared, en = a.lang === 'en';
  const L = (tr, e) => (en ? e : tr);
  const c = (id, name, state, detail, fix = null) => ({ id, name, state, detail, fix });
  const envc = c('env', L('Yapılandırma', 'Configuration'), a.env ? 'ok' : 'app', a.env ? HOME + '/.env' : L('API token üretilecek', 'an API token is generated'));
  if (native) return [
    ...(q.get('os') === 'linux' ? [c('display', L('Ekran (noVNC)', 'Display (noVNC)'), 'user', L('ekran yok, eksik: x11vnc, websockify', 'no display, missing: x11vnc, websockify'), 'sudo apt install xvfb x11vnc websockify novnc openbox')] : [c('rosetta', 'Rosetta 2', 'ok', L('kurulu', 'installed'))]),
    c('python', 'Python 3.9+', mem('py', q.get('nopy') !== '1') ? 'ok' : 'user', L("Apple'ın komut satırı araçlarıyla gelir", "comes with Apple's command line tools"), mem('py', q.get('nopy') !== '1') ? null : 'xcode-select --install'),
    c('wine', 'Wine 11', p ? 'ok' : 'app', p ? L('indirildi', 'downloaded') : L('~215 MB indirilecek (Gcenx macOS derlemesi)', '~215 MB download (Gcenx macOS build)')),
    c('winpython', 'Windows Python + MetaTrader5', p ? 'ok' : 'app', p ? L('kurulu', 'installed') : L('Wine içine kurulacak', 'installed into Wine')),
    c('mt5', 'MetaTrader 5', p ? 'ok' : 'app', p ? L('kurulu', 'installed') : L('MetaQuotes kurulum programıyla kurulacak', 'installed with the MetaQuotes installer')),
    c('venv', L("Durum API'si", 'Status API'), p ? 'ok' : 'app', p ? L('hazır', 'ready') : L('küçük bir Python ortamı oluşturulacak', 'a small Python environment is created')),
    envc];
  return [
    c('packages', 'Docker, Colima, QEMU (Homebrew)', p ? 'ok' : 'app', p ? 'colima, docker, docker-compose, qemu, lima-additional-guestagents' : L('eksik: colima, qemu', 'missing: colima, qemu'), p ? null : 'brew install colima qemu'),
    c('vm', L('x86_64 sanal makine', 'x86_64 virtual machine'), p ? 'ok' : 'app', p ? L("'x86' çalışıyor", "'x86' running") : L('oluşturulacak: 4 CPU, 8 GB RAM, 60 GB disk (~5–10 dk)', 'created: 4 CPU, 8 GB RAM, 60 GB disk (~5–10 min)')),
    c('image', L('Runtime imajı', 'Runtime image'), p ? 'ok' : 'app', p ? L('yerelde mevcut', 'present') : L('indirilecek (GHCR)', 'pulled from GHCR')),
    envc];
}


function status(ph = phase) {
  equity += (Math.random() - 0.45) * 6;
  const ready = ph === 'ready';
  return {
    version: '1.3.0', display: 'novnc', phase: ph, detail: ph === 'installing_mt5' ? 'attempt 1/3' : ph === 'error' ? 'MT5 installer exited with code 1' : '',
    supervisor: { phase: ph, detail: '', updated: ph === 'ready' ? t0 : now() - 312 },
    mt5: { installed: ph !== 'installing_mt5', running: ['ready', 'waiting_login'].includes(ph), maxbars: 100000 },
    bridge: {
      port: 8001, listening: ['ready', 'waiting_login'].includes(ph), probe_fresh: true,
      probe: ready ? {
        ok: true, latency_ms: 30 + Math.random() * 25, checked_at: Math.floor(now() / 3) * 3, positions: 2, orders: 0,
        terminal: { connected: true, build: 6295, trade_allowed: false, maxbars: 100000, dlls_allowed: false },
        account: { login_masked: '•••234', server: 'DemoBroker-Server', company: 'Demo Broker Ltd', currency: 'USD', trade_mode: 'demo', leverage: 100, balance: 10000.5, equity, profit: equity - 10000.5, margin: 52.4, margin_free: equity - 52.4, margin_level: (equity / 52.4) * 100 },
      } : { ok: ph === 'waiting_login', latency_ms: 41, checked_at: now(), terminal: { connected: false, build: 6295 }, account: null, error: ph === 'waiting_login' ? null : 'bridge not listening' },
    },
    host: { cpu_translation: native ? 'rosetta2' : 'none' }, disk: native ? { path: HOME, free_mb: 212400, total_mb: 494400 } : { path: '/config', free_mb: 41200, total_mb: 59800 }, api: { mutations_enabled: true, level: q.get('oldrt') === '1' ? 0 : 3 }, time: now(),
  };
}

function nativeSnapshot() {
  return {
    ...baseSnapshot(), provider: 'macOS · Wine 11 + Rosetta 2 (no VM)', provider_kind: 'mac-native', docker_context: null, global_context: '',
    container: phase ? { status: 'running', health: null } : scenario === 'stopped' ? { status: 'exited', health: null } : null,
    status: phase ? { ...status(), display: 'native' } : null, manages_vm: false, mode: 'native', mode_label: 'Mac yerel · Wine 11 + Rosetta 2', engine_label: 'Mac',
    display: q.get('headless') === '1' ? 'novnc' : 'native', display_url: q.get('headless') === '1' ? 'about:blank' : '', image: q.get('os') === 'linux' ? 'Wine 11' : 'Wine 11 + Rosetta 2', native: true, native_home: HOME,
  };
}

function snapshot() {
  const sv = activeServer();
  if (sv) return serverSnapshot(sv);
  return native ? nativeSnapshot() : baseSnapshot();
}

// ---- Server mode: ?srv=nokey|missing|docker|ok|win|win-unknown|win-pending|win-nosession|mismatch (wizard) · ?servers=demo (Servers page)
const srvScenario = q.get('srv') || 'ok';
function serversList() {
  if (q.get('servers') === 'demo' && !mem('servers', null)) keep('servers', [
    { id: 'a1b2c3', name: 'VPS-1', ssh: 'ubuntu@203.0.113.10', mode: 'linux-native', port_base: 18000, host_key: 'SHA256:x', added_at: 0, demo: 'ready' },
    { id: 'd4e5f6', name: 'Frankfurt', ssh: 'ubuntu@fra1.example.net', mode: 'docker', port_base: 18100, host_key: 'SHA256:y', added_at: 0, demo: 'host_key_changed' },
    { id: 'g7h8i9', name: 'Yedek', ssh: 'yedek', mode: 'linux-native', port_base: 18200, host_key: 'SHA256:z', added_at: 0, demo: 'update' },
    { id: 'j0k1l2', name: 'Yeni VPS', ssh: 'root@203.0.113.9', mode: 'linux-native', port_base: 18300, host_key: 'SHA256:w', added_at: 0, demo: 'fresh' },
    { id: 'w1x2y3', name: 'Win VPS', ssh: 'Administrator@203.0.113.20', mode: 'windows', port_base: 18400, host_key: 'SHA256:v', added_at: 0, demo: 'ready' },
  ]);
  if (srvScenario === 'win-nosession' && !mem('servers', null)) { keep('servers', [{ id: 'w9z8y7', name: 'Win VPS', ssh: 'Administrator@203.0.113.20', mode: 'windows', port_base: 18400, host_key: 'SHA256:v', added_at: 0, demo: 'no_session' }]); keep('active', 'w9z8y7'); }
  return mem('servers', []);
}
const activeServer = () => serversList().find(x => x.id === mem('active', 'local')) || null;
function serverSnapshot(sv) {
  const b = sv.port_base, demo = sv.demo || (mem('srvInstalled', false) ? 'ready' : 'fresh');
  // ?link=connecting: every server is still opening its tunnel (app just started)
  const link = q.get('link') === 'connecting' ? { state: 'connecting' } : demo === 'host_key_changed' ? { state: 'failed', kind: 'host_key_changed', detail: 'host key changed', at: now() - 300 } : { state: 'connected', since: now() - 3600 };
  const up = demo === 'ready' || demo === 'update';
  if (sv.mode === 'windows') return windowsSnapshot(sv, demo, link);
  return {
    ...baseSnapshot(), provider: 'Sunucu · Linux yerel', provider_kind: sv.mode, docker_context: null, global_context: '', manages_vm: false,
    container: up && link.state === 'connected' ? { status: 'running', health: null } : null, status: up && link.state === 'connected' ? status('ready') : null,
    lifecycle: link.state === 'connecting' ? 'connecting' : link.state !== 'connected' ? 'unreachable' : up ? 'ready' : 'not_installed',
    display: 'novnc', display_port: b + 81, display_url: `http://127.0.0.1:${b + 81}/vnc.html?autoconnect=1&resize=scale#password=demo`, bridge_port: b + 1, status_port: b + 2,
    native: sv.mode === 'linux-native', target: sv.id, target_name: sv.name, link, update_needed: demo === 'update', linger: demo !== 'update',
    has_password: up, has_token: up, env_exists: up,
  };
}
// Windows server: no Wine/Rosetta; `no_session` while nobody is logged on.
// After the install the server has no session until the reboot (autologon).
function windowsSnapshot(sv, demo, link) {
  const b = sv.port_base;
  let up = demo === 'ready', life = 'ready';
  if (demo === 'no_session') { up = false; life = 'no_session'; }
  else if (sv.demo === 'fresh' || (!sv.demo && !mem('srvInstalled', false))) { up = false; life = 'not_installed'; }
  else if (!sv.demo) { // freshly installed through the wizard
    const r = mem('rebootAt', 0), age = r ? (Date.now() - r) / 1000 : -1;
    if (age < 0 && srvScenario === 'win-session') { up = true; life = 'waiting_login'; } // an RDP/console session is already open
    else if (age < 0) { up = false; life = 'no_session'; }
    // ?srv=win-slowlogon: down 0–6 s, then Windows is up but logs on only after 20 s (autologon stays on)
    else if (srvScenario === 'win-slowlogon') { if (age < 6) { up = false; life = 'unreachable'; } else if (age < 20) { up = false; life = 'no_session'; } else { up = true; life = 'waiting_login'; } }
    else if (age < 3) { up = false; life = 'unreachable'; } else { up = true; life = 'waiting_login'; }
  }
  const lk = life === 'unreachable' ? { state: 'failed', kind: 'network', detail: 'rebooting', at: now() } : link;
  return {
    ...baseSnapshot(), provider: 'Sunucu · Windows', provider_kind: 'windows', docker_context: null, global_context: '', manages_vm: false,
    container: up ? { status: 'running', health: null } : null, status: up ? { ...status(life === 'ready' ? 'ready' : 'waiting_login'), host: { cpu_translation: 'none' } } : null,
    lifecycle: link.state === 'connecting' ? 'connecting' : link.state !== 'connected' ? 'unreachable' : life, mode: 'native', mode_label: 'Sunucu · Windows', engine_label: 'Windows', image: 'MT5 (Windows)', native: true, native_home: null,
    display: 'novnc', display_port: b + 81, display_url: `http://127.0.0.1:${b + 81}/vnc.html?autoconnect=1&resize=scale#password=demo`, bridge_port: b + 1, status_port: b + 2,
    target: sv.id, target_name: sv.name, link: lk, update_needed: false, linger: true, has_password: life !== 'not_installed', has_token: life !== 'not_installed', env_exists: life !== 'not_installed',
  };
}
function serverCards() {
  const act = mem('active', 'local');
  const cards = serversList().map(sv => {
    const sn = serverSnapshot(sv), a = sn.status?.bridge?.probe?.account;
    return { id: sv.id, name: sv.name, ssh: sv.ssh, mode: sv.mode, port_base: sv.port_base, bridge_port: sv.port_base + 1, remote_bridge_port: 8001, link: sn.link, lifecycle: sn.lifecycle,
      phase: sn.status?.phase || null, account: a || null, balance: a?.balance ?? null, equity: a?.equity ?? null, currency: a?.currency ?? null,
      today_pl: a ? 12.5 : null, latency_ms: sn.link.state === 'connected' ? 38 : null, update_needed: sn.update_needed, linger: sn.linger, active: act === sv.id, error: null };
  });
  if (installed) cards.unshift({ id: 'local', name: app().lang === 'en' ? 'This computer' : 'Bu bilgisayar', ssh: '', mode: native ? 'mac-native' : 'mac-colima', port_base: 0, bridge_port: 8001,
    link: null, lifecycle, phase, account: null, balance: null, equity: null, currency: null, today_pl: null, latency_ms: null, update_needed: false, linger: false, active: act === 'local', error: null });
  return cards;
}
function winPrecheck() {
  // pending_reboot (?srv=win-pending) clears 3 s after the restart was requested
  const pending = srvScenario === 'win-pending' && !(mem('rebootAt', 0) && Date.now() - mem('rebootAt', 0) > 3000);
  const win = { build: 20348, arch: 'AMD64', os_caption: 'Windows Server 2022 Standard', admin: true, disk_free_mb: 80000, mem_mb: 8192, ps_major: 5, tar: true, network: true, pending_reboot: pending, logon_banner: false, foreign_mt5: false, installed: false, version: null };
  return { preflight: null, win, problems: pending ? [{ id: 'pending_reboot', blocking: false }] : [], fix: null };
}
function precheckResult(mode) {
  if (mode === 'windows') return winPrecheck();
  // the Terminal fix lands ~4 s after it was opened (sudo password, apt)
  const fixed = !!mem('fixedAt', 0) && Date.now() - mem('fixedAt', 0) > 4000;
  const tools = { Xvfb: true, x11vnc: true, websockify: true, novnc: true, python3_venv: true, curl: true, xz: true, docker: mode !== 'docker', compose: mode !== 'docker' };
  const pf = { arch: 'x86_64', os_id: 'ubuntu', os_pretty: 'Ubuntu 26.04.1 LTS', disk_free_mb: 191000, mem_mb: 61000, systemd_user: true, linger: true, apt: true, sudo_nopass: false, docker_access: false, installed: false, version: null, tools };
  let problems = [], fix = null;
  if (!fixed && srvScenario === 'missing') { pf.tools.Xvfb = false; pf.tools.x11vnc = false; pf.linger = false; problems = [{ id: 'packages', blocking: true }, { id: 'linger', blocking: true }]; fix = 'sudo apt-get update && sudo apt-get install -y xvfb openbox x11vnc && sudo loginctl enable-linger "$USER"'; }
  if (!fixed && mode === 'docker') { problems = [{ id: 'packages', blocking: true }]; fix = 'sudo apt-get update && sudo apt-get install -y docker.io docker-compose-v2 && sudo usermod -aG docker "$USER"'; }
  if (fixed && mode === 'docker') { pf.tools.docker = true; pf.tools.compose = true; pf.docker_access = true; }
  return { preflight: pf, problems, fix };
}

function baseSnapshot() {
  return {
    runtime_dir: '/Users/demo/Documents/GitHub/silicon-metatrader5/runtime', provider: 'macOS · Colima x86_64 VM (QEMU)', provider_kind: 'mac-colima',
    docker_context: 'colima-x86', global_context: 'colima',
    container: phase ? { status: 'running', health: phase === 'ready' ? 'healthy' : 'starting' } : null,
    status: phase ? status() : null, status_error: null, lifecycle, manages_vm: true, mode: 'safe', mode_label: 'Güvenli · Colima x86_64 QEMU VM', engine_label: 'VM', native: false, native_home: null,
    display: 'novnc', display_port: 6081, display_url: 'http://127.0.0.1:6081/vnc.html?autoconnect=1&resize=scale#password=demo', bridge_port: 8001, status_port: 8002,
    has_password: app().env, has_token: app().env, env_exists: app().env, image: 'siliconmt5:1.3.0-novnc',
  };
}

const years = (from, to, base) => Object.fromEntries(Array.from({ length: to - from + 1 }, (_, i) => [String(from + i), Math.round(base * (0.6 + Math.random() * 0.6))]));
function inventory() {
  const syms = [['EURUSD', 2016, 34e6, 41e6], ['XAUUSD', 2018, 36e6, 22e6], ['GBPUSD', 2016, 31e6, 12e6], ['USDJPY', 2017, 30e6, 8e6], ['US500', 2020, 24e6, 0], ['BTCUSD', 2021, 28e6, 0]];
  const history = syms.map(([s, y, b]) => { const ys = years(y, 2026, b); const total = Object.values(ys).reduce((a, x) => a + x, 0); return { symbol: s, bytes: total + 4e6, years: ys, cache_bytes: 4e6, other_bytes: 0 }; });
  const ticks = syms.filter(x => x[3]).map(([s, , , t]) => ({ symbol: s, bytes: t, months: { 202608: t * 0.3, 202609: t * 0.3, 202610: t * 0.4 }, other_bytes: 2e6 }));
  const hb = history.reduce((a, h) => a + h.bytes, 0), tb = ticks.reduce((a, t) => a + t.bytes, 0);
  return {
    root: '/config/.wine/drive_c/Program Files/MetaTrader 5', exists: true, total_bytes: hb + tb + 11e6 + 3e6, bases_bytes: hb + tb + 11e6, tester_bytes: 3e6, scanned_at: now(),
    servers: [
      { name: 'DemoBroker-Server', bytes: hb + tb, history_bytes: hb, ticks_bytes: tb, cache_bytes: 24e6, history, ticks },
      { name: 'Default', bytes: 11e6, history_bytes: 11e6, ticks_bytes: 0, cache_bytes: 1e6, history: [{ symbol: 'EURUSD', bytes: 11e6, years: { 2025: 5e6, 2026: 5e6 }, cache_bytes: 1e6, other_bytes: 0 }], ticks: [] },
    ],
  };
}

function datatest(symbol, tf) {
  const bars = []; let p = 1.0884;
  for (let i = 0; i < 300; i++) {
    const o = p, c = o + (Math.random() - 0.48) * 0.0006, h = Math.max(o, c) + Math.random() * 0.0003, l = Math.min(o, c) - Math.random() * 0.0003;
    const d = new Date(Date.UTC(2026, 10, 3, 13, 15) - (299 - i) * 300000);
    bars.push({ t: d.toISOString().replace('T', ' ').slice(0, 19), o, h, l, c, v: 100 + Math.round(Math.random() * 400) }); p = c;
  }
  const ck = (id, title, detail, ms) => ({ id, title, ok: true, detail, ms });
  return {
    symbol, timeframe: tf, verdict: 'ok', message: '3 yöntemin üçü de bar döndürdü · son bar 0.8 dk önce', server_offset: 'UTC+2', server_offset_s: 7200, server_now: '2026-11-03 13:16:04',
    checks: [ck('initialize', 'Köprü bağlantısı', 'initialize() · 127.0.0.1:8001', 420), ck('account', 'Hesap ve terminal', 'account_info() · terminal_info() · demo · connected', 310), ck('symbol', 'Sembol seçimi', `symbol_select("${symbol}") · in Market Watch`, 80), ck('clock', 'Broker saati', 'last tick → UTC+2', 60), ck('from_pos', 'Canlı barlar', 'copy_rates_from_pos(…, 0, 300) · 300 bars · last 13:15', 960), ck('from', 'Tarihten geriye', 'copy_rates_from(…, now, 300) · 300 bars · Δ 0 min', 880), ck('range', 'Aralık', 'copy_rates_range(…, −6 h, now) · 72 bars', 1120), ck('ticks', 'Canlı tick', 'symbol_info_tick() · 5 s listening · 6 ticks', 5000)],
    methods: { copy_rates_from_pos: { rows: 300, fresh_min: 0.8 }, copy_rates_from: { rows: 300, fresh_min: 0.8 }, copy_rates_range: { rows: 72, fresh_min: 0.8 } },
    bars, ticks: { count: 6, seconds: 5, digits: 5, bid: 1.08884, ask: 1.08896, spread_points: 12, samples: [1, 2, 3, 4, 5, 6].map(i => ({ t: `2026-11-03 13:16:0${i}`, bid: 1.0888 + i * 1e-5, ask: 1.0889 + i * 1e-5 })) },
  };
}

const LOG = `2026-11-03 11:02:14 [supervisor] seeding persistent prefix from baked runtime
2026-11-03 11:02:31 [supervisor] phase=preparing_prefix (copying prefix)
2026-11-03 11:03:02 [installer] installing MetaTrader 5 (attempt 1/3)
2026-11-03 11:09:47 [installer] MT5 installation detected
2026-11-03 11:09:48 [mt5] mt5cfg.ini synchronized (MaxBars=100000)
2026-11-03 11:09:49 [supervisor] MT5 pid=412
2026-11-03 11:10:04 [supervisor] bridge pid=488
2026-11-03 11:10:09 [bridge] listening on 127.0.0.1:8001
2026-11-03 11:14:22 [supervisor] phase=waiting_login
2026-11-03 11:21:40 [supervisor] phase=ready
2026-11-03 12:47:03 [supervisor] warning: bridge process missing, restarting
2026-11-03 12:47:05 [bridge] probe timeout (2.0s) · reconnect attempt 1/5
2026-11-03 12:47:14 [bridge] reconnected · latency 44 ms
2026-11-03 13:16:58 [status] heartbeat ok · terminal_info.connected=true`.split('\n');

const NATIVE_DOCTOR = [
  { id: 'host', title: 'Host', level: 'info', detail: 'macos aarch64 — macOS · Wine 11 + Rosetta 2 (no VM)' },
  { id: 'rosetta', title: 'Rosetta 2', level: 'ok', detail: 'installed' },
  { id: 'python', title: 'Python (status API)', level: 'ok', detail: `venv in ${HOME}/venv` },
  { id: 'wine', title: 'Wine', level: scenario === 'vmoff' ? 'warn' : 'ok', detail: scenario === 'vmoff' ? 'not downloaded yet (Wine 11, ~215 MB)' : 'wine-11.21', fix: scenario === 'vmoff' ? 'smt5 build' : null },
  { id: 'install', title: 'MT5 + Windows Python', level: scenario === 'vmoff' ? 'warn' : 'ok', detail: scenario === 'vmoff' ? 'missing: MetaTrader 5, Windows Python · MT5 is copied from the Güvenli mode container if it exists (login + history), else installed' : `${HOME}/prefix/drive_c/Program Files/MetaTrader 5`, fix: scenario === 'vmoff' ? 'smt5 build' : null },
  { id: 'env', title: 'Configuration (.env)', level: 'ok', detail: 'API token set' },
  { id: 'agent', title: 'launchd agent', level: phase ? 'ok' : 'info', detail: phase ? 'com.siliconmt5.runtime loaded' : 'stopped', fix: phase ? null : 'smt5 up' },
];

const DOCTOR = [
  { id: 'host', title: 'Host', level: 'info', detail: 'macos aarch64 — macOS · Colima x86_64 VM (QEMU)' },
  { id: 'docker-cli', title: 'Docker CLI', level: 'ok', detail: 'docker 29.7.1' },
  { id: 'colima', title: 'Colima', level: 'ok', detail: 'colima 0.9.1' },
  { id: 'vm', title: 'x86_64 VM', level: phase ? 'ok' : 'warn', detail: phase ? "'x86' running · 4 CPU · 8 GiB RAM · 60 GiB disk" : "profile 'x86' does not exist", fix: phase ? null : 'smt5 vm start' },
  { id: 'engine', title: 'Docker engine', level: phase ? 'ok' : 'fail', detail: phase ? "context 'colima-x86' · x86_64" : "context 'colima-x86' is not reachable", fix: phase ? null : 'smt5 vm start' },
  { id: 'global-context', title: 'Global docker context', level: 'info', detail: "'colima' (other projects) — SiliconMetaTrader5 pins 'colima-x86'" },
  { id: 'env', title: 'Configuration (.env)', level: phase ? 'ok' : 'warn', detail: phase ? 'password and API token set' : 'runtime/.env missing', fix: phase ? null : 'smt5 init' },
];

// Supertrend for the mock (same rules as runtime/status/supertrend_strategy.py)
function mockSupertrend(args) {
  const tfs = { M1: 60, M5: 300, M15: 900, M30: 1800, H1: 3600, H4: 14400, D1: 86400 }, step = tfs[args.tf] || 3600;
  const n = Math.min(args.bars || 3000, 3000), period = args.period || 10, mult = args.multiplier || 3;
  let seed = 7; const rnd = () => { seed = (seed * 16807) % 2147483647; return seed / 2147483647 - 0.5; };
  const end = Math.floor(Date.UTC(2026, 10, 4, 14) / 1000 / step) * step, bars = [];
  let price = 1.085;
  for (let i = 0; i <= n; i++) {
    const o = price, c = o + 0.0006 * Math.sin(i / 35) + rnd() * 0.0024, h = Math.max(o, c) + Math.abs(rnd()) * 0.0012, l = Math.min(o, c) - Math.abs(rnd()) * 0.0012;
    bars.push([end - (n - i) * step, o, h, l, c]); price = c;
  }
  const forming = bars.pop()[0];
  let atr = null, trs = 0, up = null, lo = null, dir = 0, pc = null, val = null, pending = null, pos = null;
  const line = [], trend = [], signals = [], trades = [];
  bars.forEach((b, i) => {
    if (pending) { if (pos) { pos.exit_time = b[0]; pos.exit_price = b[1]; pos.pnl = pos.side === 'BUY' ? b[1] - pos.entry_price : pos.entry_price - b[1]; trades.push(pos); } pos = { side: pending.side, entry_time: b[0], entry_price: b[1], exit_time: null, exit_price: null, pnl: 0, open: false }; pending = null; }
    const tr = pc == null ? b[2] - b[3] : Math.max(b[2] - b[3], Math.abs(b[2] - pc), Math.abs(b[3] - pc));
    if (atr == null) { trs += tr; if (i + 1 >= period) atr = trs / period; } else atr = (atr * (period - 1) + tr) / period;
    if (atr != null) {
      const mid = (b[2] + b[3]) / 2; let u = mid + mult * atr, l = mid - mult * atr;
      if (up != null) { l = (l > lo || pc < lo) ? l : lo; u = (u < up || pc > up) ? u : up; }
      const before = dir;
      dir = dir === 0 ? -1 : dir === -1 ? (b[4] > u ? 1 : -1) : (b[4] < l ? -1 : 1);
      up = u; lo = l; val = dir === 1 ? l : u;
      if (before !== 0 && before !== dir) { const sg = { time: b[0], side: dir === 1 ? 'BUY' : 'SELL', price: b[4], line: val }; signals.push(sg); pending = sg; }
    }
    pc = b[4]; line.push(atr == null ? null : val); trend.push(atr == null ? 0 : dir);
  });
  const last = bars[bars.length - 1];
  if (pos) { pos.exit_time = last[0]; pos.exit_price = last[4]; pos.open = true; pos.pnl = pos.side === 'BUY' ? last[4] - pos.entry_price : pos.entry_price - last[4]; trades.push(pos); }
  const closed = trades.filter(x => !x.open), wins = closed.filter(x => x.pnl > 0);
  let eq = 0, peak = 0, dd = 0; const curve = closed.map(x => { eq += x.pnl; peak = Math.max(peak, eq); dd = Math.max(dd, peak - eq); return eq; });
  const gl = -closed.filter(x => x.pnl <= 0).reduce((a, x) => a + x.pnl, 0), gp = wins.reduce((a, x) => a + x.pnl, 0);
  const T = 300;
  return { symbol: args.symbol, timeframe: args.tf, digits: 5, params: { period, multiplier: mult }, count: bars.length, from: bars[0][0], last_closed: last[0], forming,
    trend: dir, line_now: val, chart: { bars: bars.slice(-T), line: line.slice(-T), trend: trend.slice(-T) }, signals: signals.slice(-300), trades: trades.slice(-300),
    stats: { trades: closed.length, wins: wins.length, win_rate: closed.length ? wins.length / closed.length : null, net: gp - gl, gross_profit: gp, gross_loss: gl,
      profit_factor: gl > 0 ? gp / gl : null, avg_trade: closed.length ? (gp - gl) / closed.length : null, max_drawdown: dd, equity: curve.slice(-500) } };
}

export async function call(cmd, args) {
  await new Promise(r => setTimeout(r, 60));
  switch (cmd) {
    case 'servers': return serverCards();
    case 'server_probe': await new Promise(r => setTimeout(r, 300)); if (srvScenario === 'win-nossh') { const h = args.ssh.split('@').pop(); throw app().lang === 'en' ? `could not reach ${h}:22 or read its SSH host key` : `${h}:22 adresine ulaşılamadı ya da SSH anahtarı alınamadı`; } return { dest: args.ssh, host: args.ssh.split('@').pop().split(':')[0], port: 22, user: args.ssh.includes('@') ? args.ssh.split('@')[0] : null, os: srvScenario === 'win-unknown' ? null : srvScenario.startsWith('win') ? 'windows' : 'linux', keys: [{ kind: 'ED25519', sha256: 'SHA256:Xq3mVJ4m0b6dXy0Qk3l1cR8tT0z7nYb9W2aEo5p6uHs' }, { kind: 'RSA', sha256: 'SHA256:8vV1nZt0bq1C0g5xWlQm2y7v0Jt3uLr6sK9dE4fA2hQ' }] };
    case 'server_add': { const list = serversList(); if (list.some(x => x.ssh === args.ssh)) throw new Error(args.ssh + ' zaten ekli'); const sv = { id: 's' + (list.length + 1) + 'a2b3', name: args.name, ssh: args.ssh, identity: args.identity, mode: 'linux-native', port_base: 18000 + 100 * list.length, host_key: args.sha256, added_at: now() }; keep('servers', [...list, sv]); return sv; }
    case 'set_active_target': keep('active', args.id); return null;
    // nokey: the key works ~4 s after Terminal opened (the user is still typing the password)
    case 'server_test': {
      await new Promise(r => setTimeout(r, 300));
      const keyed = mem('keyedAt', 0) && Date.now() - mem('keyedAt', 0) > 4000;
      if ((srvScenario === 'nokey' || srvScenario === 'win-unknown') && !keyed) return { ok: false, kind: 'key_required', detail: 'key_required', os: null, os_mismatch: false };
      const sv = serversList().find(x => x.id === args.id);
      const os = srvScenario.startsWith('win') ? 'windows' : srvScenario === 'mismatch' ? 'windows' : 'linux';
      return { ok: true, kind: '', detail: '', os, os_mismatch: !!sv && (os === 'windows') !== (sv.mode === 'windows') };
    }
    case 'server_keygen': { keep('keyedAt', Date.now()); const sv = serversList().find(x => x.id === args.id); const pub = '/Users/demo/.ssh/siliconmt5_ed25519.pub';
      return { pub_path: pub, line: sv?.mode === 'windows' ? `ssh -t ${sv.ssh} powershell -NoProfile -EncodedCommand JABrAGUAeQA9ACcAcwBzAGgALQBlAGQAMgA1ADUAMQA5ACAAQQBBAEEAQQAuAC4ALgAnADsA # administrators_authorized_keys` : `ssh-copy-id -i ${pub} ${sv?.ssh || ''}` }; }
    case 'server_autologon': keep('autologonAt', Date.now()); return null;
    case 'server_autologon_status': return !!mem('autologonAt', 0) && Date.now() - mem('autologonAt', 0) > 6000;
    case 'server_reboot': keep('rebootAt', Date.now()); return null;
    case 'server_set_mode': keep('servers', serversList().map(x => x.id === args.id ? { ...x, mode: args.mode } : x)); return null;
    case 'server_precheck': await new Promise(r => setTimeout(r, 300)); return precheckResult(args.mode);
    case 'server_fix': { const fix = precheckResult(args.mode).fix; keep('fixedAt', Date.now()); return fix; }
    case 'server_rekey': case 'server_set_ports': return null;
    case 'server_forget': keep('servers', serversList().filter(x => x.id !== args.id)); if (mem('active', 'local') === args.id) keep('active', 'local'); return null;
    case 'app_info': { const a = app(); return { active_target: activeServer() ? mem('active', 'local') : 'local', servers: serversList().length, targets: serversList().map(x => [x.id, x.name]), version: '1.3.0', os: q.get('os') === 'linux' ? 'linux' : 'macos', arch: q.get('os') === 'linux' ? 'x86_64' : 'aarch64', runtime_dir: '/Users/demo/Documents/GitHub/silicon-metatrader5/runtime', runtime_error: null, packaged: q.get('packaged') === '1', lang: a.lang, lang_chosen: a.lang_chosen, setup_complete: a.setup_complete, tray: true, close_action: mem('close_action', null), api_level: 3 }; }
    case 'accounts': { const cur = scenario === 'ready' ? { login: 5123234, server: 'DemoBroker-Server', name: 'Demo User', company: 'Demo Broker Ltd', currency: 'USD', trade_mode: 'demo', last_seen: now() } : null;
      const known = mem('known', [{ login: 5123234, server: 'DemoBroker-Server', name: 'Demo User', company: 'Demo Broker Ltd', currency: 'USD', trade_mode: 'demo', last_seen: now() }, { login: 80044121, server: 'ExampleFX-Live', name: 'Demo User', company: 'Example FX Ltd', currency: 'USD', trade_mode: 'real', last_seen: now() - 86400 * 3 }]);
      return { current: cur, connected: !!cur, known, error: null }; }
    case 'account_history': {
      const D = 86400, end = Math.floor(Date.UTC(2026, 10, 4, 15) / 1000), days = { today: 1, '7d': 7, '30d': 30, '90d': 90, all: 120 }[args.range] || 30;
      let seed = 3; const rnd = () => { seed = (seed * 16807) % 2147483647; return seed / 2147483647; };
      const all = [[end - 100 * D, 2, 25000]];
      for (let i = 0; i < 140; i++) all.push([end - 98 * D + i * 0.7 * D, 1, Math.round((rnd() - 0.43) * 380 * 100) / 100]);
      all.push([end - 20 * D, 2, -1000]);
      all.sort((x, y) => x[0] - y[0]);
      const since = args.range === 'all' ? 0 : end - days * D;
      const bal = all.reduce((a, d) => a + d[2], 0);
      const inR = all.filter(d => d[0] >= since);
      const start = bal - inR.reduce((a, d) => a + d[2], 0);
      let run = start; const curve = [[since || inR[0][0], start, -1], ...inR.map(d => { run += d[2]; return [d[0], Math.round(run * 100) / 100, d[1]]; })];
      const tr = inR.filter(d => d[1] === 1);
      return { account: { login: 5123234, server: 'DemoBroker-Server', currency: 'USD' }, range: args.range, from: since, to: end, start_balance: start, balance: bal, equity: bal + 37.4, floating: 37.4,
        trading_pl: Math.round(tr.reduce((a, d) => a + d[2], 0) * 100) / 100, deposits: inR.filter(d => d[1] === 2 && d[2] > 0).reduce((a, d) => a + d[2], 0), withdrawals: inR.filter(d => d[1] === 2 && d[2] < 0).reduce((a, d) => a + d[2], 0),
        other: 0, trades: tr.length, wins: tr.filter(d => d[2] > 0).length, losses: tr.filter(d => d[2] < 0).length, deals: inR.length, curve, by_symbol: [] };
    }
    case 'account_forget': { const k = mem('known', []).filter(a => !(a.login === args.login && a.server === args.server)); keep('known', k); return k; }
    case 'set_close_action': keep('close_action', args.action); return null;
    case 'hide_main': case 'show_main_window': case 'tray_action': return null;
    case 'set_lang': keep('lang', args.lang); keep('lang_chosen', true); return null;
    case 'set_setup_complete': keep('setup_complete', args.done); return null;
    case 'components': return components();
    case 'snapshot': return args?.target === 'local' ? (native ? nativeSnapshot() : baseSnapshot()) : snapshot();
    case 'resources': if (activeServer()) { const sn = serverSnapshot(activeServer()); return { container: sn.container?.status === 'running' ? { cpu_percent: 23.5, mem_used_bytes: 1.4 * 2 ** 30, mem_limit_bytes: 8 * 2 ** 30, mem_percent: 17.5 } : null, vm: null }; }
      if (native) return { container: phase ? { cpu_percent: 14.2, mem_used_bytes: 1.1 * 2 ** 30, mem_limit_bytes: 16 * 2 ** 30, mem_percent: 6.9 } : null, vm: null };
      return { container: phase ? { cpu_percent: 118.4, mem_used_bytes: 2.9 * 2 ** 30, mem_limit_bytes: 7.7 * 2 ** 30, mem_percent: 37.6 } : null, vm: { name: 'x86', status: scenario === 'vmoff' || scenario === 'none' ? 'Stopped' : 'Running', arch: 'x86_64', cpus: 4, memory: 8 * 2 ** 30, disk: 60 * 2 ** 30 } };
    case 'doctor': return native ? NATIVE_DOCTOR : DOCTOR;
    case 'logs': return { sizes: { supervisor: 2.1e6, mt5: 9.6e6, bridge: 0.4e6, installer: 0.1e6, display: 1.2e6, status: 3.3e6 }, source: args.source, lines: args.source === 'supervisor' ? LOG : LOG.filter(l => l.includes(`[${args.source}]`)) };
    case 'inventory': return inventory();
    case 'purge': return args.spec.dry_run ? { dry_run: true, files: 7, bytes: 212e6, sample: ['Bases/DemoBroker-Server/history/EURUSD/2016.hcc', 'Bases/DemoBroker-Server/history/EURUSD/2017.hcc'] } : { dry_run: false, deleted: true, removed: 7, freed_bytes: 212e6, failed: [] };
    case 'datatest': await new Promise(r => setTimeout(r, 900)); return datatest(args.symbol, args.tf);
    case 'restart': return { accepted: true, target: args.target };
    case 'settings': return { values: { SMT5_ALGO_TRADING: '1', SMT5_DLL_IMPORT: '0', SMT5_LOG_MAX_MB: '10', SMT5_DISPLAY_PORT: '6081', SMT5_BRIDGE_PORT: '8001', SMT5_STATUS_PORT: '8002', SMT5_MAXBARS: '100000', SMT5_RESOLUTION: '1366x768', TZ: 'Europe/Istanbul' }, has_password: true, has_token: true, env_path: '/Users/demo/Documents/GitHub/silicon-metatrader5/runtime/.env', env_exists: true };
    case 'save_settings': return Object.keys(args.values);
    case 'regenerate': return { outcome: 'updated', path: '.env', generated: [], password: args.which === 'password' ? '5f1c9a0b7e2d4c6a88' : null };
    case 'reveal_password': return '029dd934974d663f5c';
    case 'init_env': keep('env', true); return { outcome: 'created', path: '.env', generated: ['SMT5_WEB_PASSWORD', 'SMT5_API_TOKEN'], password: '5f1c9a0b7e2d4c6a88' };
    case 'image_present': return native ? scenario !== 'vmoff' && installed : installed;
    case 'focus_mt5': return null;
    case 'app_copy_status': return { brew: '/opt/homebrew/Cellar/siliconmt5/1.4.0/SiliconMetaTrader5.app', brew_version: '1.4.0', copy: '/Applications/SiliconMetaTrader5.app', copy_version: new URLSearchParams(location.search).get('copy') || null, copy_is_link: !new URLSearchParams(location.search).get('copy'), needs_install: true };
    case 'app_copy_install': return '/Applications/SiliconMetaTrader5.app';
    case 'market_watch': return { symbols: ['EURUSD', 'GBPUSD', 'USDJPY', 'XAUUSD', 'US500.cash', 'BTCUSD', 'GER40.cash'], total: 812 };
    case 'job_state': return job;
    case 'start_job': {
      (window.__mockJobs ||= []).push({ kind: args.kind, opts: args.opts });
      job = { id: job.id + 1, kind: args.kind, title: jobTitle(args.kind), running: true, code: null, started: now(), finished: null, lines: [] };
      const id = job.id;
      const lines = jobLines(args.kind);
      lines.forEach((l, i) => setTimeout(() => { job.lines.push(l); fire('job-line', { id, line: l }); }, 300 * (i + 1)));
      const jobSrv = serversList().find(x => x.id === args.opts?.target) || activeServer();
      const unreachable = jobSrv?.demo === 'host_key_changed' && args.kind.startsWith('server_');
      // ?srv=win-installfail: smt5.ps1 install exits 1 (the server keeps VERSION + .env, no state\installed)
      const failed = srvScenario === 'win-installfail' && job.kind === 'server_install';
      window.__mockJobDone = false;
      setTimeout(() => { job.running = false; job.code = unreachable ? 255 : failed ? 1 : 0; job.finished = now(); job.lines.push(failed ? (app().lang === 'en' ? '✗ Exit code 1' : '✗ Çıkış kodu 1') : (app().lang === 'en' ? '✓ Done' : '✓ Tamamlandı')); if (['prepare', 'install', 'build'].includes(job.kind)) keep('prepared', true); if (job.kind === 'server_install' && !failed) keep('srvInstalled', true); if (job.kind === 'server_uninstall' && job.code === 0 && jobSrv) { keep('servers', serversList().filter(x => x.id !== jobSrv.id)); if (mem('active', 'local') === jobSrv.id) keep('active', 'local'); } fire('job-done', { ...job }); window.__mockJobDone = true; }, 300 * (lines.length + 2));
      return id;
    }
    case 'cancel_job': return true;
    case 'clipboard_push': (window.__mockClipPush = (window.__mockClipPush || 0) + 1); return null;
    case 'clipboard_sync': if (args.enabled) setTimeout(() => fire('clipboard', { to: 'mt5', chars: 42 }), 800); return null;
    case 'clear_logs': return { cleared: true, freed_bytes: 18e6 };
    case 'quit_app': return null;
    case 'bench': await new Promise(r => setTimeout(r, 700)); return native
      ? { ok: true, error: null, symbol: args.symbol, cpu_s: 0.14, rates_rows: 25000, rates_s: 0.06, transfer_rows: 10000, transfer_s: 0.03, tick_ms: 0.8, boot_s: 20.3, wine_flavor: 'native', wine_version: 'wine-11.21', cpu_translation: 'rosetta2', at: Date.now() / 1000 }
      : { ok: true, error: null, symbol: args.symbol, cpu_s: 1.89, rates_rows: 0, rates_s: 0.9, transfer_rows: 10000, transfer_s: 1.1, tick_ms: 10.96, boot_s: 55.2, wine_flavor: 'winehq', wine_version: '10.0', cpu_translation: 'none', at: Date.now() / 1000 };
    case 'modes': return [
      { id: 'safe', label: q.get('os') === 'linux' ? 'Docker · Wine 11 container' : 'Güvenli · Colima x86_64 QEMU VM', current: !native, available: true, hint: null },
      { id: 'native', label: q.get('os') === 'linux' ? 'Linux yerel · Wine 11' : 'Mac yerel · Wine 11 + Rosetta 2', current: native, available: true, hint: null }];
    case 'set_mode': keep('env', true); return ['SMT5_PROVIDER'];
    case 'removal_info': return {
      manages_vm: true, profile: { name: 'x86', status: 'Running', arch: 'x86_64', cpus: 4, memory: 8 * 2 ** 30, disk: 60 * 2 ** 30 },
      other_profiles: ['default'], other_containers: [], engine_up: !!phase, volumes: phase ? ['siliconmt5_siliconmt5_config', 'siliconmt5_siliconmt5_logs'] : [],
      images: ['ghcr.io/bahadirumutiscimen/siliconmt5:1.3.0-novnc (9.8GB)'], env_path: '/Users/demo/Documents/GitHub/silicon-metatrader5/runtime/.env', env_exists: true,
      config_path: '/Users/demo/Library/Application Support/com.siliconmt5.desktop/config.json', brew_formulae: ['colima', 'lima', 'qemu', 'docker', 'docker-compose'],
      packages_allowed: false, packages_reason: "Başka Colima profilleri var (default): colima ve docker başka projelerce kullanılıyor", docker_desktop: false, colima_delete_data: true,
      native, native_home: HOME, native_exists: native, native_bytes: native ? 3.4 * 2 ** 30 : null,
      manual_steps: ['SiliconMetaTrader5 uygulamasını Uygulamalar klasöründen Çöp Sepeti\'ne taşıyın', 'Repo klasörünü isterseniz silin: /Users/demo/Documents/GitHub/silicon-metatrader5'] };
    case 'strategy_supertrend': await new Promise(r => setTimeout(r, 400));
      if (q.get('cold') === '1' && (window.__cold = (window.__cold || 0) + 1) < 3) throw new Error('status API error 502: RuntimeError: no bars for ' + args.symbol + ' ' + args.tf + ' yet (history still downloading?)');
      return mockSupertrend(args);
    case 'example_source': { const r = await fetch(new URL('../../../../runtime/status/' + args.name, location.href)); if (!r.ok) throw new Error('mock: ' + r.status); return r.text(); }
    case 'open_display_window': case 'open_external': case 'set_runtime_dir': return null;
    default: throw new Error('mock: unknown command ' + cmd);
  }
}

function fire(ev, payload) { (listeners[ev] || []).forEach(fn => fn(payload)); }
window.__mockFire = fire;
export async function on(ev, fn) { (listeners[ev] ||= []).push(fn); return () => {}; }
