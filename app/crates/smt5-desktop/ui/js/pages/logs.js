import { call } from '../api.js';
import { $, $$, html, render, icon, esc, toast, copyText, bytes, modal } from '../dom.js';
import { state, isRunning, targetName } from '../store.js';
import { t } from '../i18n.js';

// Labels are translated where they are rendered.
const ALL_SOURCES = [['supervisor', 'Supervisor'], ['mt5', 'MT5 terminal'], ['bridge', 'Python köprüsü'], ['installer', 'Kurulum'], ['display', 'Görüntü'], ['status', 'Durum API']];
// Mac yerel mode: no installer / display logs (native.sh install prints to the job drawer)
const sources = () => ALL_SOURCES.filter(([k]) => !(state.snap?.native && ((k === 'installer' && state.snap.provider_kind !== 'windows') || (k === 'display' && state.snap.display !== 'novnc'))));
let el, timer, source = 'supervisor', level = 'all', query = '', follow = true, lines = [], sizes = {}, quiet = true;
try { quiet = localStorage.getItem('logs-quiet') !== '0'; } catch { /* ignore */ }

// Harmless lines from the display stack and old images (no sound card, no
// DPMS in Xvfb, plain-HTTP websockify, one line per viewer connection).
const NOISE = [
  /^ALSA lib /, /Xlib: +extension "DPMS" missing/, /could not find self\.pem/, /Plain non-SSL \(ws:\/\/\) WebSocket connection/,
  /Path: '\/websockify'/, /connecting to: localhost:5901/, /Using installed websockify/, /^\S+ - - \[[^\]]+\] "GET \/(?!websockify)/,
];
const isNoise = l => NOISE.some(r => r.test(l));

const levelOf = l => /\b(error|failed|fatal|traceback|exception|err:)/i.test(l) ? 'e' : /\b(warn|warning|retry|timeout|timed out|missing)/i.test(l) ? 'w' : /\b(ready|listening|detected|ok\b|complete|started)/i.test(l) ? 'o' : '';

function paint() {
  const box = $('#logbox', el); if (!box) return;
  const q = query.toLowerCase();
  const base = quiet ? lines.filter(l => !isNoise(l)) : lines;
  const hidden = lines.length - base.length;
  const shown = base.filter(l => (level === 'all' || levelOf(l) === level) && (!q || l.toLowerCase().includes(q)));
  const hn = $('#lg-hidden', el);
  if (hn) hn.textContent = quiet && hidden ? t('{n} gürültü satırı gizlendi', { n: hidden }) : '';
  const total = Object.values(sizes).reduce((a, b) => a + b, 0);
  const sz = $('#lg-size', el);
  if (sz) sz.textContent = total ? t('Bu log {size} · tüm loglar {total}', { size: bytes(sizes[source] || 0), total: bytes(total) }) : '';
  const counts = { e: 0, w: 0 };
  base.forEach(l => { const v = levelOf(l); if (v in counts) counts[v]++; });
  $('#lv-e', el).textContent = `${t('Hata')} · ${counts.e}`;
  $('#lv-w', el).textContent = `${t('Uyarı')} · ${counts.w}`;
  if (source === 'supervisor') state.logWarnings = counts.e;
  const stick = box.scrollTop + box.clientHeight >= box.scrollHeight - 40;
  box.innerHTML = shown.length ? shown.map(l => {
    const m = l.match(/^(\d{4}-\d\d-\d\d )?(\d\d:\d\d:\d\d)\s+(.*)$/);
    let body = esc(m ? m[3] : l);
    if (q) body = body.replace(new RegExp(esc(q).replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), 'gi'), x => `<mark>${x}</mark>`);
    return `<div class="${levelOf(l)}">${m ? `<span class="t">${m[2]}</span>  ` : ''}${body}</div>`;
  }).join('') : `<div class="t">${esc(t('Bu filtrede satır yok.'))}</div>`;
  if (follow && stick) box.scrollTop = box.scrollHeight;
}

async function load() {
  try {
    const r = await call('logs', { source, lines: 1500 });
    lines = r.lines || [];
    sizes = r.sizes || {};
    $('#logerr', el)?.classList.add('hidden');
  } catch (e) {
    const err = $('#logerr', el);
    if (err) {
      err.classList.remove('hidden');
      err.className = isRunning() ? 'callout bad' : 'callout info';
      err.lastChild.textContent = isRunning() ? String(e) : t('Runtime çalışmıyor: loglar durum API\'sinden okunur, başlatınca burada görünür.');
    }
  }
  paint();
}

export default {
  mount(node) {
    el = node;
    render(el, html`
      <div class="head">
        <div><div class="crumbs">SiliconMetaTrader5 › ${targetName() || t('Lokal runtime')} › <b>${t('Loglar')}</b></div><h1>${t('Loglar')}</h1></div>
        <div class="row wrap">
          <label class="row" style="font-size:13px;font-weight:500"><input type="checkbox" id="lg-follow" checked style="accent-color:var(--accent)"> ${t('Canlı takip')}</label>
          <label class="row" style="font-size:13px;font-weight:500" title="${t('ALSA, DPMS, websockify bağlantı satırları gibi zararsız satırları gizler')}"><input type="checkbox" id="lg-quiet" ${quiet ? 'checked' : ''} style="accent-color:var(--accent)"> ${t('Gürültüyü gizle')}</label>
          <button class="btn" id="lg-copy">${icon('copy')}${t('Kopyala')}</button>
          <button class="btn" id="lg-clear">${icon('trash')}${t('Temizle…')}</button>
          <button class="btn" id="lg-reload">${icon('refresh')}${t('Yenile')}</button>
        </div>
      </div>
      <div class="row wrap between">
        <div class="seg" id="lg-src">${sources().map(([k, l]) => html`<button data-v="${k}" class="${k === source ? 'sel' : ''}">${t(l)}</button>`)}</div>
        <div class="row">
          <div class="search" style="width:220px">${icon('logs')}<input id="lg-q" placeholder="${t('Ara')}" aria-label="${t('Loglarda ara')}"></div>
          <div class="seg" id="lg-lv"><button data-v="all" class="sel">${t('Tümü')}</button><button data-v="w" id="lv-w">${t('Uyarı')}</button><button data-v="e" id="lv-e">${t('Hata')}</button></div>
        </div>
      </div>
      <div class="callout bad hidden" id="logerr">${icon('alert')}<span></span></div>
      <section class="card"><div class="log selectable" id="logbox"></div></section>
      <div class="row between muted" style="font-size:12px"><span id="lg-size"></span><span id="lg-hidden"></span><a href="#/settings">${t('Log boyutu sınırı: Ayarlar')}</a></div>`);
    $('#lg-src', el).onclick = e => { const b = e.target.closest('button'); if (!b) return; source = b.dataset.v; $$('#lg-src button', el).forEach(x => x.classList.toggle('sel', x === b)); load(); };
    $('#lg-lv', el).onclick = e => { const b = e.target.closest('button'); if (!b) return; level = b.dataset.v; $$('#lg-lv button', el).forEach(x => x.classList.toggle('sel', x === b)); paint(); };
    $('#lg-q', el).oninput = e => { query = e.target.value; paint(); };
    $('#lg-follow', el).onchange = e => { follow = e.target.checked; };
    $('#lg-reload', el).onclick = load;
    $('#lg-quiet', el).onchange = e => { quiet = e.target.checked; try { localStorage.setItem('logs-quiet', quiet ? '1' : '0'); } catch { /* ignore */ } paint(); };
    $('#lg-clear', el).onclick = async () => {
      const ok = await modal({ title: t('Tüm loglar temizlensin mi?'), body: html`<p class="muted" style="margin:0">${t('Container içindeki altı log dosyası ve eski kopyaları boşaltılır. Çalışan süreçler yazmaya devam eder.')}</p>`, buttons: [{ label: t('Vazgeç'), value: false }, { label: t('Temizle'), value: true, primary: true, danger: true }] });
      if (!ok) return;
      try { const r = await call('clear_logs'); toast(t('Loglar temizlendi · {size} boşaldı', { size: bytes(r.freed_bytes) })); load(); } catch (e) { toast(String(e), true); }
    };
    $('#lg-copy', el).onclick = () => lines.length ? copyText(lines.join('\n')) : toast(t('Kopyalanacak satır yok'));
    load();
    timer = setInterval(() => { if (follow) load(); }, 3000);
  },
  unmount() { clearInterval(timer); el = null; },
};
