import { call } from '../api.js';
import { $, $$, html, render, icon, money, num, bytes, duration, clock, toast, modal } from '../dom.js';
import { state, phaseOf, account, probe, startJob, nativeWindow } from '../store.js';
import { t, th } from '../i18n.js';

let el;

// ---- account result from MT5's deal history (per account, never mixed) ----
const RANGES = [['today', 'Bugün'], ['7d', '7 gün'], ['30d', '30 gün'], ['90d', '90 gün'], ['all', 'Tümü']];
let range = (() => { try { return localStorage.getItem('ov-range') || '30d'; } catch { return '30d'; } })();
let hist = null, histErr = null, histKey = '', histBusy = false, histAt = 0;

async function loadHistory(force = false) {
  const a = account();
  const key = a ? `${a.server}|${a.login_masked}|${range}|${Math.round((a.balance || 0) * 100)}` : '';
  if (!key || histBusy || (!force && key === histKey && Date.now() - histAt < 60000)) return;
  histBusy = true;
  try { hist = await call('account_history', { range }); histErr = null; histKey = key; histAt = Date.now(); }
  catch (e) { histErr = String(e); if (!/\b404\b/.test(histErr) && hist?.account?.server !== a.server) hist = null; }
  histBusy = false;
  draw();
}

const fmtTime = s => new Date(s * 1000).toISOString().slice(0, 16).replace('T', ' ');   // server clock

function historyChart(h, cur) {
  const pts = (h.curve || []).map(p => ({ t: p[0], v: p[1], k: p[2] }));
  if (pts.length && h.to > pts[pts.length - 1].t) pts.push({ t: h.to, v: h.balance, k: -2 });
  if (pts.length < 2) return html`<div class="muted" style="height:180px;display:flex;align-items:center;justify-content:center;font-size:13px">${t('Bu aralıkta işlem yok.')}</div>`;
  const W = 600, H = 180, P = 8;
  const vals = [...pts.map(p => p.v), h.equity];
  let lo = Math.min(...vals), hi = Math.max(...vals);
  if (hi - lo < 1e-9) { lo -= 1; hi += 1; }
  const pad = (hi - lo) * 0.1; lo -= pad; hi += pad;
  const t0 = pts[0].t, t1 = pts[pts.length - 1].t;
  const x = tt => P + ((tt - t0) / Math.max(1, t1 - t0)) * (W - 2 * P);
  const y = v => H - ((v - lo) / (hi - lo)) * H;
  let d = `M${x(pts[0].t).toFixed(1)} ${y(pts[0].v).toFixed(1)}`;
  for (let i = 1; i < pts.length; i++) d += ` H${x(pts[i].t).toFixed(1)} V${y(pts[i].v).toFixed(1)}`;   // balance changes in steps
  const last = pts[pts.length - 1];
  const ticks = [hi, (hi + lo) / 2, lo];
  return html`<div class="row" style="gap:10px;align-items:stretch">
    <div class="muted num" style="font-size:11px;display:flex;flex-direction:column;justify-content:space-between;height:${H}px">${ticks.map(v => html`<span>${num(v, 0)}</span>`)}</div>
    <div class="grow chartwrap" id="ov-hist" data-w="${W}">
      <svg class="chart" role="img" aria-label="${t('Bakiye eğrisi (MT5 işlem geçmişi)')}" height="${H}" viewBox="0 0 ${W} ${H}" preserveAspectRatio="none" style="height:${H}px">
        <path d="M0 ${H / 2}H${W}" stroke="var(--line)" stroke-dasharray="4 4" vector-effect="non-scaling-stroke"/>
        <path d="${d} V${H} H${x(pts[0].t).toFixed(1)} Z" fill="var(--accent)" fill-opacity=".08"/>
        <path d="${d}" fill="none" stroke="var(--accent)" stroke-width="2" stroke-linejoin="round" vector-effect="non-scaling-stroke"/>
        ${Math.abs(h.floating) > 0.005 ? html`<path d="M${x(last.t).toFixed(1)} ${y(last.v).toFixed(1)} V${y(h.equity).toFixed(1)}" stroke="${h.floating >= 0 ? 'var(--pos)' : 'var(--neg)'}" stroke-width="2" stroke-dasharray="3 3" vector-effect="non-scaling-stroke"/>` : ''}
        <path id="ov-x" d="" stroke="var(--text-2)" stroke-dasharray="2 3" vector-effect="non-scaling-stroke"/></svg>
      <div class="tip hidden" id="ov-tip"></div>
      <div class="muted row between" style="font-size:12px;margin-top:6px"><span>${fmtTime(t0)}</span><span>${t('şimdi (sunucu saati {time})', { time: fmtTime(t1).slice(11) })}</span></div></div></div>`;
}

function bindHistoryChart(h, cur) {
  const wrap = $('#ov-hist', el); if (!wrap) return;
  const svg = wrap.querySelector('svg'), tip = $('#ov-tip', el), cross = $('#ov-x', el);
  const pts = (h.curve || []).map(p => ({ t: p[0], v: p[1], k: p[2] }));
  if (pts.length < 2) return;
  const W = +wrap.dataset.w, P = 8, t0 = pts[0].t, t1 = Math.max(h.to, pts[pts.length - 1].t);
  const KIND = { 0: t('İşlem'), 1: t('İşlem'), 2: t('Yatırım / çekim'), [-1]: t('Aralık başı') };
  svg.addEventListener('mousemove', e => {
    const box = svg.getBoundingClientRect(), sx = (e.clientX - box.left) * (W / box.width);
    const tt = t0 + ((sx - P) / (W - 2 * P)) * (t1 - t0);
    let i = 0; while (i + 1 < pts.length && pts[i + 1].t <= tt) i++;
    const p = pts[i];
    cross.setAttribute('d', `M${sx.toFixed(1)} 0V180`);
    render(tip, html`<b>${fmtTime(p.t)}</b><div>${t('Bakiye')} <b class="num">${money(p.v, cur)}</b></div><div class="muted">${KIND[p.k] ?? t('Diğer')}</div>`);
    tip.classList.remove('hidden');
    tip.style.left = `${Math.min(box.width - tip.offsetWidth - 4, Math.max(4, (sx / W) * box.width + 12))}px`;
  });
  svg.addEventListener('mouseleave', () => { tip.classList.add('hidden'); cross.setAttribute('d', ''); });
}

function historyCard(a, cur) {
  const h = hist && a && hist.account && hist.account.server === a.server ? hist : null;
  const pl = h?.trading_pl;
  const pct = h && h.start_balance > 0 ? (pl / h.start_balance) * 100 : null;
  return html`<section class="card pad main col" aria-label="${t('Hesap sonucu')}">
    <div class="row between wrap"><div><div class="muted" style="font-size:13px">${t('İşlem sonucu · MT5 geçmişi')}${h?.account ? ` · ${h.account.login}` : ''}</div>
      <div class="row" style="margin-top:4px"><span class="num ${pl == null ? '' : pl >= 0 ? 'pos' : 'neg'}" style="font-size:23px;font-weight:650">${pl == null ? '—' : (pl >= 0 ? '+' : '') + money(pl, cur)}</span>
        ${pct != null ? html`<span class="chip ${pl >= 0 ? 'up' : 'down'}">${pl >= 0 ? '+' : ''}${num(pct, 2)}%</span>` : ''}${histBusy ? icon('spinner', 'spin') : ''}</div></div>
      <div class="seg" id="ov-range" role="group" aria-label="${t('Aralık')}">${RANGES.map(([v, l]) => html`<button data-v="${v}" class="${v === range ? 'sel' : ''}">${t(l)}</button>`)}</div></div>
    ${histErr && !h ? html`<div class="callout ${/\b404\b/.test(histErr) ? 'warn' : 'bad'}">${icon('info')}<div>${/\b404\b/.test(histErr) ? t('İşlem geçmişi için runtime\'ın yeniden başlatılması gerekiyor (kenar çubuğundaki uyarı).') : histErr}</div></div>` : ''}
    ${h ? historyChart(h, cur) : !histErr ? html`<div class="muted" style="height:180px;display:flex;align-items:center;justify-content:center;font-size:13px">${a ? t('MT5 işlem geçmişi okunuyor…') : t('MT5\'te giriş yapılmış bir hesap yok.')}</div>` : ''}
    ${h ? html`<div class="row wrap hstats">
      <span>${t('Kapanan işlem')} <b class="num">${h.trades}</b></span>
      <span>${t('Kazanan')} <b class="num pos">${h.wins}</b> · ${t('Kaybeden')} <b class="num neg">${h.losses}</b></span>
      ${h.deposits ? html`<span>${t('Yatırım')} <b class="num">+${money(h.deposits, cur)}</b></span>` : ''}
      ${h.withdrawals ? html`<span>${t('Çekim')} <b class="num">${money(h.withdrawals, cur)}</b></span>` : ''}
      ${h.other ? html`<span>${t('Diğer (ücret, faiz)')} <b class="num">${money(h.other, cur)}</b></span>` : ''}
      <span>${t('Açık pozisyonlar')} <b class="num ${h.floating >= 0 ? 'pos' : 'neg'}">${h.floating >= 0 ? '+' : ''}${money(h.floating, cur)}</b></span></div>
      <div class="muted" style="font-size:12px">${t('Yalnız bu hesabın kapanan işlemleri (kâr + komisyon + swap). Para yatırma ve çekme sonuca katılmaz; hesap değişince sonuç da değişir.')}</div>` : ''}
  </section>`;
}

const STEPS = [
  ['prefix', 'Wine prefix hazır'],
  ['installed', 'MT5 kurulu'],
  ['terminal', 'MT5 terminali çalışıyor'],
  ['bridge', 'Python köprüsü dinliyor'],
  ['login', 'Broker girişi yapıldı'],
];
const ORDER = ['preparing_prefix', 'installing_mt5', 'starting_terminal', 'starting_bridge', 'waiting_login', 'ready'];

function stepStates(s) {
  const st = s?.status;
  if (!st) return STEPS.map(() => 'wait');
  const idx = ORDER.indexOf(st.phase);
  const done = {
    prefix: idx > 0 || st.mt5?.installed,
    installed: !!st.mt5?.installed,
    terminal: !!st.mt5?.running,
    bridge: !!st.bridge?.listening,
    login: st.phase === 'ready',
  };
  const order = STEPS.map(([k]) => k);
  const firstOpen = order.find(k => !done[k]);
  return order.map(k => done[k] ? 'ok' : (st.phase === 'error' && k === firstOpen ? 'bad' : k === firstOpen ? 'run' : 'wait'));
}

function latencySpark(samples) {
  const last = samples.slice(-24).filter(s => s.latency != null);
  if (!last.length) return html`<div class="muted" style="font-size:12px">${t('Henüz örnek yok')}</div>`;
  const max = Math.max(...last.map(s => s.latency), 1);
  const avg = last.reduce((a, s) => a + s.latency, 0) / last.length;
  return html`<div class="row between" style="font-size:13px;margin-bottom:10px"><span>${t('Köprü gecikmesi · son {n} yoklama', { n: last.length })}</span><span class="muted num">${t('ort. {v} ms', { v: num(avg, 0) })}</span></div>
    <div class="spark" aria-hidden="true">${last.map((s, i) => html`<span class="${i === last.length - 1 ? 'last' : s.latency > avg * 2 ? 'hi' : ''}" style="height:${Math.max(6, (s.latency / max) * 100)}%"></span>`)}</div>`;
}

function meter(label, value, pct) {
  const cls = pct >= 90 ? 'full' : pct >= 75 ? 'hot' : '';
  return html`<div class="col" style="gap:6px"><div class="row between" style="font-size:13px"><span>${label}</span><span class="num" style="font-weight:600">${value}</span></div>
    <div class="meter ${cls}" role="meter" aria-label="${label}" aria-valuenow="${Math.round(pct)}" aria-valuemin="0" aria-valuemax="100"><span style="width:${Math.min(100, Math.max(0, pct))}%"></span></div></div>`;
}

function statCard(ic, label, value, chip) {
  return html`<div class="card stat"><div class="ico">${icon(ic)}</div><div><div class="muted" style="font-size:13px">${label}</div>
    <div class="row" style="margin-top:4px"><span class="v">${value}</span>${chip || ''}</div></div></div>`;
}

function draw() {
  const s = state.snap;
  if (!el || !s) return;
  const ph = phaseOf();
  const logo = document.documentElement.dataset.theme === 'dark' ? 'img/logo-dark.svg' : 'img/logo-light.svg';
  const srv = s.target && s.target !== 'local';
  const where = srv ? s.target_name || s.target : t('Lokal runtime');
  const head = html`<div class="head"><div><div class="crumbs">SiliconMetaTrader5 › ${where} › <b>${t('Genel Bakış')}</b></div><h1>${t('Genel Bakış')}</h1></div>
    <span class="chip lg ${ph.tone}"><span class="d"></span>${ph.label}</span></div>`;
  // A server whose tunnel is not up yet (app just opened) or is down: no runtime data to show.
  if (srv && (ph.key === 'connecting' || ph.key === 'unreachable')) {
    const busy = ph.key === 'connecting';
    render(el, html`${head}<div class="card empty"><img class="logo-img" src="${logo}" alt="">
      <h2 style="font-size:18px">${busy ? t('{name} sunucusuna bağlanılıyor…', { name: where }) : t('{name} sunucusuna ulaşılamıyor', { name: where })}</h2>
      <p>${busy ? t('SSH tüneli açılıyor; genelde birkaç saniye sürer. MT5 sunucuda çalışmaya devam ediyor.') : t('SSH bağlantısı kurulamadı. Uygulama arka planda yeniden deniyor; sorunun ayrıntısı Sunucular sayfasında.')}</p>
      ${!busy && s.link?.detail ? html`<p class="mono" style="font-size:12px">${s.link.detail}</p>` : ''}
      <div class="row" style="justify-content:center">${busy ? html`<span class="chip acc">${icon('spinner', 'spin')}${t('Bağlanıyor')}</span>` : html`<a class="btn p" href="#/servers">${icon('server')}${t('Sunucular sayfasını aç')}</a>`}</div></div>`);
    return;
  }
  // Windows: nobody is logged on, so MT5 cannot run (store maps it to group 'off': handle it first)
  if (ph.key === 'no_session') {
    render(el, html`${head}<div class="card empty"><img class="logo-img" src="${logo}" alt="">
      <h2 style="font-size:18px">${t('Windows oturumu açık değil')}</h2>
      <p>${t("MT5 yalnız açık bir Windows oturumunda çalışır. Sunucuda kimse oturum açmamış: MT5 Ekranı'nı açıp Windows'a giriş yap; otomatik oturum açıksa sunucuyu yeniden başlatmak da yeter.")}</p>
      <div class="row" style="justify-content:center"><a class="btn p" href="#/screen">${icon('screen')}${t("MT5 Ekranı'ndan giriş yap")}</a></div></div>`);
    return;
  }
  if (ph.group === 'setup') {
    render(el, html`${head}<div class="card empty"><img class="logo-img" src="${logo}" alt="">
      <h2 style="font-size:18px">${ph.key === 'not_created' ? t('Runtime hazır, henüz başlatılmadı') : t('Kurulum tamamlanmadı')}</h2>
      <p>${srv ? t('Sihirbaz sunucuyu kontrol eder, runtime\'ı SSH ile kurar, sonunda verinin gerçekten geldiğini test eder.') : s.native ? t("Kurulum sihirbazı sistemi kontrol eder, Wine 11'i ve MT5'i hazırlar, sonunda verinin gerçekten geldiğini test eder.") : t("Kurulum sihirbazı sistemi kontrol eder, x86_64 VM'i ve MT5 runtime'ını hazırlar, sonunda verinin gerçekten geldiğini test eder.")}</p>
      <a class="btn p" href="#/setup">${icon('wand')}${t('Kurulum sihirbazını aç')}</a></div>`);
    return;
  }
  if (ph.group === 'off') {
    const vmOff = ph.key === 'engine_down';
    render(el, html`${head}<div class="card empty"><img class="logo-img" src="${logo}" alt="">
      <h2 style="font-size:18px">${t('Runtime durduruldu')}</h2>
      <p>${t('MT5 kurulumu, broker girişi ve indirilen geçmiş veri korunuyor.')} ${vmOff && s.manages_vm ? t('Başlattığında MT5 1–2 dakika içinde kaldığı yerden açılır; önce x86_64 VM açılır (~1 dk).') : t('Başlattığında MT5 1–2 dakika içinde kaldığı yerden açılır.')}</p>
      ${vmOff && !s.manages_vm ? html`<p class="mono">sudo systemctl start docker</p>` : ''}
      <div class="row"><button class="btn p" id="ov-up" ${state.job?.running ? 'disabled' : ''}>${icon('play')}${t('Başlat')}</button>
      ${!vmOff && s.manages_vm ? html`<button class="btn" id="ov-vmoff">${t("VM'i de kapat")}</button>` : ''}</div></div>`);
    $('#ov-up', el).onclick = () => startJob('start').catch(e => toast(String(e), true));
    $('#ov-vmoff', el)?.addEventListener('click', () => startJob('vm_stop').catch(e => toast(String(e), true)));
    return;
  }
  const a = account(), p = probe(), st = s.status;
  const cur = a?.currency || 'USD';
  const steps = stepStates(s);
  const okCount = steps.filter(x => x === 'ok').length;
  const res = state.res;
  const cstats = res?.container;
  const disk = st?.disk;
  const diskUsed = disk?.total_mb ? disk.total_mb - disk.free_mb : null;
  const pnl = a?.profit;
  const margin = a?.margin_level;
  const samples = state.samples;
  const detail = st?.detail || st?.supervisor?.detail;
  const elapsed = st?.supervisor?.updated ? Date.now() / 1000 - st.supervisor.updated : null;
  const running = s.container?.status === 'running';
  const win = s.provider_kind === 'windows';

  render(el, html`
    <div class="head">
      <div><div class="crumbs">SiliconMetaTrader5 › ${where} › <b>${t('Genel Bakış')}</b></div><h1>${t('Genel Bakış')}</h1></div>
      <div class="row wrap">
        <span class="chip lg ${ph.tone}"><span class="d"></span>${ph.label}${a?.trade_mode ? ' · ' + ({ demo: t('Demo hesap'), real: t('Gerçek hesap'), contest: t('Yarışma') }[a.trade_mode] || a.trade_mode) : ''}</span>
        <button class="btn" id="ov-restart" ${running ? '' : 'disabled'}>${icon('refresh')}${t('Yeniden başlat')}</button>
        ${nativeWindow(s) ? html`<button class="btn p" id="ov-focus">${icon('window')}${t('MT5 penceresi')}</button>` : html`<a class="btn p" href="#/screen">${icon('screen')}${t('MT5 Ekranını Aç')}</a>`}
      </div>
    </div>

    ${ph.key === 'error' ? html`<div class="callout bad">${icon('alert')}<div><b>${t('Runtime hata verdi.')}</b> ${detail || ''} <a href="#/logs">${t('Loglara bak')}</a></div></div>` : ''}
    ${ph.key === 'waiting_login' ? html`<div class="callout warn">${icon('info')}<div><b>${t('MT5 açık, broker girişi bekleniyor.')}</b> ${s.native && !win ? th('MT5 penceresinde <i>File › Login to Trade Account</i> ile hesabına gir; geçmiş barlar sonra arka planda iner.') : th("MT5 Ekranı'nda <i>File › Login to Trade Account</i> ile hesabına gir; geçmiş barlar sonra arka planda iner.")}</div></div>` : ''}
    ${['preparing_prefix', 'installing_mt5', 'starting_terminal', 'starting_bridge', 'booting'].includes(ph.key) ? html`<div class="callout info">${icon('spinner', 'spin')}<div><b>${ph.label}…</b> ${detail || ''} ${elapsed ? html`<span class="muted">· ${duration(elapsed)}</span>` : ''}<br><span class="muted">${s.native ? t('MT5 ilk açılışta yaklaşık bir dakika sürer.') : srv || (state.info?.os === 'linux' && state.info?.arch === 'x86_64') ? t('MT5 ilk açılışta birkaç dakika sürebilir.') : t('İlk açılış emülasyon altında birkaç dakika ile yarım saat sürebilir.')}</span></div></div>` : ''}

    <section class="grid g4" aria-label="${t('Hesap özeti')}">
      ${statCard('wallet', t('Bakiye'), a?.balance != null ? money(a.balance, cur) : '—', a?.leverage ? html`<span class="chip neu">1:${a.leverage}</span>` : '')}
      ${statCard('trend', t('Varlık (Equity)'), a?.equity != null ? money(a.equity, cur) : '—', pnl != null ? html`<span class="chip ${pnl >= 0 ? 'up' : 'down'}">${pnl >= 0 ? '+' : ''}${num(pnl)}</span>` : '')}
      ${statCard('clock', t('Serbest teminat'), a?.margin_free != null ? money(a.margin_free, cur) : '—', margin ? html`<span class="chip neu">${t('%{n}', { n: num(margin, 0) })}</span>` : '')}
      ${statCard('pulse', t('Köprü gecikmesi'), p?.latency_ms != null ? `${num(p.latency_ms, p.latency_ms < 10 ? 1 : 0)} ms` : '—', p ? html`<span class="chip ${p.ok ? 'up' : 'down'}">${p.ok ? t('Sağlıklı') : t('Yanıt yok')}</span>` : '')}
    </section>

    <div class="split">
      ${historyCard(a, cur)}
      <section class="card pad aside col" aria-label="${t('Runtime sağlığı')}">
        <div class="row between"><h2>${t('Runtime sağlığı')}</h2><span class="chip ${okCount === STEPS.length ? 'up' : 'neu'}">${okCount}/${STEPS.length}</span></div>
        <div class="col" style="gap:10px">${STEPS.map(([k, label], i) => html`<div class="step"><span class="dot ${steps[i]}">${steps[i] === 'ok' ? icon('check') : steps[i] === 'run' ? icon('spinner', 'spin') : steps[i] === 'bad' ? icon('x') : ''}</span>${win && k === 'prefix' ? t('Windows oturumu açık') : t(label)}</div>`)}</div>
        <div>
          <div class="kv"><span class="muted">${t('Sağlayıcı')}</span><span>${s.provider}</span></div>
          <div class="kv"><span class="muted">${t('Görüntü')}</span><span class="mono">${nativeWindow(s) ? t('macOS penceresi') : `noVNC · 127.0.0.1:${s.display_port}`}</span></div>
          <div class="kv"><span class="muted">${t('Köprü')}</span><span class="mono">rpyc · 127.0.0.1:${s.bridge_port}</span></div>
          <div class="kv"><span class="muted">${win ? 'MT5 (Windows)' : s.native ? 'Wine' : t('İmaj')}</span><span class="mono">${win ? t('Windows (doğrudan)') : s.native ? (st?.supervisor && s.image) || s.image : s.image}</span></div>
          <div class="kv"><span class="muted">${t('CPU çevirisi')}</span><span>${win ? t('yok (Windows)') : s.native && st?.host?.cpu_translation === 'rosetta2' ? 'Rosetta 2 (macOS)' : s.native ? t('yok (yerel x86_64)') : st?.host?.cpu_translation === 'none' || !st?.host?.cpu_translation ? t('yok (tam VM)') : st.host.cpu_translation}</span></div>
        </div>
      </section>
    </div>

    <div class="split">
      <section class="card pad main col" aria-label="${t('Hesap')}">
        <div class="row between"><h2>${t('Hesap ve terminal')}</h2>${p?.checked_at ? html`<span class="muted" style="font-size:12px">${t('son yoklama {time}', { time: clock(p.checked_at) })}</span>` : ''}</div>
        ${a ? html`<div class="grid g3" style="gap:0 24px">
          <div><div class="kv"><span class="muted">${t('Sunucu')}</span><span>${a.server || '—'}</span></div>
            <div class="kv"><span class="muted">${t('Şirket')}</span><span>${a.company || '—'}</span></div>
            <div class="kv"><span class="muted">${t('Hesap')}</span><span class="mono">${a.login_masked || '—'}</span></div></div>
          <div><div class="kv"><span class="muted">${t('Açık pozisyon')}</span><span class="num">${p?.positions ?? '—'}</span></div>
            <div class="kv"><span class="muted">${t('Bekleyen emir')}</span><span class="num">${p?.orders ?? '—'}</span></div>
            <div class="kv"><span class="muted">${t('Teminat')}</span><span class="num">${a.margin != null ? money(a.margin, cur) : '—'}</span></div></div>
          <div><div class="kv"><span class="muted">Terminal</span><span>${p?.terminal?.connected ? t('bağlı') : t('bağlı değil')}${p?.terminal?.build ? ' · build ' + p.terminal.build : ''}</span></div>
            <div class="kv"><span class="muted">Algo trading</span><span>${p?.terminal?.trade_allowed ? t('açık') : t('kapalı')}</span></div>
            <div class="kv"><span class="muted">MaxBars</span><span class="num">${st?.mt5?.maxbars ?? '—'}</span></div></div></div>`
          : html`<p class="muted" style="margin:0;font-size:13px">${p?.error ? t('Köprü yanıtı: {error}', { error: p.error }) : t('Broker girişi yapılınca hesap bilgileri burada görünür.')}</p>`}
      </section>
      <section class="card pad aside col" aria-label="${t('Kaynak kullanımı')}">
        <div class="row between"><h2>${t('Kaynak kullanımı')}</h2><span class="muted" style="font-size:12px">${res?.vm ? `${res.vm.cpus} CPU · ${bytes(res.vm.memory)} VM` : win ? t('MT5 süreçleri') : s.native ? t('Wine süreçleri') : 'container'}</span></div>
        ${cstats ? meter(win ? t('CPU · MT5 (çekirdek başına %)') : s.native ? t('CPU · Wine (çekirdek başına %)') : 'CPU · container', t('%{n}', { n: num(cstats.cpu_percent, 0) }), cstats.cpu_percent / Math.max(1, res?.vm?.cpus || 1)) : meter(win ? 'CPU · MT5' : s.native ? 'CPU · Wine' : 'CPU · container', '—', 0)}
        ${cstats ? meter(t('Bellek'), `${bytes(cstats.mem_used_bytes)} / ${bytes(cstats.mem_limit_bytes)}`, cstats.mem_percent) : meter(t('Bellek'), '—', 0)}
        ${disk?.total_mb ? meter(srv ? t('Disk · Sunucu') : res?.vm ? 'Disk · VM' : s.native ? (state.info?.os === 'linux' ? 'Disk · Linux' : 'Disk · Mac') : 'Disk · Docker', `${bytes(diskUsed * 1048576)} / ${bytes(disk.total_mb * 1048576)}`, (diskUsed / disk.total_mb) * 100) : meter('Disk', '—', 0)}
        <div style="border-top:1px solid var(--line-2);padding-top:14px">${latencySpark(samples)}</div>
      </section>
    </div>`);

  $('#ov-range', el)?.addEventListener('click', e => {
    const b = e.target.closest('button'); if (!b || b.dataset.v === range) return;
    range = b.dataset.v; try { localStorage.setItem('ov-range', range); } catch { /* private */ }
    hist = null; draw(); loadHistory(true);
  });
  if (hist) bindHistoryChart(hist, cur);
  $('#ov-restart', el)?.addEventListener('click', async () => {
    const v = await modal({
      title: t('Neyi yeniden başlatalım?'),
      body: html`<p class="muted" style="margin:0">${t("MT5'i yeniden başlatmak köprüyü de yeniden başlatır (~1 dk).")} ${win ? t("Runtime yeniden başlatma MT5'i de baştan açar.") : s.native ? t("Runtime yeniden başlatma Wine'ı da baştan açar.") : t("Container yeniden başlatma Wine'ı da baştan açar.")}</p>`,
      buttons: [{ label: t('Vazgeç'), value: null }, { label: s.native ? t('Tüm runtime') : 'Container', value: 'container' }, { label: t('Yalnız köprü'), value: 'bridge' }, { label: t('MT5 + köprü'), value: 'mt5', primary: true }],
    });
    if (!v) return;
    try { await call('restart', { target: v }); toast(v === 'container' ? t('Runtime yeniden başlatıldı') : t('Yeniden başlatma istendi')); }
    catch (e) { toast(String(e), true); }
  });
  $('#ov-focus', el)?.addEventListener('click', () => call('focus_mt5').catch(e => toast(String(e), true)));
}

export default {
  mount(node) { el = node; draw(); loadHistory(true); },
  update(what) { if (what === 'snap' || what === 'res') { draw(); if (what === 'snap') loadHistory(); } },
  unmount() { el = null; },
};
