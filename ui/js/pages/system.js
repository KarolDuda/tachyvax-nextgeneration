import { call } from '../api.js';
import { h, page, bytes, meter, skeleton } from '../ui.js';

export default async function system() {
  const body = h('div', { class: 'stack' }, skeleton(4, 90));
  const wrap = page('System', 'Read-only overview of your hardware and Windows.', body);
  const s = await call('system_snapshot');
  const kv = (k, v) => h('div', { class: 'row' }, h('span', { class: 'muted', style: { width: '120px' } }, k), h('span', { class: 'grow sel' }, v));
  const totalGb = s.os.totalKb * 1024, usedGb = (s.os.totalKb - s.os.freeKb) * 1024;
  body.replaceChildren(
    h('div', { class: 'grid g2' },
      h('div', { class: 'card hover col' }, h('div', { class: 'eyebrow' }, 'Processor'), h('b', {}, s.cpu.name), kv('Cores', `${s.cpu.cores} / ${s.cpu.threads} threads`), kv('Max clock', s.cpu.mhz + ' MHz')),
      h('div', { class: 'card hover col' }, h('div', { class: 'eyebrow' }, 'Memory'), h('b', {}, bytes(totalGb)), meter(usedGb / totalGb * 100), h('span', { class: 'small muted' }, `${bytes(usedGb)} in use`)),
      s.gpus.map(g => h('div', { class: 'card hover col' }, h('div', { class: 'eyebrow' }, 'Graphics'), h('b', {}, g.name), kv('Driver', g.driver), kv('Video memory', g.vram > 0 ? bytes(g.vram) : 'n/a'), kv('Refresh', g.hz ? g.hz + ' Hz' : 'n/a'))),
      h('div', { class: 'card hover col' }, h('div', { class: 'eyebrow' }, 'Windows'), h('b', {}, s.os.caption), kv('Version', `${s.os.version} (build ${s.os.build})`), kv('Power', s.battery ? 'Laptop (battery)' : 'Desktop'))),
    h('h3', { class: 'h2' }, 'Storage'),
    h('div', { class: 'grid g2' }, s.volumes.map(v => { const used = v.total - v.free; return h('div', { class: 'card hover col' }, h('b', {}, `${v.name} · ${v.fs}`), meter(used / v.total * 100), h('span', { class: 'small muted' }, `${bytes(used)} used of ${bytes(v.total)} · ${bytes(v.free)} free`)); })),
    s.disks.length ? h('div', { class: 'grid g2' }, s.disks.map(d => h('div', { class: 'card hover col' }, h('b', {}, d.model), h('span', { class: 'small muted' }, `${d.type || 'Disk'} · ${bytes(d.size)} · ${d.status}`)))) : null);
  return wrap;
}
