import { call } from '../api.js';
import { h, page, switchEl, toast, bytes, skeleton, confirmBox } from '../ui.js';

export default async function startup() {
  const sList = h('div', { class: 'stack' }, skeleton(4, 64)), svc = h('div', { class: 'stack' }), procs = h('div', { class: 'stack' });
  const wrap = page('Startup & services', 'Disable what you do not need at boot. Critical Windows services are marked and never offered.', sList,
    h('h3', { class: 'h2' }, 'Heaviest processes'), procs, h('h3', { class: 'h2' }, 'Services'), svc);
  call('startup_list').then(list => sList.replaceChildren(...(list.length ? list.map(e => h('div', { class: 'tw' }, h('div', { class: 'grow' }, h('div', { class: 'title' }, e.name), h('div', { class: 'desc mono' }, e.command), h('div', { class: 'small muted' }, e.location)),
    switchEl(e.enabled, async s => { const on = !s.classList.contains('on'); s.disabled = true; try { const err = await call('startup_set', { hive: e.hive, name: e.name, enabled: on }); if (err) toast(err, 'err'); else s.set(on); } finally { s.disabled = false; } })))
    : [h('div', { class: 'empty' }, 'No startup entries found.')])));
  call('processes_top').then(p => procs.replaceChildren(...p.slice(0, 8).map(x => h('div', { class: 'delta' }, h('b', { class: 'grow' }, x.name), h('span', { class: 'muted small' }, 'PID ' + x.pid), h('span', {}, bytes(x.workingSetBytes))))));
  call('services_list').then(l => svc.replaceChildren(...l.map(x => h('div', { class: 'tw' }, h('div', { class: 'grow' }, h('div', { class: 'title' }, x.displayName || x.name), h('div', { class: 'desc' }, `${x.name} · ${x.status} · ${x.startType}`)),
    x.isCritical ? h('span', { class: 'badge b-highRisk' }, 'Critical') : h('span', { class: 'badge b-info' }, 'Use Optimizations to change')))));
  return wrap;
}
