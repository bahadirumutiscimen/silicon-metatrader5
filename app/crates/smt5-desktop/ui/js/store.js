// Shared state: runtime snapshot (every 3 s), resources (every 10 s), the
// current long job, and session samples for the small charts.

import { call, on } from './api.js';
import { t } from './i18n.js';

const subs = new Set();
export const state = {
  info: null,          // app_info
  snap: null,          // snapshot
  snapError: null,
  res: null,           // resources
  job: null,           // job_state
  samples: [],         // [{t, equity, balance, latency}] of samplesAccount
  samplesAccount: '',
  logWarnings: 0,
};

export function subscribe(fn) { subs.add(fn); return () => subs.delete(fn); }
function emit(what) { subs.forEach(fn => { try { fn(what, state); } catch (e) { console.error(e); } }); }

/** Runtime state for labels and buttons, from the backend's `lifecycle`.
 *  group: setup (not installed yet) · off (installed, not running) · busy · on · error */
export function phaseOf(snap = state.snap) {
  if (!snap) return { key: 'unknown', label: t('Bilinmiyor'), tone: 'neu', group: 'off' };
  const vm = snap.manages_vm;
  const map = {
    engine_down: [t('{engine} kapalı', { engine: snap.engine_label || (vm ? 'VM' : 'Docker') }), 'neu', 'off'],
    no_config: ['Kurulum tamamlanmadı', 'neu', 'setup'],
    no_image: ['İmaj yok', 'neu', 'setup'],
    not_installed: ['Kurulmadı', 'neu', 'setup'],
    not_created: ['Hiç başlatılmadı', 'neu', 'setup'],
    stopped: ['Durduruldu', 'neu', 'off'],
    booting: ['Açılıyor', 'acc', 'busy'],
    preparing_prefix: ['Wine hazırlanıyor', 'acc', 'busy'],
    installing_mt5: ['MT5 kuruluyor', 'acc', 'busy'],
    starting_terminal: ['MT5 açılıyor', 'acc', 'busy'],
    starting_bridge: ['Köprü açılıyor', 'acc', 'busy'],
    waiting_login: ['Broker girişi bekleniyor', 'warn', 'on'],
    ready: ['Hazır', 'up', 'on'],
    maintenance: ['Bakım', 'warn', 'busy'],
    stopping: ['Duruyor', 'neu', 'busy'],
    error: ['Hata', 'down', 'error'],
    // Server mode
    connecting: ['Bağlanıyor', 'acc', 'busy'],
    unreachable: ['Sunucuya ulaşılamıyor', 'down', 'error'],
    installing: ['Kuruluyor', 'acc', 'busy'],
    no_session: ['Windows oturumu yok', 'warn', 'off'],
  };
  const key = snap.lifecycle || 'unknown';
  const [label, tone, group] = map[key] || [key, 'neu', 'busy'];
  return { key, label: t(label), tone, group };
}

/** Is the container up (status API may still be starting)? */
/** Native mode with MT5 in a desktop window (Mac, Linux desktop); headless Linux uses noVNC. */
/** The active server's name for breadcrumbs; null = this computer. */
export const targetName = (snap = state.snap) => (snap?.target && snap.target !== 'local' ? snap.target_name || snap.target : null);
export const nativeWindow = (snap = state.snap) => !!snap?.native && snap.display !== 'novnc';
export const isRunning = (snap = state.snap) => snap?.container?.status === 'running';

export const account = (snap = state.snap) => snap?.status?.bridge?.probe?.account || null;
export const probe = (snap = state.snap) => snap?.status?.bridge?.probe || null;

let snapBusy = false;
export async function refreshSnapshot() {
  if (snapBusy || !state.info?.runtime_dir) return;
  snapBusy = true;
  try {
    state.snap = await call('snapshot');
    state.snapError = null;
    const p = probe();
    const a = account();
    // samples belong to one account: start over when MT5 switches accounts
    const acct = a ? `${a.server}|${a.login_masked}` : '';
    if (acct && acct !== state.samplesAccount) { state.samples = []; state.samplesAccount = acct; }
    if (p?.ok && a?.equity != null) {
      const last = state.samples[state.samples.length - 1];
      const t = p.checked_at || Date.now() / 1000;
      if (!last || last.t !== t) {
        state.samples.push({ t, equity: a.equity, balance: a.balance, latency: p.latency_ms });
        if (state.samples.length > 720) state.samples.shift();
      }
    }
  } catch (e) {
    state.snapError = String(e);
  } finally {
    snapBusy = false;
    emit('snap');
  }
}

export async function refreshResources() {
  if (!state.info?.runtime_dir) return;
  try { state.res = await call('resources'); } catch { state.res = null; }
  emit('res');
}

export async function refreshInfo() {
  state.info = await call('app_info');
  emit('info');
}

export async function startJob(kind, opts = null) {
  const id = await call('start_job', { kind, opts });
  state.job = await call('job_state');
  emit('job');
  return id;
}

/** Resolves when the job with `id` finishes; returns its exit code. */
export function waitJob(id) {
  return new Promise(resolve => {
    const check = () => {
      if (state.job && state.job.id === id && !state.job.running) { off(); resolve(state.job.code); }
    };
    const off = subscribe(what => { if (what === 'job') check(); });
    check();
  });
}

export async function init() {
  await refreshInfo();
  try { state.job = await call('job_state'); } catch { /* none */ }
  await on('job-line', ({ id, line }) => {
    if (!state.job || state.job.id !== id) return;
    state.job.lines.push(line);
    if (state.job.lines.length > 3000) state.job.lines.splice(0, 750);
    emit('job-line');
  });
  await on('job-done', view => { state.job = view; emit('job'); refreshSnapshot(); });
  await refreshSnapshot();
  refreshResources();
  setInterval(refreshSnapshot, 3000);
  setInterval(refreshResources, 10000);
}
