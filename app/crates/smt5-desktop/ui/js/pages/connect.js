// Education › Bot connection: which host and port a bot writes, depending on
// where the bot runs. No bot template: bots differ, the only thing they share
// is the MetaTrader5(host=…, port=…) line. The values come from this setup
// (the local runtime, each server's tunnel port and its own bridge port).

import { call } from '../api.js';
import { $$, html, render, icon, copyText } from '../dom.js';
import { t, th } from '../i18n.js';

let el = null, cards = null, error = null;
let pick = null;       // selected card id
let where = 'here';    // here · server · other

async function refresh() {
  try { cards = await call('servers'); error = null; } catch (e) { error = String(e); }
  draw();
}

/** The 3-line test a person pastes where the bot will run. */
export function testCode(port) {
  return `from siliconmetatrader5 import MetaTrader5
mt5 = MetaTrader5(host="127.0.0.1", port=${port})
print(mt5.initialize(), mt5.last_error())`;
}

/** The tunnel a person opens on a computer without this app. */
export function tunnelCmd(local, remote, ssh) {
  return `ssh -N -L ${local}:127.0.0.1:${remote} ${ssh}`;
}

/** host/port and the rest of the answer for one card and one place. */
export function answer(c, place) {
  if (!c) return null;
  if (c.id === 'local') return { place: 'local', port: c.bridge_port };
  const remote = c.remote_bridge_port || 8001;
  if (place === 'server') return { place, port: remote };
  if (place === 'other') return { place, port: c.bridge_port, tunnel: tunnelCmd(c.bridge_port, remote, c.ssh || 'user@server') };
  return { place: 'here', port: c.bridge_port };
}

// ---- pieces

const box = (title, sub, cls = '', mono = true) => html`<div class="cn-box ${cls}"><b>${title}</b>${sub ? html`<span class="${mono ? 'mono' : ''}">${sub}</span>` : ''}</div>`;
const arrow = (label = '') => html`<div class="cn-arrow ${label ? 'tun' : ''}" aria-hidden="${label ? 'false' : 'true'}">${label ? html`<span>${label}</span>` : ''}</div>`;
const machine = (name, inner) => html`<div class="cn-machine"><div class="cn-mname">${name}</div><div class="cn-row">${inner}</div></div>`;

/** The path from the bot to MT5 for the chosen case, drawn with boxes. */
function diagram(c, a) {
  const remote = c.remote_bridge_port || 8001;
  const bridge = p => box(t('Köprü'), `127.0.0.1:${p}`, 'br');
  const mt5 = box('MetaTrader 5', '', 'mt');
  const bot = box(t('Botunuz'), '', 'bot');
  const srvName = c.name;
  let body;
  if (a.place === 'local') body = machine(t('Bu bilgisayar'), html`${bot}${arrow()}${bridge(a.port)}${arrow()}${mt5}`);
  else if (a.place === 'server') body = machine(srvName, html`${bot}${arrow()}${bridge(remote)}${arrow()}${mt5}`);
  else {
    const here = a.place === 'here' ? t('Bu bilgisayar') : t('Diğer bilgisayar');
    const door = a.place === 'here' ? t('uygulamanın tüneli') : t('sizin açtığınız tünel');
    body = html`${machine(here, html`${bot}${arrow()}${box(`127.0.0.1:${a.port}`, door, 'door', false)}`)}${arrow(t('SSH, şifreli'))}${machine(srvName, html`${bridge(remote)}${arrow()}${mt5}`)}`;
  }
  return html`<div class="cn-diagram" role="img" aria-label="${t('Botun MT5\'e giden yolu')}">${body}</div>`;
}

function whenLine(a, c) {
  switch (a.place) {
    case 'local': return t('Bu bilgisayarda MT5 çalıştığı sürece.');
    case 'here': return t('SiliconMetaTrader5 açık olduğu sürece (tüneli uygulama tutar). Uygulamayı kapatırsanız bot bağlantıyı kaybeder; açınca kendiliğinden geri gelir.');
    case 'server': return t('Her zaman: bu bilgisayar kapalıyken bile. 7/24 çalışacak botlar için önerilen yol.');
    default: return t('O bilgisayarda SSH komutu açık kaldığı sürece. Komutu kapatırsanız bağlantı kesilir.');
  }
}

function steps(a, c, reg) {
  const out = [];
  if (a.place === 'server') out.push(html`<li>${th('Botu sunucuya kopyalayın ve orada Python ile <code>pip install siliconmetatrader5</code> kurun.')}</li>`);
  else out.push(html`<li>${th('Botun çalışacağı yerde <code>pip install siliconmetatrader5</code> kurulu olsun.')}</li>`);
  if (a.tunnel) out.push(html`<li>${t('O bilgisayarda bir Terminal açıp bu komutu çalıştırın ve açık bırakın:')}
    ${code(a.tunnel, reg)}
    <div class="muted" style="font-size:12px">${t('O bilgisayarın da sunucuya SSH ile girebilmesi gerekir (anahtar veya şifre). {p} orada doluysa başka bir sayı seçin; botta da aynı sayıyı yazın.', { p: a.port })}</div></li>`);
  out.push(html`<li>${th('Botunuzun kodunda bağlantı satırını bulun ve <b>host</b> ile <b>port</b>\'u yukarıdaki değerlerle değiştirin.')}</li>`);
  out.push(html`<li>${t('Emin olmak için önce bu üç satırı aynı yerde çalıştırın. True görürseniz bağlantı tamamdır:')}${code(testCode(a.port), reg)}</li>`);
  return html`<ol class="rules">${out}</ol>`;
}

function code(text, reg) {
  const id = reg(text);
  return html`<div class="codebox"><button class="btn sm ib cp" data-copy="${id}" aria-label="${t('Kodu kopyala')}" title="${t('Kodu kopyala')}">${icon('copy')}</button><pre class="code selectable">${text}</pre></div>`;
}

const PLACES = [
  ['here', 'laptop', 'Bu bilgisayarda', 'Bot bu bilgisayarda, MT5 sunucuda.'],
  ['server', 'server', 'Sunucunun içinde', 'Bot da MT5\'in yanında, sunucuda.'],
  ['other', 'terminal', 'Başka bir bilgisayarda', 'Bu uygulamanın kurulu olmadığı bir makine.'],
];

function draw() {
  if (!el) return;
  const list = cards || [];
  if (!pick || !list.some(c => c.id === pick)) pick = (list.find(c => c.active) || list[0])?.id || null;
  const c = list.find(x => x.id === pick);
  const a = answer(c, where);
  const snippets = {};
  let n = 0;
  const reg = s => { const id = 'k' + n++; snippets[id] = s; return id; };
  const win = c?.mode === 'windows';
  render(el, html`
    <div class="head"><div><div class="crumbs">SiliconMetaTrader5 › ${t('Eğitim')} › <b>${t('Bot bağlantısı')}</b></div><h1>${t('Botumu nasıl bağlarım?')}</h1>
      <div class="muted" style="margin-top:4px;max-width:760px">${t('Botunuz MT5\'e köprü üzerinden bağlanır. Kodunuzda değiştireceğiniz şey yalnızca iki değerdir: adres (host) ve port. Hangi değeri yazacağınız, botun nerede çalıştığına bağlıdır.')}</div></div>
      <a class="btn" href="#/learn">${icon('book')}${t('Python API rehberi')}</a></div>

    <section class="card pad col">
      <div class="cn-keys">
        <div><span class="mono cn-big">127.0.0.1</span><div>${th('Her zaman bu adres. Anlamı: <b>botun çalıştığı makinenin kendisi</b>. Bot nerede çalışıyorsa orayı gösterir.')}</div></div>
        <div><span class="mono cn-big">port</span><div>${th('O makinede köprüye giden <b>kapının numarası</b>. Değişen tek şey budur.')}</div></div>
      </div>
    </section>

    ${error ? html`<div class="callout bad">${icon('alert')}<div>${error}</div></div>` : ''}
    ${cards && !list.length ? html`<div class="callout info">${icon('info')}<div>${t('Henüz kurulu bir MT5 yok. Önce Kurulum Sihirbazı\'ndan bu bilgisayara ya da bir sunucuya kurun.')} <a href="#/setup">${t('Kurulum Sihirbazı')}</a></div></div>` : ''}

    ${list.length ? html`
    <section class="card pad col">
      <h2>${t('1. Hangi MT5\'e bağlanacak?')}</h2>
      <div class="seg" role="group" aria-label="${t('MT5 seçin')}" id="cn-pick" style="flex-wrap:wrap;align-self:flex-start">
        ${list.map(x => html`<button data-pick="${x.id}" class="${x.id === pick ? 'sel' : ''}">${x.id === 'local' ? icon('laptop') : icon('server')} ${x.name}</button>`)}
      </div>
      ${c.id !== 'local' ? html`
      <h2 style="margin-top:8px">${t('2. Botunuz nerede çalışacak?')}</h2>
      <div class="cn-places" role="radiogroup" aria-label="${t('Botunuz nerede çalışacak?')}">
        ${PLACES.map(([id, ic, title, sub]) => html`<button class="cn-place ${where === id ? 'sel' : ''}" role="radio" aria-checked="${where === id}" data-where="${id}">
          ${icon(ic)}<b>${t(title)}</b><span class="muted">${t(sub)}</span>${id === 'server' ? html`<span class="chip up">${t('7/24 için önerilen')}</span>` : ''}</button>`)}
      </div>` : ''}
    </section>

    <section class="card pad col" id="cn-answer" data-place="${a.place}" data-port="${a.port}">
      <h2>${t('Botunuza yazacağınız değerler')}</h2>
      <div class="cn-vals">
        <div><span class="muted">host</span><span class="mono cn-big selectable">127.0.0.1</span></div>
        <div><span class="muted">port</span><span class="mono cn-big selectable" id="cn-port">${a.port}</span></div>
        <button class="btn sm" data-copy="${reg(`host="127.0.0.1", port=${a.port}`)}">${icon('copy')}${t('Kopyala')}</button>
      </div>
      ${diagram(c, a)}
      <div class="callout info">${icon('clock')}<div><b>${t('Ne zaman çalışır?')}</b> ${whenLine(a, c)}</div></div>
      ${a.place === 'here' && c.link && c.link.state !== 'connected' ? html`<div class="callout warn">${icon('alert')}<div>${t('Şu an bu sunucuya tünel kurulu değil; düzelene kadar bot bağlanamaz.')} <a href="#/servers">${t('Sunucular')}</a></div></div>` : ''}
      ${steps(a, c, reg)}
      ${a.place === 'server' && win ? html`<div class="callout warn" id="cn-win">${icon('info')}<div>${th('<b>Windows sunucuda iki yol var.</b> siliconmetatrader5 paketi (köprü, port {p}) her zaman çalışır. MetaQuotes\'un resmi <code>MetaTrader5</code> paketi ise yalnızca MT5 ile <b>aynı Windows oturumunda</b> başlatılan bir botta çalışır; SSH\'tan ya da zamanlanmış görevden başlatılan bot ona ulaşamaz. Emin değilseniz köprüyü kullanın.', { p: a.port })}</div></div>` : ''}
    </section>

    <section class="card pad col">
      <h2>${t('Bot resmi MetaTrader5 paketiyle mi yazıldı?')}</h2>
      <p class="muted" style="margin:0">${t('Yalnızca bağlantı satırları değişir; geri kalan kod (copy_rates, order_send …) aynı kalır.')}</p>
      ${code(`# ${t('önce')}\nimport MetaTrader5 as mt5\nmt5.initialize()\n\n# ${t('sonra')}\nfrom siliconmetatrader5 import MetaTrader5\nmt5 = MetaTrader5(host="127.0.0.1", port=${a.port})\nmt5.initialize()`, reg)}
    </section>

    ${c.id !== 'local' ? html`
    <section class="card pad col" id="cn-why">
      <h2>${t('Neden sunucunun IP adresini yazmıyorum?')}</h2>
      <p style="margin:0;line-height:1.6">${th('Köprü emir gönderebildiği için <b>internete kapalıdır</b>: yalnızca sunucunun kendi içinden (127.0.0.1) ulaşılır. Botunuza sunucunun IP adresini yazarsanız bağlantı reddedilir; bu bir hata değil, koruma.')}</p>
      <p style="margin:0;line-height:1.6">${th('Dışarıdan ulaşmanın yolu <b>SSH tüneli</b>dir. Tünel, bilgisayarınızdaki bir kapıyı ({lp}) şifreli bir yoldan sunucunun içindeki köprüye ({rp}) bağlar. Botunuz 127.0.0.1 yazar ama aslında sunucudaki MT5 ile konuşur.', { lp: c.bridge_port, rp: c.remote_bridge_port || 8001 })}</p>
    </section>` : ''}` : ''}`);
  $$('[data-copy]', el).forEach(b => b.onclick = e => { e.preventDefault(); copyText(snippets[b.dataset.copy]); });
  $$('[data-pick]', el).forEach(b => b.onclick = () => { pick = b.dataset.pick; draw(); });
  $$('[data-where]', el).forEach(b => b.onclick = () => { where = b.dataset.where; draw(); });
}

export default {
  mount(node) { el = node; draw(); refresh(); },
  update(what) { if (what === 'info') refresh(); },
  unmount() { el = null; },
};
