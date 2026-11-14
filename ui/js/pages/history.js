import { call } from '../api.js';
import { h, page, toast, confirmBox, fmtTime, skeleton, modal, lockButton } from '../ui.js';
import { S, refreshInfo, refreshLicense } from '../state.js';

export default async function history({ go }) {
  await refreshLicense();
  const P = S.lic.premium;
  const hist = h('div', { class: 'stack' }, skeleton(4, 56)), bases = h('div', { class: 'stack' });
  const wrap = page('History & restore', 'Everything NEOX changes is logged here. Revert all brings your system back to how it was before NEOX.',
    h('div', { class: 'row wrap' }, h('button', { class: 'btn danger', onclick: async () => { if (!(await confirmBox('Revert everything?', 'All optimizations applied by NEOX will be undone.', 'Revert all'))) return; const n = await call('revert_all'); toast(`Reverted ${n} item(s).`, 'ok'); refreshInfo(); go('history'); } }, 'Revert all changes'),
      P ? h('button', { class: 'btn', onclick: async () => { const name = 'Snapshot ' + new Date().toLocaleString(); await call('baseline_capture', { name }); toast('Baseline saved.', 'ok'); go('history'); } }, 'Save baseline snapshot') : lockButton(go, 'Baselines (Premium)')),
    h('h3', { class: 'h2' }, 'Baselines'), bases, h('h3', { class: 'h2' }, 'Activity'), hist);
  call('baselines_list').then(b => bases.replaceChildren(...(b.length ? b.map((x, i) => h('div', { class: 'delta' }, h('b', { class: 'grow' }, x.name), h('span', { class: 'muted small' }, fmtTime(x.time)),
    !P ? lockButton(go) : h('button', { class: 'btn sm', onclick: async () => { const d = await call('baseline_compare', { index: i }); const ch = d.filter(r => r.before !== r.now);
      modal({ title: 'Changes since "' + x.name + '"', body: ch.length ? ch.map(r => `${r.title}: ${r.before} → ${r.now}`).join('\n') : 'No differences.' }); } }, 'Compare'))) : [h('div', { class: 'empty' }, 'No baselines yet.')])));
  call('history_list').then(l => hist.replaceChildren(...(l.length ? l.map(e => h('div', { class: 'delta' }, h('i', { class: 'dot ' + (e.success ? 'on' : 'busy'), style: { marginTop: 0 } }),
    h('div', { class: 'grow' }, h('b', {}, e.title), h('div', { class: 'small muted' }, `${e.action} · ${e.message}`)), h('span', { class: 'small muted' }, fmtTime(e.time)))) : [h('div', { class: 'empty' }, 'No changes yet.')])));
  return wrap;
}
