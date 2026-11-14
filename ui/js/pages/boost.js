import { call, on } from '../api.js';
import { h, page, toast, busy, bytes, meter, lockButton, premiumBadge, confirmBox, fmtTime } from '../ui.js';
import { S, refreshLicense } from '../state.js';

const METRICS = [
  ['cpuOpsPerSec', 'CPU', 'Mops/s', true, 5, v => (v / 1e6).toFixed(1)],
  ['memoryMbps', 'Memory bandwidth', 'MB/s', true, 5, v => Math.round(v).toLocaleString()],
  ['timerAvgMs', 'Timer average', 'ms', false, 10, v => v.toFixed(3)],
  ['timerP99Ms', 'Timer p99 (stutter)', 'ms', false, 10, v => v.toFixed(3)],
];

export default async function boost({ go }) {
  await refreshLicense();
  const P = S.lic.premium;
  return page('Boost', 'Two Premium tools: free up RAM on demand, and track how your PC performs after each change.', memoryCard(P, go), scoreCard(P, go));
}

function memoryCard(P, go) {
  const out = h('div', { class: 'stack' });
  const run = h('button', { class: 'btn gold' }, 'Clean memory now');
  run.onclick = () => busy(run, async () => {
    run.textContent = 'Cleaning…'; out.replaceChildren();
    try {
      const r = await call('memory_clean');
      out.append(h('div', { class: 'delta' }, h('div', { class: 'grow' }, h('b', {}, 'Free RAM'), h('div', { class: 'small muted' }, `${bytes(r.beforeFree)} → ${bytes(r.afterFree)} of ${bytes(r.total)}`)),
        h('div', { class: 'pct ' + (r.freed > 0 ? 'up' : 'flat') }, '+' + bytes(r.freed))),
        meter(100 - r.afterFree / r.total * 100),
        h('p', { class: 'small muted' }, 'This empties idle working sets and the standby cache. Windows refills the cache as you use programs, so the effect fades; use it before a game or a heavy task, not on a timer.'));
    } catch (e) { toast(String(e), 'err', 6000); }
  });
  return h('div', { class: 'card col', style: { gap: '12px', borderColor: 'rgba(245,199,107,.3)' } },
    h('div', { class: 'row' }, h('div', { class: 'grow' }, h('div', { class: 'h2' }, 'Memory Cleaner ', P ? null : premiumBadge()),
      h('div', { class: 'small muted' }, 'One click empties idle working sets and purges the standby list, giving a big block of RAM back at once. Helps when memory is full and games stutter after hours of uptime.'))),
    h('div', { class: 'row' }, P ? run : [lockButton(go, 'Memory Cleaner is Premium')]), out);
}

function scoreCard(P, go) {
  const body = h('div', { class: 'stack' }), status = h('div', { class: 'small muted mono' });
  on('progress', m => { if (document.body.contains(status)) status.textContent = m; });
  const label = h('input', { class: 'btn', placeholder: 'Label, e.g. "after Auto-Tune"', style: { width: '260px', textAlign: 'left', userSelect: 'text' } });
  const run = h('button', { class: 'btn gold' }, 'Measure now');
  const draw = list => {
    if (!list.length) { body.replaceChildren(h('div', { class: 'empty' }, 'No measurements yet. Take one now, change something, then measure again.')); return; }
    const first = list[0], rows = [...list].reverse().map((e, i, arr) => {
      const prev = arr[i + 1];
      return h('div', { class: 'card col', style: { gap: '6px' } }, h('div', { class: 'row' }, h('b', { class: 'grow' }, e.label), h('span', { class: 'small muted' }, fmtTime(e.time * 1000))),
        h('div', { class: 'grid g2' }, METRICS.map(([k, name, unit, hib, noise, fmt]) => {
          const vs = prev || (e === first ? null : first); let tag = null;
          if (vs && vs[k] > 0) { const pc = (e[k] - vs[k]) / vs[k] * 100, better = hib ? pc > noise : pc < -noise, worse = hib ? pc < -noise : pc > noise;
            tag = h('span', { class: 'badge ' + (better ? 'b-recommended' : worse ? 'b-highRisk' : 'b-info') }, (pc > 0 ? '+' : '') + pc.toFixed(1) + '%' + (better || worse ? '' : ' noise')); }
          return h('div', { class: 'row' }, h('span', { class: 'muted small grow' }, name), h('b', {}, fmt(e[k]) + ' ' + unit), tag); })));
    });
    body.replaceChildren(...rows, h('button', { class: 'btn sm danger', onclick: async () => { await call('score_clear'); draw([]); } }, 'Clear history'));
  };
  run.onclick = () => busy(run, async () => {
    run.textContent = 'Measuring…';
    try { draw(await call('score_run', { label: label.value })); status.textContent = ''; label.value = ''; } catch (e) { toast(String(e), 'err'); }
  });
  call('score_list').then(draw).catch(() => draw([]));
  return h('div', { class: 'card col', style: { gap: '12px', borderColor: 'rgba(245,199,107,.3)' } },
    h('div', { class: 'h2' }, 'Performance Score ', P ? null : premiumBadge()),
    h('div', { class: 'small muted' }, 'Runs short CPU, memory and timer tests and keeps every result. Each run is compared with the previous one, so you see what a tweak really changed. Gains inside normal run-to-run variation are labelled noise.'),
    h('div', { class: 'row wrap' }, P ? [label, run] : [lockButton(go, 'Performance Score is Premium')], status), body);
}
