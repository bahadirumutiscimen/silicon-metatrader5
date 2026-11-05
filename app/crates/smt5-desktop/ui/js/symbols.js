// Symbol picker shared by Data Test and the strategy page: Market Watch symbols
// plus symbols typed earlier; "Other…" asks in a dialog, so the list never goes away.

import { html, modal } from './dom.js';
import { t } from './i18n.js';

const KEY = 'smt5.extraSymbols';
function load() { try { return JSON.parse(localStorage.getItem(KEY) || '[]').filter(s => typeof s === 'string'); } catch { return []; } }
function save(list) { try { localStorage.setItem(KEY, JSON.stringify(list.slice(-20))); } catch { /* private mode */ } }

/** <select> with the Market Watch symbols, earlier custom ones and "Other…". */
export function symbolSelect(id, watchSymbols, current) {
  const names = [...new Set([...(watchSymbols || []), ...load(), current].filter(Boolean))];
  return html`<select class="f" id="${id}" style="min-width:150px">${names.map(n => html`<option ${n === current ? 'selected' : ''}>${n}</option>`)}<option value="__other">${t('Diğer…')}</option></select>`;
}

/** Ask for a symbol name; resolves to the name (remembered) or null. */
export async function promptSymbol(current = '') {
  let input = null;
  const ok = await modal({
    title: t('Sembol adı'),
    body: html`<p class="muted" style="margin:0;font-size:13px">${t("Broker'daki tam adı yaz (ör. EURUSD.m, XAUUSD, US500.cash). Market Watch'ta yoksa SiliconMetaTrader5 onu ekler.")}</p>
      <input class="f" id="sym-ask" value="${current}" placeholder="EURUSD" autocapitalize="characters" spellcheck="false" style="width:100%">`,
    buttons: [{ label: t('Vazgeç'), value: false }, { label: t('Seç'), value: true, primary: true }],
    onMount: bg => {
      input = bg.querySelector('#sym-ask');
      input.focus(); input.select();
      input.addEventListener('keydown', e => { if (e.key === 'Enter') bg.querySelector('button.p')?.click(); });
    },
  });
  const name = ok && input ? input.value.trim() : '';
  if (!name) return null;
  const list = load().filter(s => s !== name); list.push(name); save(list);
  return name;
}

/** Wire a select made by symbolSelect: calls onPick(name) after a change. */
export function bindSymbolSelect(sel, current, onPick) {
  if (!sel || sel.tagName !== 'SELECT') return;
  sel.onchange = async () => {
    if (sel.value !== '__other') return onPick(sel.value);
    const name = await promptSymbol(current());
    if (name) onPick(name); else sel.value = current();
  };
}
