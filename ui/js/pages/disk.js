import { call, on } from '../api.js';
import { h, page, bytes, meter, busy, toast } from '../ui.js';

export default async function disk() {
  const out = h('div', { class: 'stack' }), status = h('div', { class: 'small muted mono' });
  const root = h('input', { class: 'btn', value: 'C:\\', style: { width: '160px', textAlign: 'left', userSelect: 'text' } });
  const go = h('button', { class: 'btn primary' }, 'Analyze');
  const wrap = page('Disk analyzer', 'Sizes of top-level folders. Read-only — nothing is deleted here. Use Optimizations → Cleanup for safe cleanup.', h('div', { class: 'row' }, root, go, status), out);
  await on('progress', m => { if (document.body.contains(status) && m.startsWith('Scanning')) status.textContent = m.slice(0, 80); });
  go.onclick = () => busy(go, async () => { go.textContent = 'Scanning…'; out.replaceChildren();
    try { const rows = await call('storage_analyze', { root: root.value }); status.textContent = ''; const max = rows[0]?.bytes || 1;
      out.replaceChildren(...(rows.length ? rows.map(r => h('div', { class: 'card hover col' }, h('div', { class: 'row' }, h('b', { class: 'grow mono' }, r.path), h('b', {}, bytes(r.bytes))), meter(r.bytes / max * 100))) : [h('div', { class: 'empty' }, 'Nothing found.')])); }
    catch (e) { toast(String(e), 'err'); } });
  return wrap;
}
