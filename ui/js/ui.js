// Small DOM helpers: no framework, no build step.
export function h(tag, attrs, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v == null || v === false) continue;
    if (k === 'class') el.className = v;
    else if (k === 'html') el.innerHTML = v;
    else if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
    else if (k === 'style' && typeof v === 'object') Object.assign(el.style, v);
    else el.setAttribute(k, v === true ? '' : v);
  }
  for (const kid of kids.flat(Infinity)) { if (kid == null || kid === false) continue; el.append(kid.nodeType ? kid : document.createTextNode(String(kid))); }
  return el;
}
export const esc = s => String(s ?? '').replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));

export function toast(msg, kind = 'info', ms = 4200) {
  if (String(msg).startsWith('PREMIUM:')) return; // the shell shows one upsell prompt instead
  const t = h('div', { class: 'toast ' + kind }, msg);
  document.getElementById('toasts').append(t);
  setTimeout(() => { t.classList.add('out'); setTimeout(() => t.remove(), 350); }, ms);
}

export function modal({ title, body, actions }) {
  return new Promise(resolve => {
    const root = document.getElementById('modal-root');
    const close = v => { bg.remove(); resolve(v); };
    const btns = (actions || [{ label: 'OK', value: true, primary: true }]).map(a =>
      h('button', { class: 'btn' + (a.primary ? ' primary' : '') + (a.danger ? ' danger' : ''), onclick: () => close(a.value) }, a.label));
    const bg = h('div', { class: 'modal-bg', onclick: e => { if (e.target === bg) close(null); } },
      h('div', { class: 'modal' }, h('h3', {}, title), h('div', { class: 'body-text' }, body), h('div', { class: 'actions' }, btns)));
    root.append(bg);
  });
}
export const confirmBox = (title, body, yes = 'Continue') => modal({ title, body, actions: [{ label: 'Cancel', value: false }, { label: yes, value: true, primary: true }] });

export function bytes(n) {
  n = Number(n) || 0; const u = ['B', 'KB', 'MB', 'GB', 'TB']; let i = 0;
  while (n >= 1024 && i < u.length - 1) { n /= 1024; i++; }
  return (n >= 100 || i === 0 ? n.toFixed(0) : n.toFixed(1)) + ' ' + u[i];
}
export function countUp(el, to, ms = 1100) {
  const t0 = performance.now();
  const step = t => { const p = Math.min(1, (t - t0) / ms), e = 1 - Math.pow(1 - p, 3); el.textContent = Math.round(to * e); if (p < 1) requestAnimationFrame(step); };
  requestAnimationFrame(step);
}
export function ring(score, color = 'var(--accent)') {
  const r = 70, c = 2 * Math.PI * r;
  const wrap = h('div', { class: 'ring' });
  wrap.innerHTML = `<svg viewBox="0 0 168 168"><circle class="track" cx="84" cy="84" r="${r}" fill="none" stroke-width="12"/><circle class="val" cx="84" cy="84" r="${r}" fill="none" stroke="${color}" stroke-width="12" stroke-dasharray="${c}" stroke-dashoffset="${c}"/></svg><div class="num"><div><b>0</b><span>of 100</span></div></div>`;
  const val = wrap.querySelector('.val'), num = wrap.querySelector('b');
  requestAnimationFrame(() => requestAnimationFrame(() => { val.style.strokeDashoffset = c * (1 - score / 100); countUp(num, score); }));
  return wrap;
}
export function sparkline(values, max = 100) {
  const w = 300, hgt = 46, n = Math.max(values.length, 2);
  const pts = values.map((v, i) => [i * (w / (n - 1)), hgt - 3 - (Math.min(v, max) / max) * (hgt - 6)]);
  const line = pts.map((p, i) => (i ? 'L' : 'M') + p[0].toFixed(1) + ' ' + p[1].toFixed(1)).join(' ');
  const el = h('div');
  el.innerHTML = `<svg class="spark" viewBox="0 0 ${w} ${hgt}" preserveAspectRatio="none"><defs><linearGradient id="sg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#22d3ee"/><stop offset="1" stop-color="#22d3ee" stop-opacity="0"/></linearGradient></defs><path class="fill" d="${line} L${w} ${hgt} L0 ${hgt} Z"/><path class="line" d="${line}"/></svg>`;
  return el;
}
export function meter(pct) { return h('div', { class: 'meter' }, h('i', { style: { width: Math.max(0, Math.min(100, pct)) + '%' } })); }
export const badge = (risk) => h('span', { class: 'badge b-' + risk }, { recommended: 'Recommended', caution: 'Caution', experimental: 'Experimental', highRisk: 'High risk' }[risk] || risk);
export function switchEl(on, onchange) {
  const s = h('button', { class: 'switch' + (on ? ' on' : ''), role: 'switch', 'aria-checked': on ? 'true' : 'false' });
  s.addEventListener('click', async () => { if (s.disabled) return; await onchange(s); });
  s.set = v => { s.classList.toggle('on', v); s.setAttribute('aria-checked', v ? 'true' : 'false'); };
  return s;
}
export function skeleton(n = 4, height = 64) { return h('div', { class: 'stack' }, Array.from({ length: n }, () => h('div', { class: 'skeleton', style: { height: height + 'px', borderRadius: '16px' } }))); }
export function page(title, sub, ...content) {
  return h('div', { class: 'page stagger' }, h('div', { class: 'eyebrow' }, 'NEOX'), h('h2', { class: 'page-title', style: { '--i': 0 } }, title), sub && h('p', { class: 'page-sub', style: { '--i': 1 } }, sub), content.flat().map((c, i) => { if (c && c.style) c.style.setProperty('--i', i + 2); return c; }));
}
export function attachSpotlight(root) {
  root.addEventListener('pointermove', e => {
    const c = e.target.closest && e.target.closest('.card'); if (!c) return;
    const r = c.getBoundingClientRect(); c.style.setProperty('--mx', (e.clientX - r.left) + 'px'); c.style.setProperty('--my', (e.clientY - r.top) + 'px');
  });
  root.addEventListener('pointerdown', e => {
    const b = e.target.closest && e.target.closest('.btn'); if (!b) return;
    const r = b.getBoundingClientRect(), s = Math.max(r.width, r.height) * 2;
    const rp = h('span', { class: 'ripple', style: { width: s + 'px', height: s + 'px', left: (e.clientX - r.left - s / 2) + 'px', top: (e.clientY - r.top - s / 2) + 'px' } });
    b.append(rp); setTimeout(() => rp.remove(), 650);
  });
}
export async function busy(btn, fn) {
  const t = btn.textContent; btn.disabled = true;
  try { return await fn(); } finally { btn.disabled = false; btn.textContent = t; }
}
export function fmtTime(iso) { const d = new Date(iso); return isNaN(d) ? '' : d.toLocaleString(); }
export const CAT_NAMES = { performance: 'Performance', input: 'Input', network: 'Network', privacy: 'Privacy', services: 'Services', appearance: 'Appearance', cleanup: 'Cleanup', tools: 'Tools' };
export const EFFECT = { unknown: 'Effect: unmeasured', large: 'Claimed effect: large', medium: 'Claimed effect: medium', hardwareDependent: 'Depends on hardware', small: 'Claimed effect: small', negligible: 'Effect: negligible', tool: 'Tool' };

export const LOCK_SVG = '<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="4" y="11" width="16" height="10" rx="2"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/></svg>';
export function lockButton(go, label = 'Premium') { return h('button', { class: 'btn sm gold', html: LOCK_SVG + '<span>' + label + '</span>', onclick: () => go('premium') }); }
export const premiumBadge = () => h('span', { class: 'badge b-premium' }, 'Premium');

// Extra text for the confirmation dialog of risky items.
export function riskWarning(t) {
  if (t.risk === 'highRisk') return '⚠ HIGH RISK. This can lower your security or make Windows unstable. Create a restore point first (Dashboard) and read the details below.\n\n';
  if (t.risk === 'experimental') return '⚠ EXPERIMENTAL. The effect depends on your hardware and can be none or even negative. Revert it if anything gets worse.\n\n';
  return '';
}
