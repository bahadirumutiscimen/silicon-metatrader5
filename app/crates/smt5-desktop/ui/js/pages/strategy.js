// Education › Supertrend: runs runtime/status/supertrend_strategy.py (the same
// file the user can copy) on closed bars through the bridge, draws the result
// and, in live mode, checks again after every bar close.

import { call } from '../api.js';
import { $, $$, html, render, icon, num, toast, copyText } from '../dom.js';
import { state, startJob, waitJob } from '../store.js';
import { t, th, locale } from '../i18n.js';
import { symbolSelect, bindSymbolSelect } from '../symbols.js';
import { loadWatch } from './datatest.js';

const TFS = ['M1', 'M5', 'M15', 'M30', 'H1', 'H4', 'D1'];
const TF_SEC = { M1: 60, M5: 300, M15: 900, M30: 1800, H1: 3600, H4: 14400, D1: 86400 };
const KEY = 'smt5.supertrend';

let el = null, opts = null, res = null, error = null, running = false, again = false, preparing = 0, live = false, timer = null, source = null, watch = null, fresh = null, checkedAt = null;

function loadOpts() {
  const d = { symbol: 'EURUSD', tf: 'H1', period: 10, multiplier: 3, bars: 3000 };
  try { return { ...d, ...JSON.parse(localStorage.getItem(KEY) || '{}') }; } catch { return d; }
}
function saveOpts() { try { localStorage.setItem(KEY, JSON.stringify(opts)); } catch { /* private mode */ } }

const ts = (s, withDate = true, tf = null) => {
  if (s != null && tf === 'D1') return new Date(s * 1000).toISOString().slice(0, 10);
  if (s == null) return '—';
  const d = new Date(s * 1000).toISOString();          // bar times are on the server clock
  return withDate ? d.slice(0, 16).replace('T', ' ') : d.slice(11, 16);
};
const pts = (v, digits) => (v == null ? '—' : num(v * 10 ** digits, 0));

const NOT_READY = /no bars for|history still downloading/;

async function run(quiet = false, retry = 0) {
  if (running) { again = true; return; }        // a newer choice (symbol, timeframe) runs right after
  running = true; if (!quiet) error = null; draw();
  try {
    const r = await call('strategy_supertrend', { symbol: opts.symbol, tf: opts.tf, period: +opts.period, multiplier: +opts.multiplier, bars: +opts.bars });
    const prevLast = res?.signals?.at(-1);
    const nowLast = r.signals.at(-1);
    if (quiet && prevLast && nowLast && nowLast.time > prevLast.time && res.symbol === r.symbol && res.timeframe === r.timeframe) {
      fresh = nowLast;
      toast(t('Yeni sinyal: {side} {symbol} {tf} @ {price}', { side: nowLast.side === 'BUY' ? t('AL') : t('SAT'), symbol: r.symbol, tf: r.timeframe, price: num(nowLast.price, r.digits) }));
    }
    res = r; error = null; preparing = 0; checkedAt = Date.now(); saveOpts();
  } catch (e) {
    const msg = String(e);
    // MT5 builds a timeframe it was not asked for before (D1, H4 …) on the first
    // request and answers empty meanwhile: wait and ask again instead of failing.
    if (NOT_READY.test(msg) && retry < 5 && !again) {
      preparing = retry + 1; error = null; running = false; draw();
      setTimeout(() => { if (el) run(quiet, retry + 1); }, 4000);
      return;
    }
    preparing = 0; error = msg; if (!quiet) res = null;
  }
  running = false; draw();
  if (again) { again = false; run(false); }
}

function schedule() {
  clearTimeout(timer);
  if (!live || !el) return;
  timer = setTimeout(async () => { await run(true); schedule(); }, 10_000);
}

// ---------------------------------------------------------------- chart ----
function priceChart(r) {
  const c = r.chart, bars = c.bars, n = bars.length;
  if (n < 2) return html`<p class="muted">${t('Bar yok')}</p>`;
  const W = 1000, H = 360, PR = 64, PB = 22, PT = 14, cw = W - PR;
  const lo = Math.min(...bars.map(b => b[3]), ...c.line.filter(v => v != null));
  const hi = Math.max(...bars.map(b => b[2]), ...c.line.filter(v => v != null));
  const span = hi - lo || 1, step = cw / n, bw = Math.max(1, Math.min(8, step * 0.62));
  const y = v => PT + (1 - (v - lo) / span) * (H - PT - PB);
  const x = i => i * step + step / 2;
  const first = bars[0][0];
  // Supertrend as one path per trend run (gaps at flips, like on charts)
  const runs = []; let cur = null;
  c.line.forEach((v, i) => {
    if (v == null) { cur = null; return; }
    const d = c.trend[i];
    if (!cur || cur.d !== d) { cur = { d, pts: [] }; runs.push(cur); }
    cur.pts.push(`${x(i).toFixed(1)},${y(v).toFixed(1)}`);
  });
  const sig = r.signals.filter(s => s.time >= first).map(s => ({ ...s, i: bars.findIndex(b => b[0] === s.time) })).filter(s => s.i >= 0);
  const ticks = [hi, lo + span / 2, lo];
  const xl = [0, Math.floor(n / 2), n - 1];
  return html`<div class="chartwrap" id="st-chart" data-step="${step}" data-n="${n}" data-cw="${cw}" data-w="${W}">
    <svg viewBox="0 0 ${W} ${H}" role="img" aria-label="${t('{symbol} {tf} fiyat grafiği, Supertrend çizgisi ve sinyaller', { symbol: r.symbol, tf: r.timeframe })}">
      ${ticks.map(v => html`<path d="M0 ${y(v).toFixed(1)}H${cw}" class="grid"/><text x="${W - 4}" y="${(y(v) + 4).toFixed(1)}" text-anchor="end" class="ax">${num(v, r.digits)}</text>`)}
      ${xl.map(i => html`<text x="${x(i).toFixed(1)}" y="${H - 6}" text-anchor="${i === 0 ? 'start' : i === n - 1 ? 'end' : 'middle'}" class="ax">${ts(bars[i][0], true, r.timeframe)}</text>`)}
      ${bars.map((b, i) => {
        const up = b[4] >= b[1], top = y(Math.max(b[1], b[4])), bot = y(Math.min(b[1], b[4]));
        return html`<path d="M${x(i).toFixed(1)} ${y(b[2]).toFixed(1)}V${y(b[3]).toFixed(1)}" class="wick"/><rect x="${(x(i) - bw / 2).toFixed(1)}" y="${top.toFixed(1)}" width="${bw.toFixed(1)}" height="${Math.max(1, bot - top).toFixed(1)}" rx="1" class="${up ? 'cu' : 'cd'}"/>`;
      })}
      ${runs.map(rn => html`<polyline points="${rn.pts.join(' ')}" class="stl ${rn.d === 1 ? 'up' : 'dn'}"/>`)}
      ${sig.map(s => {
        const b = bars[s.i], buy = s.side === 'BUY', yy = buy ? y(b[3]) + 16 : y(b[2]) - 16, cx = x(s.i);
        return html`<g class="sig ${buy ? 'buy' : 'sell'}"><path d="${buy ? `M${cx} ${yy - 7}l6 10h-12z` : `M${cx} ${yy + 7}l6 -10h-12z`}"/><text x="${cx}" y="${buy ? yy + 15 : yy - 9}" text-anchor="middle">${buy ? t('AL') : t('SAT')}</text></g>`;
      })}
      <path id="st-x" d="" class="cross"/>
    </svg><div class="tip hidden" id="st-tip"></div></div>
    <div class="row wrap legend"><span><i class="ln up"></i>${t('Supertrend · yükseliş')}</span><span><i class="ln dn"></i>${t('Supertrend · düşüş')}</span><span class="pos">▲ ${t('AL sinyali')}</span><span class="neg">▼ ${t('SAT sinyali')}</span><span class="muted">${t('Son {n} kapanmış bar', { n })}</span></div>`;
}

function bindChart(r) {
  const wrap = $('#st-chart', el); if (!wrap) return;
  const svg = wrap.querySelector('svg'), tip = $('#st-tip', el), cross = $('#st-x', el);
  const c = r.chart, n = c.bars.length, W = +wrap.dataset.w, cw = +wrap.dataset.cw, step = +wrap.dataset.step;
  svg.addEventListener('mousemove', e => {
    const box = svg.getBoundingClientRect(), sx = (e.clientX - box.left) * (W / box.width);
    if (sx > cw) { tip.classList.add('hidden'); cross.setAttribute('d', ''); return; }
    const i = Math.max(0, Math.min(n - 1, Math.floor(sx / step))), b = c.bars[i], cx = i * step + step / 2;
    cross.setAttribute("d", `M${cx.toFixed(1)} 14V338`);
    const s = r.signals.find(q => q.time === b[0]);
    tip.innerHTML = '';
    render(tip, html`<b>${ts(b[0])}</b><div class="num">O ${num(b[1], r.digits)} · H ${num(b[2], r.digits)}<br>L ${num(b[3], r.digits)} · C ${num(b[4], r.digits)}</div>
      ${c.line[i] != null ? html`<div>Supertrend <b class="num">${num(c.line[i], r.digits)}</b> · ${c.trend[i] === 1 ? t('yükseliş') : t('düşüş')}</div>` : html`<div class="muted">${t('ısınma (ATR hesaplanıyor)')}</div>`}
      ${s ? html`<div class="${s.side === 'BUY' ? 'pos' : 'neg'}"><b>${s.side === 'BUY' ? t('AL sinyali') : t('SAT sinyali')}</b> · ${t('sonraki barın açılışında işlem')}</div>` : ''}`);
    tip.classList.remove('hidden');
    const px = (cx / W) * box.width;
    tip.style.left = `${Math.min(box.width - tip.offsetWidth - 4, Math.max(4, px + 12))}px`;
  });
  svg.addEventListener('mouseleave', () => { tip.classList.add('hidden'); cross.setAttribute('d', ''); });
}

function equityChart(r) {
  const eq = r.stats.equity || [];
  if (eq.length < 2) return html`<p class="muted" style="margin:0">${t('Eğri için en az iki kapanmış işlem gerekir.')}</p>`;
  const W = 600, H = 140, P = 6, k = 10 ** r.digits;
  const v = [0, ...eq.map(e => e * k)], lo = Math.min(...v), hi = Math.max(...v), span = hi - lo || 1;
  const x = i => P + (i / (v.length - 1)) * (W - 2 * P), y = q => P + (1 - (q - lo) / span) * (H - 2 * P);
  return html`<svg class="eq" viewBox="0 0 ${W} ${H}" role="img" aria-label="${t('Kapanmış işlemlerle birikimli sonuç (puan)')}">
    <path d="M${P} ${y(0).toFixed(1)}H${W - P}" class="zero"/>
    <polyline points="${v.map((q, i) => `${x(i).toFixed(1)},${y(q).toFixed(1)}`).join(' ')}" class="eqline"/>
    ${v.map((q, i) => html`<circle cx="${x(i).toFixed(1)}" cy="${y(q).toFixed(1)}" r="7" class="hit"><title>${t('İşlem {i}: {v} puan', { i, v: num(q, 0) })}</title></circle>`)}</svg>`;
}

function stat(label, value, sub, tone = '') {
  return html`<div class="card stat"><div class="muted" style="font-size:13px">${label}</div><div class="v ${tone}">${value}</div>${sub ? html`<div class="muted" style="font-size:12px">${sub}</div>` : ''}</div>`;
}

// ----------------------------------------------------------------- page ----
function draw() {
  if (!el) return;
  const r = res, s = r?.stats, ready = state.snap?.status?.phase === 'ready';
  const last = r?.signals?.at(-1);
  render(el, html`
    <div class="head"><div><div class="crumbs">SiliconMetaTrader5 › ${t('Eğitim')} › <b>${t('Supertrend örneği')}</b></div><h1>${t('Supertrend al/sat')}</h1>
      <div class="row wrap" style="margin-top:6px;gap:6px"><span class="chip up">${t('Repaint yok')}</span><span class="chip acc">${t('Event-driven')}</span><span class="chip neu">${t('Yalnız kapanmış barlar')}</span><span class="chip neu">${t('Backtest = canlı')}</span></div></div>
      <a class="btn" href="#/learn">${icon('book')}${t('Python API rehberi')}</a></div>

    ${!ready ? html`<div class="callout warn">${icon('info')}<div>${t('Strateji veriyi köprüden çeker; runtime Hazır (broker girişi yapılmış) olmalı.')}</div></div>` : ''}

    <section class="card pad col">
      <div class="row wrap" style="gap:12px">
        <label class="row" style="font-size:13px;font-weight:500">${t('Sembol')} ${watch?.symbols?.length ? symbolSelect('st-sym', watch.symbols, opts.symbol) : html`<input class="f" id="st-sym" value="${opts.symbol}" style="width:130px" spellcheck="false">`}</label>
        <div class="seg" id="st-tf" role="group" aria-label="${t('Zaman dilimi')}">${TFS.map(v => html`<button data-v="${v}" class="${v === opts.tf ? 'sel' : ''}">${v}</button>`)}</div>
        <label class="row" style="font-size:13px" title="${t('ATR periyodu')}">${t('Periyot')} <input class="f num" id="st-p" type="number" min="1" max="500" value="${opts.period}" style="width:70px"></label>
        <label class="row" style="font-size:13px" title="${t('ATR çarpanı')}">${t('Çarpan')} <input class="f num" id="st-m" type="number" min="0.1" max="20" step="0.1" value="${opts.multiplier}" style="width:70px"></label>
        <label class="row" style="font-size:13px">${t('Geçmiş')} <select class="f" id="st-b">${[1000, 3000, 10000, 20000].map(b => html`<option value="${b}" ${+opts.bars === b ? 'selected' : ''}>${t('{n} bar', { n: num(b, 0) })}</option>`)}</select></label>
        <div class="grow"></div>
        <button class="btn ${live ? 'p' : ''}" id="st-live" ${!res ? 'disabled' : ''} title="${t('Her 10 saniyede bir yeni kapanan barı kontrol eder')}">${icon(live ? 'stop' : 'pulse')}${live ? t('Canlıyı durdur') : t('Canlı izle')}</button>
        <button class="btn p" id="st-run" ${running ? 'disabled' : ''}>${icon(running ? 'spinner' : 'play', running ? 'spin' : '')}${running ? t('Hesaplanıyor…') : t('Çalıştır')}</button>
      </div>
      ${preparing ? html`<div class="callout info">${icon('spinner', 'spin')}<div>${t('MT5 {symbol} {tf} geçmişini hazırlıyor (bu zaman dilimi ilk kez isteniyor); birkaç saniye içinde yeniden denenecek… ({n}/5)', { symbol: opts.symbol, tf: opts.tf, n: preparing })}</div></div>` : ''}
      ${error && NOT_READY.test(error) ? html`<div class="callout warn">${icon('info')}<div>${t('MT5 {symbol} {tf} için hâlâ bar vermiyor. Broker bu sembolü bu zaman diliminde sunmuyor olabilir ya da geçmiş henüz inmedi; biraz sonra Çalıştır\'a bas.', { symbol: opts.symbol, tf: opts.tf })}</div></div>`
        : error ? (/\b404\b/.test(error)
        ? html`<div class="callout warn">${icon('info')}<div>${state.snap?.native
            ? t('Çalışan runtime bu özelliği henüz tanımıyor: uygulama güncellendi ama runtime eski betiklerle çalışıyor. Yeniden başlatınca yeni betikler kopyalanır.')
            : t('Güvenli mod imajı bu özellikten eski. Kurulum Sihirbazı\'ndan imajı yeniden derle ya da çek.')}
            ${state.snap?.native ? html`<div style="margin-top:8px"><button class="btn sm p" id="st-restart" ${state.job?.running ? 'disabled' : ''}>${icon('refresh')}${t('Runtime\'ı yeniden başlat')}</button></div>` : ''}</div></div>`
        : html`<div class="callout bad">${icon('alert')}<div>${error}</div></div>`) : ''}
      ${live && r ? html`<div class="callout ${fresh ? 'ok' : 'info'}">${icon(fresh ? 'check' : 'pulse')}<div>${fresh
        ? t('Yeni sinyal: {side} {symbol} {tf} @ {price}', { side: fresh.side === 'BUY' ? t('AL') : t('SAT'), symbol: r.symbol, tf: r.timeframe, price: num(fresh.price, r.digits) })
        : t('Canlı izleniyor: son kapanmış bar {last}, oluşan bar {forming}. Sinyal yalnız bar kapanınca verilir.', { last: ts(r.last_closed), forming: ts(r.forming, false) })}
        <span class="muted"> · ${t('son kontrol {time}', { time: new Date(checkedAt).toLocaleTimeString(locale()) })}</span></div></div>` : ''}
    </section>

    ${r ? html`
    <div class="grid g4">
      ${stat(t('Şu anki trend'), r.trend === 1 ? t('Yükseliş') : t('Düşüş'), t('Supertrend {v}', { v: num(r.line_now, r.digits) }), r.trend === 1 ? 'pos' : 'neg')}
      ${stat(t('Son sinyal'), last ? (last.side === 'BUY' ? '▲ ' + t('AL') : '▼ ' + t('SAT')) : '—', last ? `${ts(last.time)} · ${num(last.price, r.digits)}` : t('sinyal yok'), last ? (last.side === 'BUY' ? 'pos' : 'neg') : '')}
      ${stat(t('Net sonuç'), t('{v} puan', { v: pts(s.net, r.digits) }), t('{n} işlem · kazanma %{w}', { n: s.trades, w: s.win_rate == null ? '—' : Math.round(s.win_rate * 100) }), s.net >= 0 ? 'pos' : 'neg')}
      ${stat(t('Profit factor'), s.profit_factor == null ? '—' : num(s.profit_factor, 2), t('En büyük düşüş {v} puan', { v: pts(s.max_drawdown, r.digits) }))}
    </div>

    <section class="card pad col">
      <div class="row between wrap"><h2>${r.symbol} · ${r.timeframe} · ${t('Supertrend({p}, {m})', { p: r.params.period, m: r.params.multiplier })}</h2>
        <span class="muted" style="font-size:12px">${t('{n} kapanmış bar · {from} → {to} (sunucu saati)', { n: num(r.count, 0), from: ts(r.from), to: ts(r.last_closed) })}</span></div>
      ${priceChart(r)}
    </section>

    <div class="split">
      <section class="card pad main col"><div class="row between"><h2>${t('İşlemler')}</h2><span class="muted" style="font-size:12px">${t('her zaman pozisyonda · sinyalde ters çevirir')}</span></div>
        <table><thead><tr><th>${t('Yön')}</th><th>${t('Giriş')}</th><th class="r">${t('Fiyat')}</th><th>${t('Çıkış')}</th><th class="r">${t('Fiyat')}</th><th class="r">${t('Puan')}</th></tr></thead><tbody>
          ${r.trades.slice(-12).reverse().map(tr => html`<tr><td class="${tr.side === 'BUY' ? 'pos' : 'neg'}">${tr.side === 'BUY' ? '▲ ' + t('AL') : '▼ ' + t('SAT')}</td><td class="num">${ts(tr.entry_time)}</td><td class="r num">${num(tr.entry_price, r.digits)}</td>
            <td class="num">${tr.open ? html`<span class="chip neu" style="height:18px">${t('açık')}</span>` : ts(tr.exit_time)}</td><td class="r num">${num(tr.exit_price, r.digits)}</td><td class="r num ${tr.pnl >= 0 ? 'pos' : 'neg'}">${pts(tr.pnl, r.digits)}</td></tr>`)}</tbody></table>
        <p class="muted" style="margin:0;font-size:12px">${t('Spread, komisyon ve swap dahil değil. Geçmiş sonuçlar geleceği garanti etmez.')}</p></section>
      <section class="card pad aside col"><h2>${t('Birikimli sonuç (puan)')}</h2>${equityChart(r)}
        <div class="muted" style="font-size:12px">${t('Kazanan {w} · kaybeden {l} · ortalama {a} puan', { w: s.wins, l: s.trades - s.wins, a: pts(s.avg_trade, r.digits) })}</div></section>
    </div>` : !error && !running ? html`<section class="card empty"><p>${t('Sembol ve zaman dilimini seçip Çalıştır\'a bas: strateji kapanmış barlar üzerinde geçmişi tarar, sinyalleri ve işlemleri gösterir.')}</p></section>` : ''}

    <section class="card pad col">
      <div class="row between wrap"><div><h2>${t('Kaynak kod')}</h2><div class="muted" style="font-size:12px">${t('Bu ekranın çalıştırdığı dosyanın kendisi: supertrend_strategy.py')}</div></div>
        <button class="btn" id="st-copy" ${source ? '' : 'disabled'}>${icon('copy')}${t('Kodu kopyala')}</button></div>
      <div class="callout info">${icon('info')}<div>${th('Kendi bilgisayarında: <code>python3 supertrend_strategy.py backtest --symbol EURUSD --tf H1</code> ya da canlı için <code>live --tf M5</code>. Emir bloğu varsayılan olarak kapalıdır (<code>DRY_RUN = True</code>) ve gerçek hesapta çalışmayı reddeder.')}</div></div>
      <pre class="code selectable srccode">${source ?? t('Yükleniyor…')}</pre>
    </section>`);

  $('#st-run', el).onclick = () => run(false);
  $('#st-live', el).onclick = () => { live = !live; fresh = null; draw(); if (live) run(true).then(schedule); else clearTimeout(timer); };
  $('#st-tf', el).onclick = e => { const b = e.target.closest('button'); if (!b) return; opts.tf = b.dataset.v; saveOpts(); run(false); };
  const sym = $('#st-sym', el);
  if (sym.tagName === 'SELECT') bindSymbolSelect(sym, () => opts.symbol, name => { opts.symbol = name; saveOpts(); run(false); });
  else sym.onchange = () => { opts.symbol = sym.value.trim() || 'EURUSD'; saveOpts(); };
  $('#st-p', el).onchange = e => { opts.period = Math.max(1, Math.min(500, parseInt(e.target.value, 10) || 10)); saveOpts(); };
  $('#st-m', el).onchange = e => { opts.multiplier = Math.max(0.1, Math.min(20, parseFloat(e.target.value) || 3)); saveOpts(); };
  $('#st-b', el).onchange = e => { opts.bars = +e.target.value; saveOpts(); };
  $('#st-restart', el)?.addEventListener('click', async () => {
    try {
      if (state.snap?.container?.status === 'running') await waitJob(await startJob('stop'));
      await waitJob(await startJob('start'));
      error = null; draw();
    } catch (e) { error = String(e); draw(); }
  });
  $('#st-copy', el).onclick = () => source && copyText(source);
  if (r) bindChart(r);
}

export default {
  mount(node) {
    el = node; opts = loadOpts(); fresh = null;
    draw();
    if (source == null) call('example_source', { name: 'supertrend_strategy.py' }).then(s => { source = s; draw(); }).catch(e => { source = String(e); draw(); });
    loadWatch().then(w => { watch = w; draw(); }).catch(() => {});
    if (state.snap?.status?.phase === 'ready' && !res) run(false);
    if (live) schedule();
  },
  update(what) { if (what === 'info') draw(); },
  unmount() { clearTimeout(timer); el = null; },
};
