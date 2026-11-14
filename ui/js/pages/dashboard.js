import { call, on } from '../api.js';
import { h, page, ring, sparkline, meter, bytes, toast, confirmBox } from '../ui.js';
import { S, refreshInfo } from '../state.js';

export default async function dashboard({ go }) {
  const wrap = page('Dashboard', 'Your PC at a glance. Nothing here changes anything until you press a button.');
  const healthBox = h('div', { class: 'card hero row', style: { gap: '28px' } }, h('div', { class: 'skeleton', style: { width: '168px', height: '168px', borderRadius: '50%' } }));
  const checks = h('div', { class: 'stack' });
  const live = h('div', { class: 'grid g2' });
  const cpuV = h('b', { class: 'big-num', style: { fontSize: '34px' } }, '–'), ramV = h('b', { class: 'big-num', style: { fontSize: '34px' } }, '–');
  const cpuSpark = h('div'), ramBar = h('div', {}, meter(0)), cpuHist = [];
  live.append(
    h('div', { class: 'card hover' }, h('div', { class: 'eyebrow' }, 'CPU'), cpuV, cpuSpark),
    h('div', { class: 'card hover' }, h('div', { class: 'eyebrow' }, 'Memory'), ramV, h('div', { class: 'small muted', id: 'ram-detail' }, ' '), ramBar));
  wrap.append(healthBox, live, h('h3', { class: 'h2' }, 'Health checks'), checks);

  call('monitor_start');
  const off = await on('monitor', s => {
    if (!document.body.contains(wrap)) { call('monitor_stop'); return; }
    cpuHist.push(s.cpuPercent); if (cpuHist.length > 60) cpuHist.shift();
    cpuV.textContent = Math.round(s.cpuPercent) + '%'; cpuSpark.replaceChildren(sparkline(cpuHist));
    ramV.textContent = s.ramPercent + '%'; ramBar.replaceChildren(meter(s.ramPercent));
    wrap.querySelector('#ram-detail').textContent = `${bytes(s.ramUsedBytes)} of ${bytes(s.ramTotalBytes)}`;
  });

  call('health_scan').then(rep => {
    const color = rep.score >= 80 ? 'var(--good)' : rep.score >= 55 ? 'var(--accent)' : 'var(--warn)';
    healthBox.replaceChildren(ring(rep.score, color), h('div', { class: 'grow' },
      h('div', { class: 'eyebrow' }, 'System health'), h('h1', {}, rep.label),
      h('p', { class: 'muted' }, 'The score comes from six real checks below, with fixed weights. No made-up numbers.'),
      h('div', { class: 'row wrap', style: { marginTop: '14px' } },
        h('button', { class: 'btn primary', onclick: () => go('autotune') }, 'Run Auto-Tune'),
        h('button', { class: 'btn', onclick: () => go('optimizations') }, 'Browse optimizations'))));
    rep.checks.forEach(c => checks.append(h('div', { class: 'card hover row' },
      h('i', { class: 'dot ' + (c.ok ? 'on' : 'busy'), style: { marginTop: 0 } }),
      h('div', { class: 'grow' }, h('b', {}, c.name), h('div', { class: 'muted small' }, c.detail)),
      fixButton(c, go))));
  }).catch(e => healthBox.replaceChildren(h('div', { class: 'empty' }, 'Health scan failed: ' + e)));
  return wrap;
}

function fixButton(c, go) {
  if (c.ok || c.fix === 'none') return null;
  const map = { applySafeProfile: ['Apply safe set', async () => {
      if (!(await confirmBox('Apply the Safe profile?', 'Only low-risk, reversible tweaks. A restore point is created first when possible, and everything can be undone from History.'))) return;
      const r = await call('profile_apply', { id: 'builtin.safe', gameExe: null }); toast(`Applied ${r.applied}, failed ${r.failed}.`, r.failed ? 'err' : 'ok'); go('dashboard'); }],
    openStartup: ['Open startup', () => go('startup')], openDisk: ['Analyze disk', () => go('disk')],
    restart: ['Restart later', () => toast('Restart Windows when convenient.', 'info')] };
  const m = map[c.fix]; if (!m) return null;
  return h('button', { class: 'btn sm', onclick: () => m[1]().catch(e => toast(String(e), 'err')) }, m[0]);
}
