import { call } from '../api.js';
import { $, $$, html, render, icon, bytes, toast, modal, copyText } from '../dom.js';
import { state, startJob, waitJob, refreshInfo } from '../store.js';
import { modeCards } from '../modes.js';
import { t, th, locale } from '../i18n.js';

let el, cfg = null, draft = {}, dirty = false, lastActual = '', modes = [];
const actualKey = () => JSON.stringify([state.res?.vm, ['maxbars', 'algo', 'dll'].map(mt5Now)]);

// texts translated with t() at render
const PROVIDERS = [
  ['mac-colima', 'macOS · Apple Silicon / Intel', 'Colima VM · QEMU x86_64 tam emülasyon'],
  ['linux-docker', 'Linux · x86_64', 'Docker Engine · native, emülasyon yok'],
  ['remote-ssh', 'Uzak Linux sunucu', 'SSH docker context · sunucu/VPS'],
];

async function load() {
  call('modes').then(m => { modes = m; draw(); }).catch(() => { modes = []; });
  try { cfg = await call('settings'); draft = { ...cfg.values }; dirty = false; } catch (e) { toast(String(e), true); }
  draw();
}

function field(key, label, attrs = '', hint = '') {
  return html`<label class="fl">${label}<input class="f" data-k="${key}" value="${draft[key] ?? ''}" ${attrs ? { __raw: true, s: attrs } : ''}>${hint ? html`<small>${hint}</small>` : ''}</label>`;
}

/** "MT5'te şu an" line: what the terminal really uses (terminal_info via the probe). */
function mt5Now(what) {
  const term = state.snap?.status?.bridge?.probe?.terminal;
  if (!term || !state.snap?.status?.bridge?.probe?.ok) return null;
  if (what === 'maxbars') return term.maxbars;
  if (what === 'algo') return term.trade_allowed ? '1' : '0';
  if (what === 'dll') return term.dlls_allowed ? '1' : '0';
  return null;
}

function actual(what) {
  const v = mt5Now(what);
  if (v == null) return html`<small>${t('MT5 çalışınca gerçek değer burada görünür')}</small>`;
  const want = what === 'maxbars' ? Number(cfg.values.SMT5_MAXBARS) : cfg.values[what === 'algo' ? 'SMT5_ALGO_TRADING' : 'SMT5_DLL_IMPORT'];
  const same = what === 'maxbars' ? Number(v) === want : v === want;
  const shown = what === 'maxbars' ? Number(v).toLocaleString(locale()) : v === '1' ? t('açık') : t('kapalı');
  return html`<small>${th("MT5'te şu an: <b>{v}</b>", { v: shown })} ${same ? html`<span class="chip up" style="height:18px">${t('eşleşiyor')}</span>` : html`<span class="chip warn" style="height:18px" title="${t("Kaydedilen değer MT5'e henüz uygulanmadı ya da MT5 içinden değiştirildi")}">${t('farklı')}</span> <button class="btn sm" data-apply-mt5 title="${t('MT5 bu ayarları yalnız açılışta okur; MT5 yeniden başlatılır (~20 sn), broker girişi korunur.')}">${icon('refresh')}${t("MT5'e uygula")}</button>`}</small>`;
}

function toggle(key, label, hint, what) {
  const on = draft[key] === '1';
  return html`<div class="kv" style="align-items:center"><div><b style="font-size:13px">${label}</b><div class="muted" style="font-size:12px">${hint}</div>${actual(what)}</div>
    <label class="switch"><input type="checkbox" data-sw="${key}" ${on ? 'checked' : ''}><span></span></label></div>`;
}


function draw() {
  if (!el || !cfg) return;
  const s = state.snap, vm = state.res?.vm;
  const kind = s?.provider_kind;
  const nat = !!s?.native;
  const srv = (s?.target || 'local') !== 'local';   // a server is the target
  const novnc = s?.display === 'novnc';             // screen through noVNC (server, headless, Docker)
  render(el, html`
    <div class="head">
      <div><div class="crumbs">SiliconMetaTrader5 › ${srv ? s.target_name : t('Bu bilgisayar')} › <b>${t('Runtime Ayarları')}</b></div><h1>${t('Runtime Ayarları')}</h1>
        <div class="mono muted" style="margin-top:4px">${cfg.env_path}${cfg.env_exists ? '' : ' ' + t('(henüz yok — kaydedince oluşur)')}</div></div>
      <div class="row wrap">
        <button class="btn" id="st-revert" ${dirty ? '' : 'disabled'}>${t('Geri al')}</button>
        <button class="btn p" id="st-save" ${dirty ? '' : 'disabled'}>${t('Kaydet ve uygula')}</button>
      </div>
    </div>

    <div class="split">
      <div class="main col">
        ${srv ? html`<section class="card pad col">
          <div><h2>${t('Sunucu')}</h2><div class="muted" style="font-size:13px;margin-top:4px">${t('{name} · {mode}. Bu sayfadaki değerler sunucunun kendi .env dosyasına yazılır; "Kaydet ve uygula" runtime\'ı sunucuda yeniden başlatır.', { name: s.target_name, mode: s.mode_label })}</div></div>
        </section>` : html`<section class="card pad col">
          ${modes.length > 1 ? html`
          <div><h2>${t('Çalışma modu')}</h2><div class="muted" style="font-size:13px;margin-top:4px">${t("MT5'i çalıştıran katman. Her modun kendi verisi var; mod değiştirmek diğerininkini silmez. Mac yerel moda ilk geçişte MT5, broker girişi ve geçmişiyle birlikte Güvenli moddan kopyalanır. Hızları Veri Testi → Hız testi ile karşılaştır.")}</div></div>
          ${modeCards(modes, { disabled: !!state.job?.running })}`
          : html`
          <div><h2>${t('Host sağlayıcı')}</h2><div class="muted" style="font-size:13px;margin-top:4px">${t('Container aynı kalır; değişen yalnızca onu çalıştıran katman. Uygulama sistemini otomatik algılar.')}</div></div>
          <div class="grid" style="grid-template-columns:repeat(auto-fit,minmax(230px,1fr))">${PROVIDERS.map(([k, name, d]) => html`<div class="choice ${k === kind ? 'sel' : ''}"><div class="grow"><b style="font-size:13px">${t(name)}</b><div class="muted" style="font-size:12px">${t(d)}</div></div>${k === kind ? html`<span class="chip up">${t('Algılandı')}</span>` : ''}</div>`)}</div>`}
          ${vm ? html`<div class="grid g3" style="gap:8px">
            <div class="kv" style="border:0"><span class="muted">CPU</span><b>${t('{n} çekirdek', { n: vm.cpus })}</b></div>
            <div class="kv" style="border:0"><span class="muted">${t('Bellek')}</span><b>${bytes(vm.memory)}</b></div>
            <div class="kv" style="border:0"><span class="muted">Disk</span><b>${bytes(vm.disk)}</b></div></div>
            <div class="row wrap"><span class="muted" style="font-size:12px">${t("VM profili '{name}' · {arch} · {status}. Kaynak değişikliği için:", { name: vm.name, arch: vm.arch, status: vm.status })} <span class="mono">smt5 vm start --cpus 6 --memory 12 --disk 80</span></span></div>` : ''}
        </section>`}

        ${nat && !novnc ? '' : html`<section class="card pad col">
          <div><h2>${t('Görüntü')}</h2><div class="muted" style="font-size:13px;margin-top:4px">${t('MT5 masaüstü noVNC ile uygulamanın içinde açılır.')}</div></div>
          <div class="grid g3">
            ${field('SMT5_RESOLUTION', t('Çözünürlük'), 'list="st-res"', t('Sanal ekran boyutu'))}
            <datalist id="st-res"><option>1366x768</option><option>1500x770</option><option>1600x900</option><option>1920x1080</option></datalist>
            ${kind === 'windows' ? '' : html`<label class="fl">${t('Yazı ve düğme boyutu')}<select class="f" data-k="SMT5_DPI">${[['72', '75 %'], ['96', '100 %'], ['120', '125 %'], ['144', '150 %']].map(([v, l]) => html`<option value="${v}" ${(draft.SMT5_DPI || '96') === v ? 'selected' : ''}>${l}${v === '96' ? ' ' + t('(önerilen)') : ''}</option>`)}</select></label>`}
          </div>
        </section>`}

        <section class="card pad col">
          <div><h2>MetaTrader 5</h2><div class="muted" style="font-size:13px;margin-top:4px">${nat ? t('mt5cfg.ini başlangıç ayarları.') : t('mt5cfg.ini ve container ortamı.')}</div></div>
          <div class="grid g3">
            ${nat ? '' : field('TZ', t('Saat dilimi'), 'list="st-tz"', t("Broker'ın sunucu saatini değiştirmez"))}
            <datalist id="st-tz"><option>Europe/Istanbul</option><option>UTC</option><option>Europe/London</option><option>America/New_York</option></datalist>
            <label class="fl">MaxBars<select class="f" data-k="SMT5_MAXBARS">${['50000', '100000', '250000', '500000', '1000000'].map(v => html`<option value="${v}" ${draft.SMT5_MAXBARS === v ? 'selected' : ''}>${Number(v).toLocaleString(locale())}${v === '100000' ? ' ' + t('(önerilen)') : ''}</option>`)}</select>${actual('maxbars')}</label>
            <label class="fl">${t('Log boyutu sınırı')}<select class="f" data-k="SMT5_LOG_MAX_MB">${['5', '10', '25', '50', '100'].map(v => html`<option value="${v}" ${draft.SMT5_LOG_MAX_MB === v ? 'selected' : ''}>${v} MB / log${v === '10' ? ' ' + t('(önerilen)') : ''}</option>`)}</select><small>${t('Her log bu boyutu aşınca döndürülür; en fazla bir eski kopya tutulur')}</small></label>
          </div>
          <div class="col" style="gap:0">
            ${toggle('SMT5_ALGO_TRADING', 'Algo trading', t("MT5'in Algo Trading düğmesi ve EA'ların işlem açması"), 'algo')}
            ${toggle('SMT5_DLL_IMPORT', t('DLL içe aktarma'), t("EA'ların DLL yüklemesine izin (MT5 varsayılanı kapalı)"), 'dll')}
          </div>
          <p class="muted" style="margin:0;font-size:12px">${th(`Bu değerler MT5'e başlangıç yapılandırmasıyla (<span class="mono">mt5cfg.ini</span>) yazılır ve yalnızca değiştiklerinde uygulanır; sonra MT5 içinden değiştirdiklerin korunur. "MT5'te şu an" satırı terminalin gerçekten kullandığı değeri gösterir.`)}</p>
        </section>

        <section class="card pad col">
          <div><h2>${t('Ağ ve güvenlik')}</h2><div class="muted" style="font-size:13px;margin-top:4px">${t('Köprü kimlik doğrulamasız kod çalıştırabildiği için portlar yalnızca bu bilgisayara (127.0.0.1) açılır.')}</div></div>
          <div class="grid g3">
            ${nat && !novnc ? '' : field('SMT5_DISPLAY_PORT', t('Görüntü portu'), 'inputmode="numeric"')}
            ${field('SMT5_BRIDGE_PORT', t('Köprü portu'), 'inputmode="numeric"')}
            ${field('SMT5_STATUS_PORT', t('Durum portu'), 'inputmode="numeric"')}
          </div>
          ${nat ? '' : html`<div class="kv"><div><b>${t('Masaüstü şifresi')}</b><div class="muted" style="font-size:12px">${t('noVNC masaüstü için')}</div></div>
            <div class="row">${cfg.has_password ? html`<span class="chip up">${t('Ayarlı')}</span><button class="btn sm" id="st-pw-copy">${icon('copy')}${t('Kopyala')}</button>` : html`<span class="chip down">${t('Boş')}</span>`}${srv ? '' : html`<button class="btn sm" id="st-pw-new">${icon('key')}${t('Yeni şifre')}</button>`}</div></div>`}
          <div class="kv"><div><b>API token</b><div class="muted" style="font-size:12px">${t('Veri silme ve yeniden başlatma için gerekli')}</div></div>
            <div class="row">${cfg.has_token ? html`<span class="chip up">${t('Ayarlı')}</span>` : html`<span class="chip down">${t('Boş')}</span>`}${srv ? '' : html`<button class="btn sm" id="st-tk-new">${icon('key')}${cfg.has_token ? t('Yeniden üret') : t('Oluştur')}</button>`}</div></div>
          ${srv ? html`<p class="muted" style="margin:0;font-size:12px">${t('Sunucunun portları yalnız sunucuda 127.0.0.1\'e açılır; bu bilgisayardan SSH tüneliyle erişilir. Şifre ve token sunucuda üretilir.')}</p>` : ''}
        </section>
      </div>

      <aside class="aside col">
        <section class="card pad col">
          <h2>${t('Değişiklikler nasıl uygulanır?')}</h2>
          <p class="muted" style="margin:0;font-size:13px;line-height:1.55">${srv && state.snap?.provider_kind === 'windows'
            ? th(`Ayarlar sunucudaki <span class="mono">{path}</span> dosyasına yazılır. "Kaydet ve uygula" runtime'ı sunucuda durdurur, ayarları uygular (MT5 ayarları, ekran portu) ve yeniden başlatır; MT5, broker girişi ve geçmiş veri sunucuda kalır.`, { path: '%LOCALAPPDATA%\\siliconmetatrader5\\runtime\\.env' })
            : srv
            ? th(`Ayarlar sunucudaki <span class="mono">~/siliconmetatrader5/runtime/.env</span> dosyasına yazılır. "Kaydet ve uygula" runtime'ı sunucuda yeniden başlatır; MT5, broker girişi ve geçmiş veri sunucuda kalır.`)
            : nat
            ? th(`Ayarlar <span class="mono">runtime/.env</span> dosyasına yazılır. "Kaydet ve uygula" runtime'ı durdurup yeniden başlatır; Wine prefix'i, broker girişi ve geçmiş veri <span class="mono">~/Library/Application Support/SiliconMT5</span> içinde kalır.`)
            : th(`Ayarlar <span class="mono">runtime/.env</span> dosyasına yazılır. "Kaydet ve uygula" container'ı yeni değerlerle yeniden oluşturur; Wine prefix'i, broker girişi ve geçmiş veri volume'de kalır. Görüntü altyapısı değişirse önce imaj derlenir.`)}</p>
        </section>
      </aside>
    </div>`);

  $$('input[data-k], select[data-k]', el).forEach(i => i.addEventListener('input', () => { draft[i.dataset.k] = i.value.trim(); markDirty(); }));
  $$('input[data-sw]', el).forEach(i => i.addEventListener('change', () => { draft[i.dataset.sw] = i.checked ? '1' : '0'; markDirty(); }));
  $('#st-revert', el).onclick = () => { draft = { ...cfg.values }; dirty = false; draw(); };
  $('#st-save', el).onclick = save;
  $$('[data-mode]', el).forEach(b => b.onclick = () => switchMode(modes.find(m => m.id === b.dataset.mode)));
  $('#st-pw-copy', el)?.addEventListener('click', async () => { const p = await call('reveal_password'); if (p) copyText(p); });
  $('#st-pw-new', el)?.addEventListener('click', () => regen('password'));
  $('#st-tk-new', el)?.addEventListener('click', () => regen('token'));
  $$('[data-apply-mt5]', el).forEach(b => b.onclick = async () => {
    b.disabled = true;
    try { await call('restart', { target: 'mt5' }); toast(t('MT5 yeniden başlatılıyor; ayarlar açılışta uygulanır (~20 sn).')); }
    catch (e) { toast(String(e), true); b.disabled = false; }
  });
}

function markDirty() {
  dirty = Object.keys(draft).some(k => draft[k] !== cfg.values[k]);
  $('#st-save', el).disabled = !dirty;
  $('#st-revert', el).disabled = !dirty;
}

async function apply(rebuild) {
  try {
    if (state.snap?.native) {
      // native.sh start is a no-op while running: stop first, start picks up .env
      if (state.snap.container?.status === 'running') await waitJob(await startJob('stop'));
      await startJob('start');
      return;
    }
    if (rebuild) {
      const id = await startJob('build');
      if (await waitJob(id) !== 0) return toast(t('İmaj derlemesi başarısız, loglara bak'), true);
    }
    await startJob('up');
  } catch (e) { toast(String(e), true); }
}

async function switchMode(m) {
  if (!m) return;
  const toNative = m.id === 'native';
  const ok = await modal({
    title: t('{mode} moduna geç?', { mode: t(m.label.split(' · ')[0]) }),
    body: html`<ul class="muted" style="margin:0;padding-left:18px;font-size:13px;line-height:1.7">
        <li>${t('Şu anki runtime durdurulur; verisi silinmez.')}</li>
        ${toNative
          ? html`<li>${t('İlk başlatmada Wine 11 (~215 MB) indirilir ve Windows Python kurulur.')}</li>
             <li>${t("MT5, Güvenli moddaki container'dan broker girişi ve geçmişiyle kopyalanır (container yoksa yeniden kurulur).")}</li>
             <li>${t('MT5 ayrı bir macOS penceresinde açılır; noVNC kullanılmaz.')}</li>`
          : html`<li>${t('Güvenli moddaki x86_64 VM ve container olduğu gibi kullanılır (VM kapalıysa açılır).')}</li>`}
        <li>${t('Ayarlar (token, MaxBars, portlar…) ortak kalır.')}</li>
      </ul>`,
    buttons: [{ label: t('Vazgeç'), value: false }, { label: t('Geç ve başlat'), value: 'start', primary: true }],
  });
  if (!ok) return;
  try {
    if (state.snap?.container?.status === 'running') {
      const id = await startJob('stop');
      await waitJob(id);
    }
    await call('set_mode', { mode: m.id });
    toast(t('Mod: {mode}', { mode: m.label.split(' · ').map((p, i) => (i ? p : t(p))).join(' · ') }));
    await load();
    startJob('start').catch(e => toast(String(e), true));
  } catch (e) { toast(String(e), true); }
}

async function save() {
  const changed = Object.fromEntries(Object.entries(draft).filter(([k, v]) => v !== cfg.values[k]));
  try { await call('save_settings', { values: changed }); }
  catch (e) { return toast(String(e), true); }
  toast(t('Kaydedildi'));
  await load();
  if (!state.snap?.container) return;
  const go = await modal({
    title: t('Değişiklikler uygulansın mı?'),
    body: html`<p class="muted" style="margin:0">${state.snap?.native ? t('Runtime yeni ayarlarla yeniden başlatılacak (MT5 kısa süre kapanır, veriler korunur).') : t('Container yeni ayarlarla yeniden oluşturulacak (MT5 ~1–2 dk kapalı kalır, veriler korunur).')}</p>`,
    buttons: [{ label: t('Sonra'), value: false }, { label: t('Uygula'), value: true, primary: true }],
  });
  if (go) apply(false);
}

async function regen(which) {
  const isPw = which === 'password';
  const ok = await modal({
    title: isPw ? t('Yeni masaüstü şifresi') : (cfg.has_token ? t('API token yenilensin mi?') : t('API token oluştur')),
    body: html`<p class="muted" style="margin:0">${isPw ? t('Rastgele yeni bir şifre üretilip .env dosyasına yazılır. Uygulanınca noVNC yeni şifreyi ister.') : t("Rastgele bir token üretilir. Uygulama token'ı .env'den okur; container yeniden oluşturulunca geçerli olur.")}</p>`,
    buttons: [{ label: t('Vazgeç'), value: false }, { label: t('Üret'), value: true, primary: true }],
  });
  if (!ok) return;
  try {
    const r = await call('regenerate', { which });
    await load();
    const go = await modal({
      title: isPw ? t('Şifre üretildi') : t('Token üretildi'),
      body: isPw ? html`<div class="row"><code class="mono selectable" style="font-size:15px;padding:8px 12px;background:var(--sunken);border-radius:8px">${r.password}</code><button class="btn sm" id="m-copy">${icon('copy')}${t('Kopyala')}</button></div><p class="muted" style="margin:0">${t("Şifre .env'de de saklanır; MT5 Ekranı onu otomatik kullanır.")}</p>`
        : html`<p class="muted" style="margin:0">${t("Token .env'e yazıldı.")}</p>`,
      buttons: [{ label: t('Sonra uygula'), value: false }, { label: t('Şimdi uygula'), value: true, primary: true }],
      onMount: bg => bg.querySelector('#m-copy')?.addEventListener('click', () => copyText(r.password)),
    });
    if (go && state.snap?.container) apply(false);
  } catch (e) { toast(String(e), true); }
}

export default {
  mount(node) { el = node; load(); },
  update(what) { if ((what === 'res' || what === 'snap') && !dirty && !el?.contains(document.activeElement) && actualKey() !== lastActual) { lastActual = actualKey(); draw(); } },
  unmount() { el = null; },
};
