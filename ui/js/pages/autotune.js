import { call, on } from '../api.js';
import { h, page, toast, confirmBox, busy, skeleton, lockButton } from '../ui.js';
import { S, refreshInfo, refreshLicense } from '../state.js';

export default async function autotune({ go }) {
  const root = h('div', { class: 'stack' }, skeleton(3, 70));
  const wrap = page('Auto-Tune', 'Scans your hardware, applies only safe tweaks that suit it, and — if you want — measures before and after. Gains smaller than the noise level are reported as noise, not as wins.', root);
  const status = h('div', { class: 'muted small mono' });
  await on('progress', m => { if (document.body.contains(status)) status.textContent = m; });

  await refreshLicense();
  const view = await call('autotune_plan');
  const plan = view.plan; const chosen = new Set(plan.planned.map(p => p.id));
  const bench = h('input', { type: 'checkbox', checked: true, id: 'bench' });
  const list = h('div', { class: 'stack' }, plan.planned.map(p => h('label', { class: 'tw', style: { cursor: 'pointer' } },
    h('input', { type: 'checkbox', checked: true, onchange: e => e.target.checked ? chosen.add(p.id) : chosen.delete(p.id) }),
    h('div', { class: 'grow' }, h('div', { class: 'title' }, p.title), h('div', { class: 'desc' }, p.reason)))));
  const skipped = plan.skipped.length ? h('details', { class: 'card' }, h('summary', { class: 'h2', style: { cursor: 'pointer' } }, `Skipped on purpose (${plan.skipped.length})`),
    h('div', { class: 'stack', style: { marginTop: '10px' } }, plan.skipped.map(s => h('div', { class: 'desc muted' }, h('b', {}, s.title + ' — '), s.reason)))) : null;
  const results = h('div', { class: 'stack' });
  const run = h('button', { class: 'btn primary lg' }, 'Apply & measure');
  run.onclick = () => busy(run, async () => {
    if (!chosen.size) return toast('Select at least one tweak.', 'err');
    if (!(await confirmBox('Run Auto-Tune?', `${chosen.size} tweak(s) will be applied. Each one is verified and rolled back if it does not take. You can undo everything afterwards.`, 'Run'))) return;
    run.textContent = 'Working…'; results.replaceChildren();
    try {
      const r = await call('autotune_run', { ids: [...chosen], benchmark: bench.checked });
      status.textContent = '';
      results.append(h('div', { class: 'card hero' }, h('div', { class: 'eyebrow' }, 'Result'), h('h1', {}, `${r.apply.applied} applied` + (r.apply.failed ? `, ${r.apply.failed} failed` : '')),
        r.apply.restart !== 'none' ? h('p', { class: 'muted' }, 'A restart is needed for some changes to take effect.') : null));
      if (r.deltas.length) {
        results.append(h('h3', { class: 'h2' }, 'Measured difference'));
        r.deltas.forEach(d => { const good = d.verdict === 'better', bad = d.verdict === 'worse';
          results.append(h('div', { class: 'delta' }, h('div', { class: 'grow' }, h('b', {}, d.name), h('div', { class: 'small muted' }, `${fmt(d.before)} → ${fmt(d.after)} ${d.unit}`)),
            h('div', { class: 'pct ' + (good ? 'up' : bad ? 'down' : 'flat') }, (d.percent > 0 ? '+' : '') + d.percent.toFixed(1) + '%'),
            h('span', { class: 'badge ' + (good ? 'b-recommended' : bad ? 'b-highRisk' : 'b-info') }, good ? 'Better' : bad ? 'Worse' : 'Noise'))); });
        results.append(h('p', { class: 'small muted' }, 'Short synthetic benchmarks vary run to run. "Noise" means the change is within normal variation.'));
      }
      results.append(h('button', { class: 'btn danger', onclick: async () => { const n = await call('autotune_undo'); toast(`Reverted ${n} tweak(s).`, 'ok'); go('autotune'); } }, 'Undo Auto-Tune'));
      await call('settings_set', { patch: { firstRunDone: true } }); await refreshInfo();
    } catch (e) { toast(String(e), 'err'); }
  });
  root.replaceChildren(
    h('div', { class: 'card' }, h('div', { class: 'eyebrow' }, 'Detected hardware'), h('p', {}, view.description),
      !view.elevated ? h('p', { class: 'muted small' }, 'Not running as administrator — some tweaks will be skipped. Restart as admin from Settings.') : null),
    h('h3', { class: 'h2' }, `Planned (${plan.planned.length})`), plan.planned.length ? list : h('div', { class: 'empty' }, 'Everything suitable is already applied.'),
    skipped,
    h('label', { class: 'row' }, bench, h('span', {}, 'Measure before and after (takes about a minute)')),
    h('div', { class: 'row' }, S.lic.premium ? run : [lockButton(go, 'Auto-Tune is Premium'), h('span', { class: 'small muted' }, 'You can preview the plan for free.')], status), results);
  return wrap;
}
const fmt = n => n >= 1000 ? Math.round(n).toLocaleString() : n.toFixed(2);
