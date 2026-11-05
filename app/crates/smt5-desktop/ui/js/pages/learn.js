// Education › Python API: quick start, rules learned the hard way, every
// function of the bridge with a copyable example, and the main constants.

import { $, $$, html, render, icon, raw, copyText } from '../dom.js';
import { state } from '../store.js';
import { t, getLang } from '../i18n.js';
import { QUICKSTART, RULES, GROUPS, CONSTANTS, SSH_GUIDE } from '../learn-data.js';

let el, query = '';
const L = o => o[getLang()] || o.tr;

function codeBlock(code, id) {
  return html`<div class="codebox"><button class="btn sm ib cp" data-copy="${id}" aria-label="${t('Kodu kopyala')}" title="${t('Kodu kopyala')}">${icon('copy')}</button><pre class="code selectable">${code}</pre></div>`;
}

function draw() {
  if (!el) return;
  const q = query.trim().toLowerCase();
  const groups = GROUPS.map(gr => ({ ...gr, items: gr.items.filter(it => !q || it.name.toLowerCase().includes(q) || L(it.desc).toLowerCase().includes(q)) }))
    .filter(gr => gr.items.length);
  const port = state.snap?.bridge_port || 8001;
  const snippets = {};
  let n = 0;
  const reg = code => { const id = 'c' + n++; snippets[id] = code; return id; };
  const quick = QUICKSTART.replace('port=8001', `port=${port}`);
  render(el, html`
    <div class="head"><div><div class="crumbs">SiliconMetaTrader5 › ${t('Eğitim')} › <b>${t('Python API')}</b></div><h1>${t('Python API rehberi')}</h1>
      <div class="muted" style="margin-top:4px;max-width:760px">${t("siliconmetatrader5, MetaQuotes'un resmi MetaTrader5 paketindeki fonksiyonların aynısını köprü üzerinden sunar: aynı isimler, aynı parametreler, aynı dönüş değerleri. Aşağıdaki her örnek kopyalanıp doğrudan çalıştırılabilir.")}</div></div>
      <div class="row" style="gap:8px"><a class="btn" href="#/connect">${icon('plug')}${t('Botumu nasıl bağlarım?')}</a><a class="btn" href="#/strategy">${icon('trend')}${t('Örnek strateji: Supertrend')}</a></div></div>

    <section class="card pad col"><div class="row between wrap"><h2>${t('Hızlı başlangıç')}</h2><span class="chip neu mono">pip install siliconmetatrader5</span></div>
      ${codeBlock(quick, reg(quick))}</section>
    <section class="card pad col"><h2>${t('Kurallar')}</h2>
      <ol class="rules cols">${RULES.map(r => html`<li>${raw(L(r))}</li>`)}</ol></section>
    <section class="card pad col" id="ln-ssh"><h2>${t('Sunucuya SSH ile bağlanma')}</h2>
      <ol class="rules cols">${SSH_GUIDE.map(r => html`<li>${raw(L(r))}</li>`)}</ol></section>

    <section class="card pad col">
      <div class="row between wrap"><h2>${t('Fonksiyonlar')}</h2>
        <input class="f" id="ln-q" type="search" placeholder="${t('Ara: copy_rates, order, tick…')}" value="${query}" style="width:280px" aria-label="${t('Fonksiyon ara')}"></div>
      ${groups.length ? groups.map(gr => html`<div class="fgroup"><div class="sec2">${L(gr.title)}</div>
        ${gr.items.map(it => html`<details class="fn" ${q ? 'open' : ''}><summary><span class="mono fname">${it.name}</span>${it.extra ? html`<span class="chip acc" style="height:18px">siliconmetatrader5</span>` : ''}<span class="muted grow fdesc">${L(it.desc)}</span>${icon('chev')}</summary>
          <div class="col" style="gap:8px;padding:4px 0 12px"><code class="mono sig selectable">${it.sig}</code>${codeBlock(it.ex, reg(it.ex))}</div></details>`)}</div>`)
        : html`<p class="muted">${t('Eşleşen fonksiyon yok.')}</p>`}
    </section>

    <section class="card pad col"><h2>${t('Sık kullanılan sabitler')}</h2>
      <p class="muted" style="margin:0;font-size:13px">${t('Hepsi istemci nesnesinin özelliğidir: mt5.TIMEFRAME_H1, mt5.ORDER_TYPE_BUY …')}</p>
      <table><tbody>${CONSTANTS.map(([k, d]) => html`<tr><td class="mono selectable">${k}</td><td class="muted">${L(d)}</td></tr>`)}</tbody></table>
    </section>`);
  $$('[data-copy]', el).forEach(b => b.onclick = e => { e.preventDefault(); copyText(snippets[b.dataset.copy]); });
  const qi = $('#ln-q', el);
  qi.oninput = () => { query = qi.value; const pos = qi.selectionStart; draw(); const n2 = $('#ln-q', el); n2.focus(); n2.setSelectionRange(pos, pos); };
}

export default {
  mount(node) { el = node; draw(); },
  update(what) { if (what === 'info') draw(); },
  unmount() { el = null; },
};
