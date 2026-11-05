// Servers (Server mode): every VPS this app manages, plus this computer, as
// cards. "Aktif yap" makes a target the one every other page shows.

import { call } from '../api.js';
import { $$, html, render, icon, toast, modal, money, duration } from '../dom.js';
import { state, phaseOf, startJob, waitJob, refreshInfo, refreshSnapshot } from '../store.js';
import { t } from '../i18n.js';

let el = null, cards = null, timer = null, error = null;

async function refresh() {
  try { cards = await call('servers'); error = null; } catch (e) { error = String(e); }
  draw();
}

const now = () => Date.now() / 1000;

function linkLine(c) {
  const l = c.link;
  if (!l) return '';
  if (l.state === 'connected') return html`<span class="chip up"><span class="d"></span>${t('Bağlı')}${c.latency_ms != null ? ` · ${c.latency_ms} ms` : ''}</span>`;
  if (l.state === 'connecting') return html`<span class="chip neu">${icon('spinner', 'spin')}${t('Bağlanıyor')}</span>`;
  return html`<span class="chip down"><span class="d"></span>${t('Bağlantı yok')}</span>`;
}

/** What went wrong with the tunnel, with the one action that fixes it. */
function problem(c) {
  const l = c.link;
  if (!l || l.state !== 'failed') return '';
  const box = (txt, btn = '') => html`<div class="callout bad">${icon('alert')}<div class="col" style="gap:8px"><div>${txt}</div>${btn}</div></div>`;
  switch (l.kind) {
    case 'key_required': return box(t('SSH anahtarı gerekli: bu sunucuya anahtarla girilemedi.'), html`<div><button class="btn sm" data-keygen="${c.id}">${icon('key')}${t('Anahtarı sunucuya ekle')}</button></div>`);
    case 'host_key_changed': return box(t('Sunucu kimliği değişti. Sunucuyu yeniden kurmadıysan bu bir saldırı olabilir; bağlantı durduruldu.'), html`<div><button class="btn sm" data-rekey="${c.id}">${icon('search')}${t('Yeni parmak izini incele')}</button></div>`);
    case 'port_in_use': return box(t('Yerel port {p} kullanımda (başka bir program dinliyor).', { p: c.port_base + 2 }), html`<div><button class="btn sm" data-ports="${c.id}">${icon('sliders')}${t('Port bloğunu değiştir')}</button></div>`);
    default: return box(t('Bağlantı yok (son deneme {d} önce): {detail}', { d: duration(now() - (l.at || now())), detail: l.detail || '' }));
  }
}

function card(c) {
  const local = c.id === 'local';
  const ph = phaseOf({ lifecycle: c.lifecycle, engine_label: 'Docker' });
  const a = c.account;
  const running = ['on', 'busy'].includes(ph.group);
  const modeName = { docker: 'Docker', 'linux-docker': 'Docker', 'linux-native': t('Linux yerel'), windows: 'Windows', 'mac-native': t('Mac yerel'), 'mac-colima': t('Güvenli') }[c.mode] || c.mode;
  return html`<section class="card pad col srvcard ${c.active ? 'on' : ''}" data-card="${c.id}">
    <div class="row between wrap">
      <div class="row" style="gap:10px"><div class="osico">${icon(local ? 'laptop' : 'server')}</div>
        <div class="col" style="gap:2px"><div class="row" style="gap:6px"><b style="font-size:15px">${c.name}</b>${local ? '' : html`<span class="chip neu" data-badge>${modeName}</span>`}</div><span class="muted ${local ? '' : 'mono'}" style="font-size:12px">${local ? modeName : `${c.ssh} · ${modeName}`}</span></div></div>
      <div class="row" style="gap:6px">${c.active ? html`<span class="chip up">${t('aktif')}</span>` : html`<button class="btn sm" data-activate="${c.id}">${t('Aktif yap')}</button>`}</div></div>
    <div class="row wrap" style="gap:6px">${linkLine(c)}<span class="chip ${ph.tone}"><span class="d"></span>${ph.label}</span>
      ${c.update_needed ? html`<span class="chip warn">${t('Güncellenmeli')}</span>` : ''}</div>
    ${a ? html`<div class="grid g4">
        <div class="col" style="gap:2px"><span class="muted" style="font-size:12px">${t('Hesap')}</span><b>${a.server || '—'} · ${a.login_masked || ''}</b></div>
        <div class="col" style="gap:2px"><span class="muted" style="font-size:12px">${t('Bakiye')}</span><b class="num">${money(c.balance, c.currency)}</b></div>
        <div class="col" style="gap:2px"><span class="muted" style="font-size:12px">${t('Varlık')}</span><b class="num">${money(c.equity, c.currency)}</b></div>
        <div class="col" style="gap:2px"><span class="muted" style="font-size:12px">${t('Bugün K/Z')}</span><b class="num ${c.today_pl > 0 ? 'pos' : c.today_pl < 0 ? 'neg' : ''}">${c.today_pl == null ? '—' : money(c.today_pl, c.currency)}</b></div></div>` : ''}
    ${problem(c)}
    ${!local && c.mode === 'linux-native' && !c.linger && c.link?.state === 'connected' && c.lifecycle !== 'not_installed'
      ? html`<div class="callout warn">${icon('alert')}<div class="col" style="gap:8px"><div>${t("Linger kapalı: SSH'tan çıkınca durur. Açmak için sunucuda bir kez sudo gerekir.")}</div><div><button class="btn sm" data-fix="${c.id}">${icon('terminal')}${t("Terminal'de düzelt")}</button></div></div></div>` : ''}
    <div class="row wrap between">
      <span class="muted mono" style="font-size:12px">${t('Python köprüsü')}: 127.0.0.1:${c.bridge_port}</span>
      <div class="row wrap" style="gap:6px">
        ${local ? '' : running ? html`<button class="btn sm" data-job="stop" data-id="${c.id}">${icon('stop')}${t('Durdur')}</button>`
          : ['stopped'].includes(c.lifecycle) ? html`<button class="btn sm p" data-job="start" data-id="${c.id}">${icon('play')}${t('Başlat')}</button>` : ''}
        ${!local && c.update_needed ? html`<button class="btn sm p" data-update="${c.id}">${icon('refresh')}${t('Güncelle')}</button>` : ''}
        ${!local && c.lifecycle === 'not_installed' ? html`<button class="btn sm p" data-install="${c.id}">${icon('play')}${t('Kur')}</button>` : ''}
        <button class="btn sm" data-open="${c.id}" data-to="screen">${icon('screen')}${t('MT5 Ekranı')}</button>
        <button class="btn sm" data-open="${c.id}" data-to="logs">${icon('logs')}${t('Loglar')}</button>
        ${local ? '' : html`<button class="btn sm ib danger" data-remove="${c.id}" title="${t('Kaldır')}" aria-label="${t('Kaldır')}">${icon('trash')}</button>`}</div></div>
  </section>`;
}

function draw() {
  if (!el) return;
  render(el, html`
    <div class="head"><div><div class="crumbs">SiliconMetaTrader5 › <b>${t('Sunucular')}</b></div><h1>${t('Sunucular')}</h1>
      <div class="muted" style="margin-top:4px;max-width:760px">${t('Bu uygulamanın yönettiği sunucular. Aktif olan, diğer sayfaların gösterdiği hedeftir. Bağlantı SSH tünelinden geçer; uygulama kapalıyken sunucular çalışmaya devam eder.')}</div></div>
      <div class="row"><button class="btn" id="sv-refresh">${icon('refresh')}${t('Yenile')}</button><a class="btn p" href="#/setup?add=1" id="sv-add">${icon('plus')}${t('Sunucu ekle')}</a></div></div>
    ${error ? html`<div class="callout bad">${icon('alert')}<div>${error}</div></div>` : ''}
    ${!cards ? html`<p class="muted">${t('Yükleniyor…')}</p>`
      : cards.some(c => c.id !== 'local') ? html`<div class="col">${cards.map(card)}</div>`
      : html`${cards.map(card)}<section class="card empty srvcard"><div class="osico" style="margin:0 auto">${icon('server')}</div>
          <h2>${t('Henüz sunucu yok')}</h2>
          <p>${t('MT5\'i bir VPS\'te de çalıştırabilirsin: uygulama SSH ile kurar, buradan yönetirsin. Bu bilgisayardaki kurulum olduğu gibi kalır.')}</p>
          <div class="row" style="justify-content:center"><a class="btn p" href="#/setup?add=1" id="sv-first">${icon('plus')}${t('İlk sunucunu ekle')}</a></div></section>`}`);
  $$('#sv-refresh', el).forEach(b => b.onclick = refresh);
  $$('#sv-add, #sv-first', el).forEach(b => b.onclick = () => { try { localStorage.setItem('smt5.osPick', 'server'); localStorage.setItem('smt5.osSeen', '1'); } catch { /* private */ } });
  const activate = async id => { await call('set_active_target', { id }); await refreshInfo(); await refreshSnapshot(); };
  $$('[data-activate]', el).forEach(b => b.onclick = async () => { try { await activate(b.dataset.activate); location.reload(); } catch (e) { toast(String(e), true); } });
  $$('[data-open]', el).forEach(b => b.onclick = async () => { try { await activate(b.dataset.open); location.hash = '#/' + b.dataset.to; location.reload(); } catch (e) { toast(String(e), true); } });
  // runs on that server explicitly; the active target does not change
  const jobOn = async (id, kind) => {
    try { const code = await waitJob(await startJob(kind, { target: id })); await refresh(); return code; } catch (e) { toast(String(e), true); return -1; }
  };
  $$('[data-install]', el).forEach(b => b.onclick = async () => { try { await activate(b.dataset.install); location.hash = '#/setup'; location.reload(); } catch (e) { toast(String(e), true); } });
  $$('[data-job]', el).forEach(b => b.onclick = () => jobOn(b.dataset.id, b.dataset.job));
  $$('[data-update]', el).forEach(b => b.onclick = () => jobOn(b.dataset.update, 'server_update'));
  $$('[data-keygen]', el).forEach(b => b.onclick = async () => {
    try { await call('server_keygen', { id: b.dataset.keygen }); toast(t("Terminal açıldı: sunucu şifresini orada gir, sonra Tekrar dene.")); } catch (e) { toast(String(e), true); }
  });
  $$('[data-fix]', el).forEach(b => b.onclick = async () => {
    try { await call('server_fix', { id: b.dataset.fix, mode: 'linux-native' }); toast(t("Terminal açıldı: komut bitince Tekrar kontrol et.")); } catch (e) { toast(String(e), true); }
  });
  $$('[data-rekey]', el).forEach(b => b.onclick = async () => {
    const c = cards.find(x => x.id === b.dataset.rekey);
    try {
      const pr = await call('server_probe', { ssh: c.ssh });
      const key = pr.keys.find(k => k.kind === 'ED25519') || pr.keys[0];
      const yes = await modal({ title: t('{name} için yeni parmak izi', { name: c.name }),
        body: html`<p class="muted" style="margin:0">${t('Bunu sunucu sağlayıcının paneliyle karşılaştır. Emin değilsen kabul etme.')}</p><p class="mono selectable" style="font-size:12px">${key.kind} · ${key.sha256}</p>`,
        buttons: [{ label: t('Vazgeç'), value: false }, { label: t('Kabul et'), value: true, primary: true }] });
      if (!yes) return;
      await call('server_rekey', { id: c.id, sha256: key.sha256 });
      refresh();
    } catch (e) { toast(String(e), true); }
  });
  $$('[data-ports]', el).forEach(b => b.onclick = async () => {
    const c = cards.find(x => x.id === b.dataset.ports);
    let input = null;
    const yes = await modal({ title: t('Port bloğunu değiştir'),
      body: html`<p class="muted" style="margin:0">${t("Yeni port bloğu (18000–64900, 100'ün katı). Köprü bloğun +1, durum API'si +2, noVNC +81 portunu kullanır.")}</p><input class="f mono" id="sv-base" value="${String(c.port_base + 1000)}" inputmode="numeric">`,
      buttons: [{ label: t('Vazgeç'), value: false }, { label: t('Değiştir'), value: true, primary: true }],
      onMount: bg => { input = bg.querySelector('#sv-base'); input?.focus(); } });
    const base = yes ? Number(input?.value) : 0;
    if (!base) return;
    try { await call('server_set_ports', { id: c.id, base }); toast(t('Mac\'teki Python kodunda köprü portu artık {p}.', { p: base + 1 })); refresh(); } catch (e) { toast(String(e), true); }
  });
  // removing has one place with every option explained: Sıfırlama ve Kaldırma, on this server's tab
  $$('[data-remove]', el).forEach(b => b.onclick = () => { location.hash = '#/reset?target=' + b.dataset.remove; });
}

export default {
  mount(node) { el = node; cards = null; draw(); refresh(); timer = setInterval(refresh, 10000); },
  update() {},
  unmount() { clearInterval(timer); el = null; },
};
