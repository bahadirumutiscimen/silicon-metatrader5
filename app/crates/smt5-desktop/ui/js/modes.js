// Runtime mode cards (macOS): the setup wizard's first step and Settings.
// Mac yerel is the recommended, larger card; Güvenli (QEMU) stays available
// as the smaller fallback. Each lists its advantages and drawbacks.

import { html, icon } from './dom.js';
import { t } from './i18n.js';
import { state } from './store.js';

const INFO = {
  native: {
    title: 'Mac yerel',
    sub: 'Wine 11 + Rosetta 2 · VM ve Docker yok',
    tag: 'Önerilen',
    pros: [
      'MT5 yaklaşık 20 saniyede açılır',
      "Python ~13×, veri aktarımı ~30× hızlı; köprü gecikmesi 1 ms'nin altında",
      'MT5 normal bir macOS penceresi; kopyala-yapıştır doğrudan çalışır',
      "Sanal makineye RAM ve disk ayırmaz",
    ],
    cons: [
      "Apple'ın Rosetta 2'sine bağlı; Apple, Rosetta'yı macOS 28'den (2027) itibaren kısıtlayacağını duyurdu",
      'Wine doğrudan Mac\'te çalışır; container kadar yalıtılmış değil',
      'Wine 11 topluluk derlemesi (Gcenx) kullanılır',
    ],
  },
  safe: {
    title: 'Güvenli (QEMU)',
    sub: 'Colima x86_64 sanal makine + container',
    tag: 'Yedek',
    pros: ["Mac'ten yalıtılmış container", "Rosetta'ya bağlı değil", 'Linux sunucudaki kurulumla aynı imaj'],
    cons: ['Tam emülasyon: belirgin şekilde yavaş, ilk kurulum 30–90 dk', "VM'e 8 GB RAM ve 60 GB disk ayrılır", 'MT5 ekranı noVNC üzerinden'],
  },
};

// Linux: both modes run Wine 11 natively on x86_64 (no emulation, same speed).
const LINUX_INFO = {
  native: {
    title: 'Linux yerel',
    sub: 'Wine 11 · Docker yok',
    tag: 'Masaüstü için',
    pros: [
      'MT5 masaüstünde normal bir pencere; kopyala-yapıştır doğrudan çalışır',
      'Docker gerekmez; Wine 11 uygulamanın kendi klasörüne iner (~100 MB)',
      'Ekransız sunucuda kendi noVNC ekranını açar',
      'macOS sürümündekiyle aynı Wine (11.21)',
    ],
    cons: [
      'Wine doğrudan sistemde çalışır; container kadar yalıtılmış değil',
      'Wine 11 topluluk derlemesi (Kron4ek) kullanılır',
      'Ekransız sunucuda xvfb, x11vnc, websockify ve novnc paketleri gerekir',
    ],
  },
  safe: {
    title: 'Docker',
    sub: 'Wine 11 container + noVNC',
    tag: 'Sunucu için',
    pros: ['Sistemden yalıtılmış container', 'Sürümler sabit: her sunucuda aynı imaj', 'Ekransız sunucular için hazır (noVNC)'],
    cons: ['Docker Engine gerekir', 'İmaj ilk kez derlenir ya da indirilir (~10–25 dk)', 'MT5 ekranı noVNC üzerinden'],
  },
};

function card(m, disabled, big) {
  const i = (state.info?.os === 'linux' ? LINUX_INFO : INFO)[m.id] || { title: m.label.split(' · ')[0], sub: m.label.split(' · ')[1] || '', pros: [], cons: [] };
  return html`<button class="choice mode ${big ? 'big' : ''} ${m.current ? 'sel' : ''}" data-mode="${m.id}" ${m.current || !m.available || disabled ? 'disabled' : ''}>
    <div class="grow col" style="gap:${big ? 10 : 6}px">
      <div class="row between"><div><b class="mt">${t(i.title)}</b> ${i.tag ? html`<span class="chip ${big ? 'up' : 'neu'}" style="height:18px">${t(i.tag)}</span>` : ''}
        <div class="muted" style="font-size:12px;margin-top:2px">${t(i.sub)}</div></div>
        ${m.current ? html`<span class="chip up">${t('Seçili')}</span>` : ''}</div>
      <ul class="pc pro">${i.pros.map(p => html`<li>${icon('check')}<span>${t(p)}</span></li>`)}</ul>
      <ul class="pc con">${i.cons.map(p => html`<li>${icon('x')}<span>${t(p)}</span></li>`)}</ul>
      ${m.hint ? html`<div class="muted mono" style="font-size:11px">${m.hint}</div>` : ''}
    </div></button>`;
}

/** Mac yerel first and wide, Güvenli second and narrow (stacked on small windows). */
export function modeCards(modes, { disabled = false } = {}) {
  const order = ['native', 'safe'];
  const sorted = [...modes].sort((a, b) => order.indexOf(a.id) - order.indexOf(b.id));
  return html`<div class="modes">${sorted.map(m => card(m, disabled, m.id === 'native'))}</div>`;
}
