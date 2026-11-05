// Reset and uninstall, three levels from mild to complete. Each shows what is
// deleted and what is kept (from the backend's removal_info), needs a typed
// confirmation (the word follows the UI language), and runs as a normal job with its output in the drawer.

import { call } from '../api.js';
import { $, $$, html, render, icon, bytes, toast, modal } from '../dom.js';
import { state, startJob, waitJob, refreshSnapshot, refreshInfo } from '../store.js';
import { t, th } from '../i18n.js';

let el, info = null, loading = false, finished = null;
// which target this page acts on: "local" or a server id (#/reset?target=…)
let target = 'local', servers = [];
const fromHash = () => (location.hash.match(/[?&]target=([^&]+)/) || [])[1];

async function load() {
  loading = true; draw();
  try { servers = (await call('servers')).filter(c => c.id !== 'local'); } catch { servers = []; }
  if (target !== 'local' && !servers.some(c => c.id === target)) target = 'local';
  try { info = await call('removal_info'); } catch (e) { toast(String(e), true); }
  loading = false; draw();
}

const list = (items, cls) => html`<ul class="${cls}">${items.filter(Boolean).map(i => html`<li>${i}</li>`)}</ul>`;

function nativeLevels() {
  const i = info;
  const home = i.native_home || '~/Library/Application Support/SiliconMT5';
  const size = i.native_bytes ? ` · ${bytes(i.native_bytes)}` : '';
  return [{
    id: 'reset', word: t('SIFIRLA'), tone: 'warn',
    title: t('MT5 verisini sıfırla'),
    lead: t("Bozulmuş bir MT5 kurulumunu ya da şişmiş geçmiş veriyi temizlemek için. Sonraki Başlat'ta Windows Python kurulur, MT5 Güvenli moddan kopyalanır ya da yeniden kurulur."),
    del: [t('Wine prefix: MT5 kurulumu, broker girişi, bar ve tick geçmişi'), t('Windows Python ve paketleri'), t('Runtime durumu (state)')],
    keep: [t('Wine 11 indirmesi'), t("Durum API'si ortamı (venv)"), t('Loglar'), t('Ayarlar ve token (.env)'), t('Güvenli moddaki VM ve verisi')],
    disabled: !i.native_exists ? t('Silinecek bir şey yok') : null,
  }, {
    id: 'reset_vm', word: 'MAC', tone: 'bad',
    title: t('Mac yerel modu sil, kurulumu baştan al'),
    lead: t('Wine tarafında bir şeyler bozulduysa. Kurulum sihirbazı ilk adımdan başlar; Wine yeniden indirilir.'),
    del: [`${home}${size}`, t('Wine 11, Wine prefix, Windows Python, venv, loglar'), t('MT5 kurulumu, broker girişi, geçmiş veri')],
    keep: [t('Güvenli moddaki VM ve verisi'), t('Ayarlar (.env) — onay penceresinde silmeyi seçebilirsin')],
    options: [{ key: 'reset_env', label: t('Token ve ayarları da sıfırla (.env)') }],
    disabled: !i.native_exists ? t('Silinecek bir şey yok') : null,
  }];
}

/** A server: reset MT5 data, remove everything there, or list only. */
function serverLevels(c) {
  if (c.mode === 'windows') return [{
    id: 'server_reset', word: t('SIFIRLA'), tone: 'warn',
    title: t('MT5 verisini sıfırla'),
    lead: t("Sunucudaki bozulmuş bir MT5 kurulumunu ya da şişmiş geçmiş veriyi temizlemek için. Programlar kalır; sonraki Başlat MT5'i yeniden hazırlar."),
    del: [t('MT5 verisi: hesaplar ve broker girişi'), t('İndirilen bar ve tick geçmişi')],
    keep: [t('Programlar: MT5, Python, TightVNC, noVNC'), t('Sunucunun ayarları, şifresi ve token\'ı (.env)'), t('Bu listedeki kayıt')],
    disabled: c.link?.state !== 'connected' ? t('Sunucuya bağlı değil') : null,
  }, {
    id: 'server_uninstall', word: t('KALDIR'), tone: 'bad',
    title: t('Sunucudan tamamen kaldır'),
    lead: t('SiliconMetaTrader5\'in bu sunucuda oluşturduğu her şey. Sonra sunucu bu listeden de çıkar.'),
    del: [t('Zamanlanmış görevler, otomatik oturum, TightVNC, MT5, Python ve klasörler'), t('MT5, broker girişi ve geçmiş veri'), t('Bu listedeki kayıt')],
    keep: [t('Kilit ve ekran koruyucu ayarları geri alınır; SSH anahtarı ve OpenSSH Server kalır'), t('Sunucunun kendisi ve diğer programlar')],
    disabled: c.link?.state !== 'connected' ? t('Sunucuya bağlı değil; ulaşılamıyorsa "Yalnız listeden kaldır"ı kullan') : null,
  }, {
    id: 'forget', word: c.name.toUpperCase(), tone: 'warn',
    title: t('Yalnız listeden kaldır'),
    lead: t('Sunucuya dokunulmaz: MT5 orada çalışmaya devam eder. Sunucu artık yoksa ya da ulaşılamıyorsa bunu kullan.'),
    del: [t('Bu listedeki kayıt ve bağlantı'), t('Uygulamanın bu sunucu için sakladığı parmak izi')],
    keep: [t('Sunucudaki her şey (MT5, veriler, runtime)')],
  }];
  return [{
    id: 'server_reset', word: t('SIFIRLA'), tone: 'warn',
    title: t('MT5 verisini sıfırla'),
    lead: t('Sunucudaki bozulmuş bir MT5 kurulumunu ya da şişmiş geçmiş veriyi temizlemek için. Sonraki Başlat MT5\'i sunucuda yeniden kurar.'),
    del: [t('MT5 kurulumu ve Wine prefix (sunucuda)'), t('Broker girişi'), t('İndirilen bar ve tick geçmişi')],
    keep: [t('Runtime, Wine ve Docker imajı'), t('Sunucunun ayarları, şifresi ve token\'ı (.env)'), t('Bu listedeki kayıt')],
    disabled: c.link?.state !== 'connected' ? t('Sunucuya bağlı değil') : null,
  }, {
    id: 'server_uninstall', word: t('KALDIR'), tone: 'bad',
    title: t('Sunucudan tamamen kaldır'),
    lead: t('SiliconMetaTrader5\'in bu sunucuda oluşturduğu her şey. Sonra sunucu bu listeden de çıkar.'),
    del: [t('MT5, broker girişi ve geçmiş veri'), t('Runtime ve ayarları (~/siliconmetatrader5)'), c.mode === 'docker' ? t('Container ve volume\'lar') : t('Wine ve Linux yerel modun dosyaları'), t('Bu listedeki kayıt')],
    keep: [t('Sistem paketleri (Docker, Xvfb, noVNC…)'), t('Sunucunun kendisi ve diğer programlar')],
    disabled: c.link?.state !== 'connected' ? t('Sunucuya bağlı değil; ulaşılamıyorsa "Yalnız listeden kaldır"ı kullan') : null,
  }, {
    id: 'forget', word: c.name.toUpperCase(), tone: 'warn',
    title: t('Yalnız listeden kaldır'),
    lead: t('Sunucuya dokunulmaz: MT5 orada çalışmaya devam eder. Sunucu artık yoksa ya da ulaşılamıyorsa bunu kullan.'),
    del: [t('Bu listedeki kayıt ve bağlantı'), t('Uygulamanın bu sunucu için sakladığı parmak izi')],
    keep: [t('Sunucudaki her şey (MT5, veriler, runtime)')],
  }];
}

function levels() {
  if (target !== 'local') { const c = servers.find(x => x.id === target); return c ? serverLevels(c) : []; }
  const i = info, vm = i.profile;
  const vmLabel = vm ? `x86_64 VM '${vm.name}' · ${bytes(vm.disk)} disk` : 'x86_64 VM';
  const others = i.other_containers.length ? t("VM'deki diğer container'lar: {list}", { list: i.other_containers.join(', ') }) : null;
  const nativeDir = i.native_exists ? t('Mac yerel mod: {path}', { path: i.native_home }) + (i.native_bytes ? ' · ' + bytes(i.native_bytes) : '') : null;
  if (i.native) return [...nativeLevels(), uninstallLevel(i, vm, vmLabel, others, nativeDir)];
  const out = [{
    id: 'reset', word: t('SIFIRLA'), tone: 'warn',
    title: t('MT5 verisini sıfırla'),
    lead: t("Bozulmuş bir MT5 kurulumunu ya da şişmiş geçmiş veriyi temizlemek için. Sonraki Başlat'ta MT5 yeniden kurulur (~10 dk)."),
    del: [t('Wine prefix ve MT5 kurulumu'), t('Broker girişi'), t('İndirilen bar ve tick geçmişi'), t("Container logları"), ...i.volumes.map(v => `volume: ${v}`)],
    keep: [t('Runtime imajı (yeniden derleme gerekmez)'), i.manages_vm ? 'x86_64 VM' : 'Docker Engine', t('Ayarlar, şifre ve token (.env)')],
    disabled: !i.engine_up ? t("Docker engine kapalı: önce runtime'ı Başlat ile açın") : null,
  }];
  if (i.manages_vm) out.push({
    id: 'reset_vm', word: vm?.name || 'x86', tone: 'bad',
    title: t("VM'i sil, kurulumu baştan al"),
    lead: t('Colima tarafında bir şeyler bozulduysa ya da disk dolduysa. Kurulum sihirbazı ilk adımdan başlar; imaj yeniden derlenir.'),
    del: [vmLabel, t("İçindeki runtime imajı ve derleme önbelleği"), t('MT5 kurulumu, broker girişi, geçmiş veri'), others],
    keep: [...i.other_profiles.map(p => t("Colima profili '{p}' (başka projeler)", { p })), t('Homebrew paketleri (colima, docker, qemu…)'), t('Ayarlar (.env) — onay penceresinde silmeyi seçebilirsin')],
    options: [{ key: 'reset_env', label: t('Şifre, token ve ayarları da sıfırla (.env)') }],
    disabled: !vm ? t('Silinecek VM profili yok') : null,
  });
  out.push(uninstallLevel(i, vm, vmLabel, others, nativeDir));
  return out;
}

function uninstallLevel(i, vm, vmLabel, others, nativeDir) {
  return {
    id: 'uninstall', word: t('KALDIR'), tone: 'bad',
    title: t("SiliconMetaTrader5'i tamamen kaldır"),
    lead: t("SiliconMetaTrader5'in bu bilgisayarda oluşturduğu her şey (iki modun da). Başka projelerin Colima profilleri ve Docker kurulumu korunur."),
    del: [nativeDir, i.manages_vm ? (vm ? vmLabel : null) : t('SiliconMetaTrader5 container, volume ve imajları'), others, ...(i.manages_vm ? [] : i.images), t('MT5 kurulumu, broker girişi, geçmiş veri'),
      t('Ayarlar: {path}', { path: i.env_path }), t('Uygulama ayarı: {path}', { path: i.config_path })],
    keep: [i.packaged ? t('Uygulama ve smt5 komutu (brew uninstall siliconmt5 ile kaldırılır)') : t('Repo klasörü (git çalışma kopyası)'), ...i.other_profiles.map(p => t("Colima profili '{p}'", { p })), i.packages_allowed ? t('Homebrew paketleri — onay penceresinde kaldırmayı seçebilirsin') : t('Homebrew paketleri / Docker')],
    options: [{ key: 'remove_packages', label: t('Homebrew paketlerini de kaldır: {list}', { list: i.brew_formulae.join(', ') || '—' }), disabled: !i.packages_allowed, reason: i.packages_reason }],
  };
}

async function run(level) {
  const opts = {};
  const ok = await modal({
    title: level.title + '?',
    body: html`<div class="callout ${level.tone === 'warn' ? 'warn' : 'bad'}">${icon('alert')}<div>${t('Bu işlem geri alınamaz. Silinecekler:')}</div></div>
      ${list(level.del, 'del')}
      ${(level.options || []).map(o => html`<label class="row" style="font-size:13px;align-items:flex-start"><input type="checkbox" data-opt="${o.key}" ${o.disabled ? 'disabled' : ''} style="margin-top:3px;accent-color:var(--neg)"><span>${o.label}${o.disabled && o.reason ? html`<br><span class="muted" style="font-size:12px">${o.reason}</span>` : ''}</span></label>`)}
      <label class="fl"><span>${th('Onaylamak için <span class="mono" style="font-size:13px">{word}</span> yaz', { word: level.word })}</span><input class="f" id="m-confirm" autocomplete="off" spellcheck="false"></label>`,
    buttons: [{ label: t('Vazgeç'), value: false }, { label: level.title, value: true, primary: true, danger: true }],
    onMount: bg => {
      const btn = bg.querySelectorAll('button[data-i]')[1];
      btn.disabled = true;
      bg.querySelector('#m-confirm').addEventListener('input', e => { btn.disabled = e.target.value.trim() !== level.word; });
      bg.querySelectorAll('input[data-opt]').forEach(c => c.addEventListener('change', () => { opts[c.dataset.opt] = c.checked; }));
    },
  });
  if (!ok) return;
  if (level.id === 'forget') {
    try { await call('server_forget', { id: target }); target = 'local'; await refreshInfo(); toast(t('Listeden kaldırıldı.')); location.hash = '#/reset'; load(); } catch (e) { toast(String(e), true); }
    return;
  }
  if (target !== 'local') {
    try {
      const code = await waitJob(await startJob(level.id, { target }));
      if (code !== 0) return toast(t('İşlem tamamlanamadı; çıktıya bakın'), true);
      if (level.id === 'server_uninstall') { await refreshInfo(); toast(t('Sunucudan kaldırıldı.')); target = 'local'; location.hash = '#/reset'; }
      else toast(t('Sunucuda MT5 verisi sıfırlandı. Başlat ile MT5 yeniden kurulur.'));
      load();
    } catch (e) { toast(String(e), true); }
    return;
  }
  opts.target = 'local';
  try {
    const id = await startJob(level.id, opts);
    const code = await waitJob(id);
    await refreshSnapshot();
    if (code !== 0) return toast(t('İşlem tamamlanamadı; çıktıya bakın'), true);
    if (level.id === 'reset') { toast(t('MT5 verisi sıfırlandı. Başlat ile MT5 yeniden kurulur.')); load(); }
    else if (level.id === 'reset_vm') { toast(info.native ? t('Mac yerel modun dosyaları silindi. Kurulum sihirbazı baştan başlıyor.') : t('VM silindi. Kurulum sihirbazı baştan başlıyor.')); location.hash = '#/setup'; }
    else { finished = opts; draw(); }
  } catch (e) { toast(String(e), true); }
}

function draw() {
  if (!el) return;
  const tname = target === 'local' ? t('Bu bilgisayar') : (servers.find(c => c.id === target)?.name || target);
  const tabs = servers.length ? html`<div class="seg" role="tablist" style="align-self:flex-start;flex-wrap:wrap">${[['local', t('Bu bilgisayar')], ...servers.map(c => [c.id, c.name])].map(([id, name]) =>
    html`<button data-rtarget="${id}" class="${id === target ? 'sel' : ''}" role="tab" aria-selected="${id === target ? 'true' : 'false'}">${name}</button>`)}</div>
    <div class="callout info">${icon('info')}<div>${th('Bu sayfadaki işlemler yalnız <b>{name}</b> hedefini etkiler.', { name: tname })}</div></div>` : '';
  const head = html`<div class="head"><div><div class="crumbs">SiliconMetaTrader5 › ${t('Uygulama')} › <b>${t('Sıfırlama ve Kaldırma')}</b></div><h1>${t('Sıfırlama ve Kaldırma')}</h1>
    <div class="muted" style="margin-top:4px;max-width:760px">${target !== 'local' ? t('Sunucu için üç seçenek: yalnız MT5 verisini sıfırla, sunucudan tamamen kaldır ya da sunucuya dokunmadan listeden çıkar.') : info?.native ? t('Hafiften kapsamlıya üç seçenek (ilk ikisi Mac yerel moda ait). Hiçbiri başka projelerin Colima profillerine, Docker kurulumuna ya da repo klasörüne dokunmaz.') : t('Hafiften kapsamlıya üç seçenek. Hiçbiri başka projelerin Colima profillerine, Docker kurulumuna ya da repo klasörüne dokunmaz.')}</div></div>
    </div>${tabs}`;
  if (finished) {
    render(el, html`${head}<section class="card empty"><span class="dot ok" style="width:48px;height:48px">${icon('check')}</span>
      <h2 style="font-size:20px">${t('SiliconMetaTrader5 kaldırıldı')}</h2>
      <p>${finished.remove_packages ? t('Runtime dosyaları, veriler, ayarlar ve Homebrew paketleri silindi. Kalan adımlar:') : t('Runtime dosyaları, veriler ve ayarlar silindi; Homebrew paketleri yerinde. Kalan adımlar:')}</p>
      <div style="text-align:left">${list(info?.manual_steps || [], 'keep')}</div>
      <div class="row"><button class="btn p" id="rs-quit">${t('Uygulamadan çık')}</button><a class="btn" href="#/setup">${t('Yeniden kur')}</a></div></section>`);
    $('#rs-quit', el).onclick = () => call('quit_app');
    return;
  }
  const bindTabs = () => $$('[data-rtarget]', el).forEach(b => b.onclick = () => { target = b.dataset.rtarget; draw(); });
  if (!info && target === 'local') { render(el, html`${head}<div class="card pad muted">${loading ? t('Kontrol ediliyor…') : t('Bilgi alınamadı.')}</div>`); bindTabs(); return; }
  const busy = state.job?.running;
  render(el, html`${head}
    <div class="grid" style="grid-template-columns:repeat(auto-fit,minmax(300px,1fr));align-items:stretch">
      ${levels().map((l, n) => html`<section class="card pad level" style="${l.tone === 'bad' ? 'border-color:var(--down-bg)' : ''}">
        <div class="row"><span class="dot ${l.tone === 'warn' ? 'warn' : 'bad'}">${n + 1}</span><h2>${l.title}</h2></div>
        <p class="muted" style="margin:0;font-size:13px">${l.lead}</p>
        <h3>${t('SİLİNİR')}</h3>${list(l.del, 'del')}
        <h3>${t('KORUNUR')}</h3>${list(l.keep, 'keep')}
        <div style="flex:1"></div>
        ${l.disabled ? html`<div class="muted" style="font-size:12px">${l.disabled}</div>` : ''}
        <button class="btn danger ${l.tone === 'bad' ? 'p' : ''}" data-level="${l.id}" ${l.disabled || busy ? 'disabled' : ''}>${icon('trash')}${l.title}…</button>
      </section>`)}
    </div>
    ${target === 'local' && info?.other_containers.length ? html`<div class="callout warn">${icon('alert')}<div>${t("SiliconMetaTrader5'in VM'inde başka container'lar da çalışıyor ({list}). VM silinirse onlar da silinir.", { list: info.other_containers.join(', ') })}</div></div>` : ''}`);
  const lv = levels();
  $$('[data-level]', el).forEach(b => b.onclick = () => run(lv.find(l => l.id === b.dataset.level)));
  bindTabs();
}

export default {
  mount(node) { el = node; finished = null; target = fromHash() || 'local'; load(); },
  update(what) { if (what === 'job' && el && !finished) draw(); },
  unmount() { el = null; },
};
