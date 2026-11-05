// Accounts: logging in happens only in MT5. The app detects the active account,
// follows every change made in MT5 (or from Python) and remembers the accounts
// it has seen; "log in" brings the MT5 window forward.

import { call } from '../api.js';
import { $, $$, html, render, icon, toast, modal, duration } from '../dom.js';
import { state, nativeWindow } from '../store.js';
import { t, th } from '../i18n.js';

let el = null, data = null, timer = null;

const mode = m => ({ demo: t('Demo'), real: t('Gerçek'), contest: t('Yarışma') }[m] || m || '—');
const same = (a, b) => a && b && a.login === b.login && a.server === b.server;

async function refresh() {
  try { data = await call('accounts'); } catch (e) { data = { error: String(e), known: data?.known || [] }; }
  draw();
}

/** Logging in is done in MT5 itself: bring its window (or the MT5 Screen) forward. */
async function openMt5() {
  try {
    if (nativeWindow()) await call('focus_mt5');
    else location.hash = '#/screen';
  } catch (e) { toast(String(e), true); }
}

function draw() {
  if (!el) return;
  const cur = data?.current, known = data?.known || [], ready = !!state.snap?.status?.bridge?.listening;
  render(el, html`
    <div class="head"><div><div class="crumbs">SiliconMetaTrader5 › <b>${t('Hesaplar')}</b></div><h1>${t('Hesaplar')}</h1>
      <div class="muted" style="margin-top:4px;max-width:760px">${t('Aktif hesap her zaman MT5\'te giriş yapılmış hesaptır. MT5\'te ya da Python\'dan başka bir hesaba geçersen burada birkaç saniye içinde değişir.')}</div></div>
      <button class="btn" id="ac-refresh">${icon('refresh')}${t('Yenile')}</button></div>

    ${!ready ? html`<div class="callout warn">${icon('info')}<div>${t('Hesaplar köprü üzerinden okunur; runtime çalışmıyor.')}</div></div>` : ''}
    ${data?.error ? html`<div class="callout bad">${icon('alert')}<div>${data.error}</div></div>` : ''}

    <section class="card pad col">
      <div class="row between"><h2>${t('Aktif hesap')}</h2>${cur ? html`<span class="chip ${data.connected ? 'up' : 'warn'}"><span class="d"></span>${data.connected ? t('Sunucuya bağlı') : t('Bağlantı yok')}</span>` : ''}</div>
      ${cur ? html`<div class="acct">
          <div class="big">${cur.name || cur.login}</div>
          <div class="grid g4">
            <div class="col" style="gap:2px"><span class="muted" style="font-size:12px">${t('Hesap')}</span><b class="num">${cur.login}</b></div>
            <div class="col" style="gap:2px"><span class="muted" style="font-size:12px">${t('Sunucu')}</span><b>${cur.server}</b></div>
            <div class="col" style="gap:2px"><span class="muted" style="font-size:12px">${t('Tür')}</span><b><span class="chip ${cur.trade_mode === 'real' ? 'warn' : 'neu'}">${mode(cur.trade_mode)}</span></b></div>
            <div class="col" style="gap:2px"><span class="muted" style="font-size:12px">${t('Şirket')}</span><b>${cur.company || '—'}</b></div></div></div>`
        : html`<p class="muted" style="margin:0">${data ? t('MT5\'te giriş yapılmış bir hesap yok.') : t('Yükleniyor…')}</p>`}
    </section>

    <section class="card pad col">
      <h2>${t('Bu bilgisayarda kullanılan hesaplar')}</h2>
      ${known.length ? html`<table><thead><tr><th>${t('Hesap')}</th><th>${t('Sunucu')}</th><th>${t('Ad')}</th><th>${t('Tür')}</th><th>${t('Son görülme')}</th><th></th></tr></thead><tbody>
        ${known.map(a => {
          const active = same(a, cur), key = `${a.login}@${a.server}`;
          return html`<tr class="${active ? 'on' : ''}"><td class="num"><b>${a.login}</b></td><td>${a.server}</td><td class="muted">${a.name || '—'}</td><td>${mode(a.trade_mode)}</td>
            <td class="muted">${active ? html`<span class="chip up">${t('aktif')}</span>` : t('{d} önce', { d: duration(Date.now() / 1000 - a.last_seen) })}</td>
            <td class="r"><div class="row" style="justify-content:flex-end;gap:6px">
              <button class="btn sm ib" data-forget="${key}" title="${t('Listeden kaldır')}" aria-label="${t('Listeden kaldır')}" ${active ? 'disabled' : ''}>${icon('trash')}</button></div></td></tr>`;
        })}</tbody></table>`
        : html`<p class="muted" style="margin:0">${t('Henüz hesap yok. MT5\'te giriş yaptığında burada görünür.')}</p>`}
      <div class="callout info">${icon('info')}<div>${th("MT5 kayıtlı hesap listesini şifreli tutar ve Python API'si bu listeyi okuyamaz; bu liste SiliconMetaTrader5'in gördüğü hesaplardır. MT5'ten sildiğin bir hesap burada kalırsa çöp kutusuyla kaldır. Bir hesabı MT5'ten de silmek için: <b>Navigator › Accounts</b> › hesaba sağ tık › <b>Delete</b>.")}</div></div>
    </section>

    <section class="card pad col">
      <div class="row between wrap"><h2>${t('Hesap eklemek ya da değiştirmek')}</h2>
        <button class="btn p" id="ac-mt5" ${ready ? '' : 'disabled'}>${icon(nativeWindow() ? 'window' : 'screen')}${t("MT5'te giriş yap")}</button></div>
      <ol class="muted" style="margin:0;padding-left:18px;font-size:13px;line-height:1.8">
        <li>${th("Kayıtlı bir hesaba geçmek için MT5'te <b>Navigator › Accounts</b> içinde hesaba çift tıkla.")}</li>
        <li>${th("Yeni bir hesap için <b>File › Login to Trade Account</b> (mevcut hesap) ya da <b>File › Open an Account</b> (broker'ını bul).")}</li>
        <li>${t('SiliconMetaTrader5 aktif hesabı birkaç saniye içinde algılar ve bu sayfa ona göre güncellenir. Şifreler yalnız MT5\'te girilir; uygulama hiçbir şifre istemez.')}</li></ol>
    </section>`);

  $('#ac-refresh', el).onclick = refresh;
  const find = key => known.find(a => `${a.login}@${a.server}` === key);
  $('#ac-mt5', el).onclick = openMt5;
  $$('[data-forget]', el).forEach(b => b.onclick = async () => {
    const a = find(b.dataset.forget);
    const ok = await modal({ title: t('{login} listeden kaldırılsın mı?', { login: a.login }), body: html`<p class="muted" style="margin:0">${t('Yalnız bu listeden kaldırılır. MT5\'teki kaydı silmek için MT5\'te Navigator › Accounts › Delete kullan.')}</p>`,
      buttons: [{ label: t('Vazgeç'), value: false }, { label: t('Kaldır'), value: true, primary: true }] });
    if (!ok) return;
    try { const known2 = await call('account_forget', { login: Number(a.login), server: a.server }); data = { ...data, known: known2 }; draw(); } catch (e) { toast(String(e), true); }
  });
}

export default {
  mount(node) { el = node; draw(); refresh(); timer = setInterval(refresh, 6000); },
  update(what) { if (what === 'snap' && !data) draw(); },
  unmount() { clearInterval(timer); el = null; },
};
