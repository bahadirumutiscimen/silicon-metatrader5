import { call } from '../api.js';
import { $, $$, html, render, icon, bytes, clock, toast, modal } from '../dom.js';
import { state, startJob, waitJob, isRunning, phaseOf, targetName } from '../store.js';
import { t, th, locale } from '../i18n.js';

let el, inv = null, server = null, sel = new Map(), loading = false, error = null;
// sel: symbol -> Set(years)

const sum = (arr, f) => arr.reduce((a, x) => a + (f(x) || 0), 0);
const isDefault = name => name.toLowerCase() === 'default';

async function load() {
  loading = true; error = null; draw();
  try {
    inv = await call('inventory');
    if (!server || !inv.servers.some(s => s.name === server)) {
      const real = inv.servers.filter(s => !isDefault(s.name)).sort((a, b) => b.bytes - a.bytes);
      server = (real[0] || inv.servers[0])?.name || null;
    }
  } catch (e) { error = String(e); }
  loading = false; draw();
}

function current() { return inv?.servers.find(s => s.name === server) || null; }

function selectionStats(srv) {
  let files = 0, size = 0;
  for (const [sym, years] of sel) {
    const h = srv.history.find(x => x.symbol === sym);
    if (!h) continue;
    for (const y of years) { if (h.years[y] != null) { files++; size += h.years[y]; } }
  }
  return { files, size, symbols: [...sel.values()].filter(s => s.size).length };
}

function itemsFromSelection() {
  return [...sel].filter(([, ys]) => ys.size).map(([symbol, ys]) => ({ kind: 'history', server, symbol, years: [...ys].map(Number).sort() }));
}

async function purgeFlow(items, what) {
  let plan;
  try { plan = await call('purge', { spec: { items, dry_run: true } }); }
  catch (e) { return toast(String(e), true); }
  if (!plan.files) return toast(t('Seçime uyan dosya yok'));
  const ok = await modal({
    title: t('{what} silinsin mi?', { what }),
    body: html`<div class="callout warn">${icon('alert')}<div>${th('<b>{files} dosya · {size}</b> silinecek. MT5 güvenle durdurulur, dosyalar silinir, MT5 ve köprü yeniden başlar (~1 dk).', { files: plan.files, size: bytes(plan.bytes) })}</div></div>
      <pre class="code" style="max-height:160px">${(plan.sample || []).join('\n')}${plan.files > (plan.sample || []).length ? '\n…' : ''}</pre>
      <p class="muted" style="margin:0;font-size:13px">${t("Silinen aralık gerekirse broker'dan yeniden indirilir.")}</p>`,
    buttons: [{ label: t('Vazgeç'), value: false }, { label: t('Sil'), value: true, primary: true, danger: true }],
  });
  if (!ok) return;
  toast(t('Siliniyor — MT5 bakım moduna alındı…'));
  try {
    const r = await call('purge', { spec: { items, dry_run: false } });
    toast(t('{n} dosya silindi · {size} boşaldı', { n: r.removed ?? 0, size: bytes(r.freed_bytes) }) + (r.failed?.length ? ' · ' + t('{n} hata', { n: r.failed.length }) : ''), !!r.failed?.length);
    sel.clear();
  } catch (e) { toast(String(e), true); }
  load();
}

function monthKey(offset) {
  const d = new Date(); d.setDate(1); d.setMonth(d.getMonth() - offset);
  return d.getFullYear() * 100 + d.getMonth() + 1;
}

function draw() {
  if (!el) return;
  const srv = current();
  const disk = state.snap?.status?.disk;
  const servers = inv?.servers || [];
  const hist = sum(servers, s => s.history_bytes), ticks = sum(servers, s => s.ticks_bytes), cache = sum(servers, s => s.cache_bytes);
  const years = srv ? [...new Set(srv.history.flatMap(h => Object.keys(h.years)))].sort() : [];
  const ticksBy = new Map((srv?.ticks || []).map(t => [t.symbol, t]));
  const symbols = srv ? [...new Set([...srv.history.map(h => h.symbol), ...srv.ticks.map(t => t.symbol)])] : [];
  const rows = symbols.map(sym => ({ sym, h: srv.history.find(x => x.symbol === sym), t: ticksBy.get(sym) }))
    .sort((a, b) => ((b.h?.bytes || 0) + (b.t?.bytes || 0)) - ((a.h?.bytes || 0) + (a.t?.bytes || 0)));
  const st = srv ? selectionStats(srv) : { files: 0, size: 0, symbols: 0 };
  const maxYear = Math.max(1, ...(srv?.history || []).flatMap(h => Object.values(h.years)));
  const maxbars = state.snap?.status?.mt5?.maxbars;
  const keepMonth = monthKey(2);
  const diskPct = disk?.total_mb && inv ? ((inv.total_bytes / (disk.total_mb * 1048576)) * 100).toFixed(1) : null;

  render(el, html`
    <div class="head">
      <div><div class="crumbs">SiliconMetaTrader5 › ${targetName() || t('Lokal runtime')} › <b>${t('Veri Yönetimi')}</b></div><h1>${t('Veri Yönetimi')}</h1>
        <div class="mono muted" style="margin-top:4px">${inv?.root || ''}</div></div>
      <div class="row wrap">
        <a class="btn" href="#/datatest">${icon('pulse')}${t('Veri testi')}</a>
        <button class="btn" id="dm-scan" ${loading ? 'disabled' : ''}>${icon('refresh', loading ? 'spin' : '')}${t('Yeniden tara')}</button>
      </div>
    </div>
    ${error ? (isRunning() ? html`<div class="callout bad">${icon('alert')}<div>${error}</div></div>`
      : html`<div class="callout info">${icon('info')}<div><b>${t('Runtime {phase}.', { phase: phaseOf().label.toLocaleLowerCase(locale()) })}</b> ${(state.snap?.target || 'local') !== 'local' ? t("MT5 verisi sunucuda duruyor; envanter ve silme için runtime'ı başlat.") : state.snap?.native ? (state.info?.os === 'macos' ? t("MT5 verisi Mac yerel modun Wine prefix'inde duruyor; envanter ve silme için runtime'ı başlat.") : t("MT5 verisi Linux yerel modun Wine prefix'inde duruyor; envanter ve silme için runtime'ı başlat.")) : t("MT5 verisi container'ın volume'ünde duruyor; envanter ve silme için runtime'ı başlat.")}</div></div>`) : ''}
    ${inv && !inv.exists ? html`<div class="callout info">${icon('info')}<div>${t('MT5 veri klasörü henüz yok. MT5 ilk açılışta oluşturur.')}</div></div>` : ''}

    <section class="grid g4">
      <div class="card stat"><div class="ico">${icon('db')}</div><div><div class="muted" style="font-size:13px">${t('MT5 veri klasörü')}</div>
        <div class="row" style="margin-top:4px"><span class="v">${inv ? bytes(inv.total_bytes) : '—'}</span>${disk?.total_mb && inv ? html`<span class="chip neu">${(state.snap?.target || 'local') !== 'local' ? t('sunucu diskinin %{n}', { n: diskPct }) : state.snap?.native ? (state.info?.os === 'macos' ? t('Mac diskinin %{n}', { n: diskPct }) : t('Linux diskinin %{n}', { n: diskPct })) : state.snap?.manages_vm ? t('VM diskinin %{n}', { n: diskPct }) : t('Docker diskinin %{n}', { n: diskPct })}</span>` : ''}</div>
        <div class="muted" style="font-size:12px;margin-top:2px">${t('{n} sunucu', { n: servers.length })}${inv ? ' · ' + t('tarama {time}', { time: clock(inv.scanned_at) }) : ''}</div></div></div>
      <div class="card stat"><div class="ico">${icon('bars')}</div><div><div class="muted" style="font-size:13px">${t('Bar geçmişi')}</div><div class="v" style="margin-top:4px">${inv ? bytes(hist) : '—'}</div><div class="muted mono">history/*.hcc</div></div></div>
      <div class="card stat"><div class="ico">${icon('pulse')}</div><div><div class="muted" style="font-size:13px">${t('Tick verisi')}</div><div class="v" style="margin-top:4px">${inv ? bytes(ticks) : '—'}</div><div class="muted mono">ticks/*.tkc · ticks.dat</div></div></div>
      <div class="card stat"><div class="ico">${icon('cpu')}</div><div><div class="muted" style="font-size:13px">${t('Tester · önbellek')}</div><div class="v" style="margin-top:4px">${inv ? bytes(inv.tester_bytes + cache) : '—'}</div><div class="muted" style="font-size:12px">${t('tester {tester} · önbellek {cache}', { tester: inv ? bytes(inv.tester_bytes) : '—', cache: inv ? bytes(cache) : '—' })}</div></div></div>
    </section>

    <div class="split">
      <section class="card main" style="padding:18px 6px 6px">
        <div class="row between wrap" style="padding:0 12px 12px;gap:10px">
          <h2>${t('Sembol bazında geçmiş')}</h2>
          <div class="row wrap">
            ${servers.length > 1 ? html`<div class="seg" id="dm-srv">${servers.map(s => html`<button data-v="${s.name}" class="${s.name === server ? 'sel' : ''}" title="${isDefault(s.name) ? t('MT5 varsayılan sunucusu, açılışta yeniden oluşturulur') : ''}">${isDefault(s.name) ? 'Default (MT5)' : s.name}</button>`)}</div>` : ''}
            ${years.length ? html`<select class="f" id="dm-before" style="height:32px;font-size:13px"><option value="">${t('Şundan eskisini seç…')}</option>${years.slice(1).map(y => html`<option value="${y}">${t('{year} öncesi', { year: y })}</option>`)}<option value="all">${t('Tüm yıllar')}</option></select>` : ''}
          </div>
        </div>
        ${!srv ? html`<div class="empty"><p>${loading ? t('Taranıyor…') : t('Henüz geçmiş veri yok. Broker girişinden sonra MT5 barları indirir.')}</p></div>` : html`
        <div style="overflow-x:auto"><table>
          <thead><tr><th>${t('Sembol')}</th><th><div class="row between" style="width:${years.length * 30}px;min-width:120px"><span>${years[0] || ''}</span><span style="font-weight:400">${t('bar yılları · tıkla')}</span><span>${years[years.length - 1] || ''}</span></div></th><th class="r">${t('Bar')}</th><th class="r">${t('Tick')}</th><th class="r">${t('Silinecek')}</th></tr></thead>
          <tbody>${rows.map(({ sym, h, t: tk }) => {
            const ys = sel.get(sym) || new Set();
            const del = h ? [...ys].reduce((a, y) => a + (h.years[y] || 0), 0) : 0;
            return html`<tr><td style="font-weight:600">${sym}</td>
              <td><div class="years">${years.map(y => {
                const b = h?.years[y];
                return b == null ? html`<span class="yr none" title="${t('{year}: yok', { year: y })}"></span>`
                  : html`<button class="yr ${ys.has(y) ? 'sel' : ''}" data-sym="${sym}" data-y="${y}" title="${y} · ${bytes(b)}" aria-label="${sym} ${y} ${bytes(b)}" style="--a:${(0.25 + 0.75 * (b / maxYear)).toFixed(2)}"></button>`;
              })}</div></td>
              <td class="r num">${h ? bytes(h.bytes - (h.cache_bytes || 0)) : '—'}</td><td class="r num">${tk ? bytes(tk.bytes) : '—'}</td><td class="r num ${del ? 'neg' : 'muted'}">${del ? bytes(del) : '—'}</td></tr>`;
          })}</tbody></table></div>
        <div class="row between wrap" style="padding:12px;border-top:1px solid var(--line);gap:10px">
          <span class="legend"><span><i style="background:var(--accent)"></i>${t('mevcut (koyu = büyük)')}</span><span><i style="background:repeating-linear-gradient(135deg,var(--neg) 0 3px,var(--down-bg) 3px 6px)"></i>${t('silinecek')}</span></span>
          <div class="row wrap" style="justify-content:flex-end"><span class="muted" style="font-size:13px">${st.files ? t('{symbols} sembol · {files} yıl · ~{size}', { symbols: st.symbols, files: st.files, size: bytes(st.size) }) : t('Seçim yok')}</span><button class="btn sm" id="dm-clear" ${st.files ? '' : 'disabled'}>${t('Seçimi temizle')}</button>
            <button class="btn sm danger p" id="dm-del" ${st.files ? '' : 'disabled'}>${icon('trash')}${t('Seçilenleri sil…')}</button></div>
        </div>`}
      </section>

      <aside class="aside col">
        <section class="card pad col">
          <h2>${t('Silme nasıl çalışır?')}</h2>
          <div class="col" style="gap:10px;font-size:13px">
            <div class="step"><span class="dot run">1</span>${t('Önce kuru çalıştırma: silinecek dosyalar listelenir')}</div>
            <div class="step"><span class="dot run">2</span>${t('MT5 güvenle durdurulur, dosyalar silinir')}</div>
            <div class="step"><span class="dot run">3</span>${t('MT5 ve köprü yeniden başlar (~1 dk)')}</div>
          </div>
          <p class="muted" style="margin:0;font-size:12px">${t("Silinen aralık gerekirse broker'dan yeniden indirilir.")}</p>
        </section>
        <section class="card pad col">
          <h2>${t('Hızlı temizlik')}</h2>
          ${srv ? html`
          <div class="row between"><div><b style="font-size:13px">${t('Önbellek')}</b><div class="muted" style="font-size:12px">${srv.name} · ${t("üst zaman dilimleri M1'den yeniden üretilir")}</div></div><button class="btn sm" id="dm-cache" ${srv.cache_bytes ? '' : 'disabled'}>${bytes(srv.cache_bytes)}</button></div>
          <div class="row between"><div><b style="font-size:13px">${t('Eski tick verisi')}</b><div class="muted" style="font-size:12px">${t('Son 3 ay dışındaki aylık .tkc dosyaları')}</div></div><button class="btn sm" id="dm-ticks" ${srv.ticks_bytes ? '' : 'disabled'}>${t('Temizle')}</button></div>
          <div class="row between"><div><b style="font-size:13px">${t('Sunucunun tüm verisi')}</b><div class="muted" style="font-size:12px">${isDefault(srv.name) ? t('Default · MT5 açılışta küçük haliyle yeniden oluşturur') : srv.name + ' · ' + t('bar, tick, önbellek')}</div></div><button class="btn sm danger" id="dm-server">${bytes(srv.bytes)}</button></div>` : ''}
          <div class="row between"><div><b style="font-size:13px">Strategy Tester</b><div class="muted" style="font-size:12px">${t('Ajan önbelleği ve test geçmişi')}</div></div><button class="btn sm" id="dm-tester" ${inv?.tester_bytes ? '' : 'disabled'}>${inv ? bytes(inv.tester_bytes) : '—'}</button></div>
        </section>
        <section class="card pad col">
          <h2>${t('Donma koruması')}</h2>
          <p class="muted" style="margin:0;font-size:13px">${th('MaxBars şu an <b class="num">{n}</b>. Emülasyon altında senkron sırasında donmaların en sık nedeni yüksek MaxBars.', { n: maxbars ?? '—' })}</p>
          <label class="fl">${t('Grafik başına MaxBars')}<select class="f" id="dm-maxbars">${[50000, 100000, 250000, 500000, 1000000].map(v => html`<option value="${v}" ${String(v) === String(maxbars) ? 'selected' : ''}>${v === 100000 ? t('{n} (önerilen)', { n: v.toLocaleString(locale()) }) : v.toLocaleString(locale())}</option>`)}</select></label>
        </section>
      </aside>
    </div>`);

  $('#dm-scan', el).onclick = load;
  $('#dm-srv', el)?.addEventListener('click', e => { const b = e.target.closest('button'); if (!b) return; server = b.dataset.v; sel.clear(); draw(); });
  $$('.yr', el).forEach(b => b.onclick = () => {
    const ys = sel.get(b.dataset.sym) || new Set();
    ys.has(b.dataset.y) ? ys.delete(b.dataset.y) : ys.add(b.dataset.y);
    sel.set(b.dataset.sym, ys); draw();
  });
  $('#dm-before', el)?.addEventListener('change', e => {
    const v = e.target.value; if (!v || !srv) return;
    sel.clear();
    srv.history.forEach(h => sel.set(h.symbol, new Set(Object.keys(h.years).filter(y => v === 'all' || y < v))));
    draw();
  });
  $('#dm-clear', el)?.addEventListener('click', () => { sel.clear(); draw(); });
  $('#dm-del', el)?.addEventListener('click', () => purgeFlow(itemsFromSelection(), t('Seçilen bar yılları')));
  $('#dm-cache', el)?.addEventListener('click', () => purgeFlow([{ kind: 'cache', server }], t('{server} önbelleği', { server })));
  $('#dm-ticks', el)?.addEventListener('click', () => purgeFlow([{ kind: 'ticks', server, before_month: keepMonth }], t('Son 3 ay dışındaki tick verisi')));
  $('#dm-server', el)?.addEventListener('click', () => purgeFlow([{ kind: 'history', server, all: true }, { kind: 'ticks', server, all: true }, { kind: 'cache', server }], t('{server} sunucusunun tüm verisi', { server })));
  $('#dm-tester', el)?.addEventListener('click', () => purgeFlow([{ kind: 'tester' }], t('Strategy Tester verisi')));
  $('#dm-maxbars', el)?.addEventListener('change', async e => {
    try {
      const changed = await call('save_settings', { values: { SMT5_MAXBARS: e.target.value } });
      if (!changed.length) return;
      const go = await modal({ title: t('MaxBars kaydedildi'), body: html`<p class="muted" style="margin:0">${t("Yeni değer runtime yeniden başlayınca MT5'e yazılır (veriler korunur).")}</p>`, buttons: [{ label: t('Sonra'), value: false }, { label: t('Şimdi uygula'), value: true, primary: true }] });
      if (go && state.snap?.native) { await waitJob(await startJob('stop')); await startJob('start'); } else if (go) await startJob('up');
    } catch (err) { toast(String(err), true); }
  });
}

export default {
  mount(node) { el = node; sel.clear(); load(); },
  update() {},
  unmount() { el = null; },
};
