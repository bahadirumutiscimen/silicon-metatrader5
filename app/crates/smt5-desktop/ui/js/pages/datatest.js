// Data test: same path as the Python client (bridge → MT5), rendered raw.
// Used standalone and as a step of the setup wizard (mountInto).

import { call } from '../api.js';
import { $, $$, html, render, icon, num } from '../dom.js';
import { state, targetName } from '../store.js';
import { t, th, locale } from '../i18n.js';
import { symbolSelect, bindSymbolSelect } from '../symbols.js';

// Turkish titles for the check ids of runtime/status/datatest.py (t() at render)
const TITLES = {
  initialize: 'Köprü bağlantısı', account: 'Hesap ve terminal', symbol: 'Sembol seçimi', clock: 'Broker saati',
  from_pos: 'Canlı barlar · copy_rates_from_pos', from: 'Tarihten geriye · copy_rates_from', range: 'Aralık · copy_rates_range', ticks: 'Canlı tick',
};
const MESSAGES = {
  ok: 'Üç yöntem de taze bar döndürdü.',
  market_closed: 'Veri geliyor ama piyasa kapalı görünüyor; tazelik değerlendirilemez.',
  stale: 'Bazı yöntemler eski bar döndürdü; geçmiş hâlâ senkron oluyor olabilir.',
};
const VERDICT = {
  ok: ['ok', 'Veri akışı doğrulandı'],
  market_closed: ['info', 'Piyasa kapalı — barlar geliyor, canlı tick yok'],
  stale: ['warn', 'Barlar geliyor ama eski'],
  no_data: ['warn', 'Henüz veri yok'],
  error: ['bad', 'Test başarısız'],
};

// "2026-11-04 13:15:00" → label suited to the timeframe
function tlabel(s, tf) {
  const dm = `${s.slice(8, 10)}.${s.slice(5, 7)}`;
  if (tf === 'D1') return dm;
  if (tf === 'H1' || tf === 'H4') return `${dm} ${s.slice(11, 16)}`;
  return s.slice(11, 16);
}

// Market Watch symbols, shared by every data test view for a minute
let watch = null, watchAt = 0, watchErr = null;
export async function loadWatch(force = false) {
  if (!force && watch && Date.now() - watchAt < 60000) return watch;
  try { watch = await call('market_watch'); watchAt = Date.now(); watchErr = null; }
  catch (e) { watchErr = String(e); }
  return watch;
}

function candles(bars, digits, tf) {
  const b = bars.slice(-40);
  if (b.length < 2) return html`<div class="muted" style="height:200px;display:flex;align-items:center;justify-content:center">${t('Bar yok')}</div>`;
  const W = 600, H = 200, lo = Math.min(...b.map(x => x.l)), hi = Math.max(...b.map(x => x.h));
  const span = hi - lo || 1, step = W / b.length, bw = Math.max(2, step * 0.6);
  const y = v => H - 6 - ((v - lo) / span) * (H - 12);
  const marks = [hi, lo + span * 0.5, lo];
  return html`<div class="row" style="gap:10px;align-items:stretch"><div class="grow">
    <svg class="chart" role="img" aria-label="${t('Son {n} bar mum grafiği', { n: b.length })}" height="${H}" viewBox="0 0 ${W} ${H}" preserveAspectRatio="none">
      ${marks.map(m => html`<path d="M0 ${y(m).toFixed(1)}H${W}" stroke="var(--line)" stroke-dasharray="4 4" vector-effect="non-scaling-stroke"/>`)}
      ${b.map((x, i) => {
        const cx = i * step + step / 2, upc = x.c >= x.o, col = upc ? 'var(--pos)' : 'var(--neg)';
        const top = y(Math.max(x.o, x.c)), bot = y(Math.min(x.o, x.c));
        return html`<path d="M${cx.toFixed(1)} ${y(x.h).toFixed(1)}V${y(x.l).toFixed(1)}" stroke="${col}" vector-effect="non-scaling-stroke"/><rect x="${(cx - bw / 2).toFixed(1)}" y="${top.toFixed(1)}" width="${bw.toFixed(1)}" height="${Math.max(1, bot - top).toFixed(1)}" fill="${col}"/>`;
      })}</svg>
    <div class="muted row between" style="font-size:12px;margin-top:6px"><span>${tlabel(b[0].t, tf)}</span><span>${tlabel(b[Math.floor(b.length / 2)].t, tf)}</span><span>${tlabel(b[b.length - 1].t, tf)}</span></div></div>
    <div class="muted num" style="font-size:11px;display:flex;flex-direction:column;justify-content:space-between;height:${H}px">${marks.map(m => html`<span>${num(m, digits)}</span>`)}</div></div>`;
}

export function datatestView(node, { compact = false, onResult } = {}) {
  let symbol = 'EURUSD', tf = 'M5', running = false, result = null, error = null;
  try { symbol = localStorage.getItem('dt-symbol') || symbol; tf = localStorage.getItem('dt-tf') || tf; } catch { /* ignore */ }
  loadWatch().then(w => {
    if (w?.symbols?.length && !w.symbols.includes(symbol)) symbol = w.symbols.includes('EURUSD') ? 'EURUSD' : w.symbols[0];
    draw();
  });

  async function run() {
    running = true; error = null; draw();
    try {
      result = await call('datatest', { symbol, tf, count: 300, ticks: 5 });
      try { localStorage.setItem('dt-symbol', symbol); localStorage.setItem('dt-tf', tf); } catch { /* ignore */ }
      onResult?.(result);
    } catch (e) { error = String(e); result = null; onResult?.(null); }
    running = false; draw();
  }

  function draw() {
    const r = result, ready = state.snap?.status?.phase === 'ready';
    const vd = r && VERDICT[r.verdict];
    const [tone, title] = r ? (vd ? [vd[0], t(vd[1])] : ['bad', r.verdict]) : ['info', ''];
    const passed = r ? r.checks.filter(c => c.ok).length : 0;
    const total = r ? r.checks.reduce((a, c) => a + (c.ms || 0), 0) : 0;
    const digits = r?.ticks?.digits ?? 5;
    const tk = r?.ticks;
    render(node, html`
      <section class="card pad col">
        <div class="row between wrap">
          <div class="row wrap">
            <label class="row" style="font-size:13px;font-weight:500">${t('Sembol')}
              ${watch?.symbols?.length
                ? symbolSelect('dt-sym', watch.symbols, symbol)
                : html`<input class="f" id="dt-sym" value="${symbol}" style="width:150px" placeholder="${t('ör. EURUSD.m')}" autocapitalize="characters" spellcheck="false">`}</label>
            <span class="muted" style="font-size:12px" title="${watchErr || ''}">${watch?.symbols?.length ? t('Market Watch · {n} sembol', { n: watch.symbols.length }) : watchErr ? t('Market Watch alınamadı') : ''}</span>
            <div class="seg" id="dt-tf">${['M1', 'M5', 'M15', 'H1', 'D1'].map(v => html`<button data-v="${v}" class="${v === tf ? 'sel' : ''}">${v}</button>`)}</div>
          </div>
          <button class="btn p" id="dt-run" ${running ? 'disabled' : ''}>${icon(running ? 'spinner' : 'play', running ? 'spin' : '')}${running ? t('Test çalışıyor… (~10 sn)') : r ? t('Testi yeniden çalıştır') : t('Testi çalıştır')}</button>
        </div>
        ${!ready && !r ? html`<div class="callout warn">${icon('info')}<div>${t('Runtime şu an "{phase}" durumunda. Test en iyi broker girişinden sonra (Hazır) çalışır.', { phase: state.snap?.status?.phase || t('kapalı') })}</div></div>` : ''}
        ${error ? html`<div class="callout bad">${icon('alert')}<div>${error}</div></div>` : ''}
        ${r ? html`<div class="callout ${tone}">${icon(tone === 'ok' ? 'check' : tone === 'bad' ? 'alert' : 'info')}<div><b>${title}</b> · ${t('{n}/{total} kontrol geçti', { n: passed, total: r.checks.length })}${r.server_offset ? ' · ' + t('broker saati {offset}', { offset: r.server_offset }) : ''}${r.server_now ? ` (${r.server_now.slice(11, 19)})` : ''}<br><span style="opacity:.85">${MESSAGES[r.verdict] ? t(MESSAGES[r.verdict]) : r.message}</span></div></div>` : ''}
      </section>
      ${r ? html`<div class="split">
        <section class="card pad main col">
          <div class="row between"><h2>${t('Kontroller')}</h2><span class="muted" style="font-size:12px">${t('toplam {s} sn', { s: (total / 1000).toFixed(1) })}</span></div>
          <div>${r.checks.map(c => html`<div class="check"><span class="dot ${c.ok ? 'ok' : 'bad'}">${icon(c.ok ? 'check' : 'x')}</span>
            <div class="grow"><div class="t">${TITLES[c.id] ? t(TITLES[c.id]) : c.title}</div><div class="s">${c.detail}</div></div><span class="muted num" style="font-size:12px">${t('{n} sn', { n: (c.ms / 1000).toFixed(2) })}</span></div>`)}</div>
        </section>
        <section class="card pad aside col">
          <div class="row between"><h2>${t('Canlı tick')}</h2>${tk ? html`<span class="chip ${tk.count ? 'up' : 'neu'}">${tk.count ? t('{n} tick / {s} sn', { n: tk.count, s: tk.seconds }) : t('Akış yok')}</span>` : ''}</div>
          ${tk ? html`<div class="grid" style="grid-template-columns:repeat(3,1fr);gap:8px">
              <div><div class="muted" style="font-size:12px">Bid</div><b class="num">${tk.bid != null ? num(tk.bid, digits) : '—'}</b></div>
              <div><div class="muted" style="font-size:12px">Ask</div><b class="num">${tk.ask != null ? num(tk.ask, digits) : '—'}</b></div>
              <div><div class="muted" style="font-size:12px">Spread</div><b class="num">${tk.spread_points != null ? num(tk.spread_points, 0) + ' pt' : '—'}</b></div></div>
            <table><thead><tr><th>${t('Zaman')}</th><th class="r">Bid</th><th class="r">Ask</th></tr></thead><tbody>
              ${(tk.samples || []).slice(-6).reverse().map(s => html`<tr><td class="num">${s.t.slice(11, 19)}</td><td class="r num">${num(s.bid, digits)}</td><td class="r num">${num(s.ask, digits)}</td></tr>`)}</tbody></table>` : html`<p class="muted" style="margin:0">${t('Tick örneği alınamadı.')}</p>`}
        </section></div>
      ${!compact ? html`<section class="card pad col">
          <div class="row between wrap"><div><h2>${r.symbol} · ${r.timeframe} · ${t('son {n} bar', { n: Math.min(40, r.bars.length) })}</h2><div class="muted" style="font-size:12px">${t('Köprüden gelen ham veri, işlem yapılmadan çizildi')}</div></div>
          ${Object.entries(r.methods || {}).map(([m, v]) => html`<span class="chip ${v.rows ? (v.fresh_min != null && v.fresh_min > 30 ? 'warn' : 'up') : 'down'}" title="${m}">${m.replace('copy_rates_', '')} · ${v.rows}${v.fresh_min != null ? ' · ' + t('{m} dk', { m: num(v.fresh_min, 1) }) : ''}</span>`)}</div>
          ${candles(r.bars, digits, r.timeframe)}</section>` : ''}` : ''}
      ${!compact ? html`<section class="card pad col"><h2>${t('"No data" görürsen')}</h2><ul class="muted" style="margin:0;padding-left:18px;font-size:13px;line-height:1.7">
        <li>${t('Broker girişinden sonra geçmiş barlar arka planda iner; 5–10 dakika bekleyip testi yeniden çalıştır.')}</li>
        <li>${t("Sembol Market Watch'ta değilse ya da broker'da adı farklıysa (ör. EURUSD.m) yukarıdan yaz.")}</li>
        <li>${th(`MaxBars çok yüksekse ilk senkron uzar; <a href="#/data">Veri Yönetimi</a>'nden düşürebilirsin.`)}</li></ul></section>` : ''}`);
    $('#dt-run', node).onclick = run;
    const sym = $('#dt-sym', node);
    if (sym.tagName === 'SELECT') bindSymbolSelect(sym, () => symbol, name => { symbol = name; draw(); });
    else sym.onchange = () => { symbol = sym.value.trim() || 'EURUSD'; };
    sym.onkeydown = e => { if (e.key === 'Enter' && sym.tagName === 'INPUT') { symbol = sym.value.trim() || 'EURUSD'; run(); } };
    $('#dt-tf', node).onclick = e => { const b = e.target.closest('button'); if (!b) return; tf = b.dataset.v; $$('#dt-tf button', node).forEach(x => x.classList.toggle('sel', x === b)); };
  }
  draw();
  return { run, redraw: draw };
}

// ---- speed test: compare runtime modes (Güvenli/QEMU vs Mac yerel) on this Mac ----
const BENCH_MODES = ['safe', 'native'];
const BENCH_ROWS = [
  ['boot_s', 'Açılış (başlat → MT5 + köprü)', 's', 'Runtime başladıktan MT5 ve köprünün çalışır olmasına kadar'],
  ['cpu_s', 'Windows Python CPU', 's', 'Wine içindeki Python ile 2 milyon adımlık döngü: emülasyon hızının saf ölçüsü'],
  ['rates_s', 'copy_rates_from_pos', 's', 'MT5 içinden M1 bar çekme'],
  ['transfer_s', '10.000 bar aktarımı', 's', 'Köprüden istemci tarafına satır aktarımı'],
  ['tick_ms', 'Köprü gecikmesi', 'ms', 'symbol_info_tick() ortalama gidiş-dönüş'],
];

// backend mode label "Güvenli · Colima …": translate the mode name, keep the technical part
const modeName = l => (l ? l.split(' · ').map((p, i) => (i ? p : t(p))).join(' · ') : '—');

function loadResults() { try { return JSON.parse(localStorage.getItem('bench-results') || '{}'); } catch { return {}; } }
function saveResult(mode, r) {
  const all = loadResults();
  all[mode] = { ...r, mode_label: state.snap?.mode_label || mode };
  try { localStorage.setItem('bench-results', JSON.stringify(all)); } catch { /* ignore */ }
}

function benchView(node) {
  let running = false, error = null;
  async function run() {
    running = true; error = null; draw();
    try {
      const r = await call('bench', { symbol: (localStorage.getItem('dt-symbol') || 'EURUSD'), bars: 50000 });
      if (!r.ok) error = r.error || t('test tamamlanamadı');
      saveResult(state.snap?.mode || 'safe', r);
    } catch (e) { error = String(e); }
    running = false; draw();
  }
  function draw() {
    const all = loadResults();
    const modes = BENCH_MODES.filter(m => all[m]);
    const base = all.safe;
    const fmt = (v, unit) => v == null ? '—' : `${num(v, unit === 'ms' || v < 10 ? 2 : 1)} ${unit}`;
    const ratio = (k, v) => base && base[k] && v ? base[k] / v : null;
    render(node, html`<section class="card pad col">
      <div class="row between wrap"><div><h2>${t('Hız testi')}</h2><div class="muted" style="font-size:13px;margin-top:2px">${th('Şu anki mod: <b>{mode}</b>. Her modda bir kez çalıştır; sonuçlar yan yana karşılaştırılır (düşük = iyi).', { mode: modeName(state.snap?.mode_label) })}</div></div>
        <button class="btn p" id="bn-run" ${running || !state.snap?.status ? 'disabled' : ''}>${icon(running ? 'spinner' : 'play', running ? 'spin' : '')}${running ? t('Ölçülüyor… (30–120 sn)') : t('Hız testini çalıştır')}</button></div>
      ${error ? html`<div class="callout bad">${icon('alert')}<div>${error}</div></div>` : ''}
      ${modes.length ? html`<div style="overflow-x:auto"><table>
        <thead><tr><th>${t('Ölçüm')}</th>${modes.map(m => html`<th class="r">${(all[m].mode_label || m).split(' · ').slice(-1)[0]}<div class="muted" style="font-weight:400;font-size:11px">${all[m].wine_flavor || ''} · ${all[m].cpu_translation || '?'}</div></th>`)}</tr></thead>
        <tbody>${BENCH_ROWS.map(([k, label, unit, hint]) => html`<tr><td title="${t(hint)}">${t(label)}</td>${modes.map(m => {
          const v = all[m][k];
          const rowsOf = r => (r ? (k === 'rates_s' ? r.rates_rows : k === 'transfer_s' ? r.transfer_rows : null) : null);
          const rows = rowsOf(all[m]);
          // a time only compares with the same number of bars (0 bars = nothing measured)
          const comparable = rows == null || (rows > 0 && rows === rowsOf(base));
          const x = m !== 'safe' && comparable ? ratio(k, v) : null;
          return html`<td class="r num">${fmt(v, unit)}${rows != null ? html`<div class="${rows ? 'muted' : 'neg'}" style="font-size:11px">${rows ? t('{n} bar', { n: rows.toLocaleString(locale()) }) : t('bar dönmedi')}</div>` : ''}${x ? html` <span class="chip ${x >= 1 ? 'up' : 'down'}" style="height:18px">${x >= 1 ? t('{x}× hızlı', { x: num(x, 1) }) : t('{x}× yavaş', { x: num(1 / x, 1) })}</span>` : ''}</td>`;
        })}</tr>`)}
        <tr><td class="muted">${t('Ölçüm zamanı')}</td>${modes.map(m => html`<td class="r muted" style="font-size:12px">${all[m].at ? new Date(all[m].at * 1000).toLocaleString(locale()) : '—'}</td>`)}</tr>
        </tbody></table></div>` : html`<p class="muted" style="margin:0;font-size:13px">${t('Henüz ölçüm yok. Önce Güvenli modda, sonra Mac yerel modda çalıştır.')}</p>`}
    </section>`);
    $('#bn-run', node).onclick = run;
  }
  draw();
  return { redraw: draw };
}

let view, bview;
export default {
  mount(node) {
    render(node, html`<div class="head"><div><div class="crumbs">SiliconMetaTrader5 › ${targetName() || t('Lokal runtime')} › <b>${t('Veri Testi')}</b></div><h1>${t('Gerçekten veri geliyor mu?')}</h1>
      <div class="muted" style="margin-top:4px">${t('Python istemcisiyle aynı yoldan köprüye bağlanıp bar ve tick çekiyoruz. Bar zamanları broker saatindedir.')}</div></div></div><div class="col" id="dt"></div><div class="col" id="bn"></div>`);
    view = datatestView($('#dt', node));
    bview = benchView($('#bn', node));
  },
  update(what) { if (what === 'snap' && bview && !document.querySelector('#bn-run .spin')) { const b = document.querySelector('#bn-run'); if (b) b.disabled = !state.snap?.status; } },
  unmount() { view = null; bview = null; },
};
