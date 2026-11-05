// UI language. Turkish is the source language: t('Türkçe metin') returns the
// English translation from i18n-en.js when the language is English, the text
// itself otherwise. {name} placeholders are filled from `vars`.
// th() is the same for texts with simple markup (<b>, <code>): the text is
// trusted, the vars are escaped, and the result goes into html`` unescaped.

import EN from './i18n-en.js';

let lang = 'tr';
let host = 'macos';
const misses = new Set();

// The texts are written for macOS; on Linux the native and Docker modes get
// their own names (Mac yerel → Linux yerel, Güvenli → Docker) and the macOS
// details (Dock, launchd, Mac clipboard) the Linux ones.
const LINUX = {
  tr: [[/Mac yerel/g, 'Linux yerel'], [/Güvenli moddan/g, 'Docker modundan'], [/Güvenli modda/g, 'Docker modunda'], [/Güvenli moda/g, 'Docker moduna'],
    [/Güvenli modun/g, 'Docker modunun'], [/Güvenli mod/g, 'Docker modu'], [/macOS penceresinde/g, 'masaüstü penceresinde'], [/macOS penceresi/g, 'masaüstü penceresi'],
    [/Mac panosuyla/g, 'masaüstü panosuyla'], [/ \(Dock'ta simgesi görünür\)/g, ''], [/Dock'ta MetaTrader 5 simgesi görünür\. /g, ''], [/\(launchd\)/g, '(systemd)']],
  en: [[/Mac native/g, 'Linux native'], [/Safe mode/g, 'Docker mode'], [/macOS window/g, 'desktop window'], [/Mac clipboard/g, 'desktop clipboard'],
    [/ \(its icon shows in the Dock\)/g, ''], [/The MetaTrader 5 icon shows in the Dock\. /g, ''], [/\(launchd\)/g, '(systemd)']],
};
export function setHost(os) { host = os === 'linux' ? 'linux' : 'macos'; }
export const getHost = () => host;

export function setLang(l) {
  lang = l === 'en' ? 'en' : 'tr';
  document.documentElement.lang = lang;
}
export const getLang = () => lang;
export const locale = () => (lang === 'en' ? 'en-US' : 'tr-TR');
/** Texts without an English entry (checked by the screenshot test). */
export const missing = () => [...misses];

function lookup(tr) {
  let s = tr;
  if (lang === 'en') {
    if (tr in EN) s = EN[tr];
    else misses.add(tr);
  }
  if (host === 'linux') for (const [re, to] of LINUX[lang === 'en' ? 'en' : 'tr']) s = s.replace(re, to);
  return s;
}
const fill = (s, vars, f) => (vars ? s.replace(/\{(\w+)\}/g, (m, k) => (k in vars ? f(vars[k]) : m)) : s);
const escape = v => String(v ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

export function t(tr, vars) { return fill(lookup(tr), vars, String); }
export function th(tr, vars) { return { __raw: true, s: fill(lookup(tr), vars, escape) }; }
if (typeof window !== 'undefined') window.__i18nMissing = missing;
