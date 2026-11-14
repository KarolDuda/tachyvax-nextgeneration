// Shared by the Keyboard and Mouse pages: every catalog item whose `section` matches is listed here (and hidden from Optimizations).
import { call } from '../api.js';
import { h, page, badge, switchEl, toast, confirmBox, skeleton, EFFECT, lockButton, premiumBadge, riskWarning } from '../ui.js';
import { S, getCatalog, refreshInfo, refreshLicense, isLocked } from '../state.js';

export async function peripheralPage({ go }, section, title, sub) {
  await refreshLicense();
  const list = h('div', { class: 'stack' }, skeleton(5));
  const catalog = (await getCatalog()).filter(t => t.section === section);
  const free = catalog.filter(t => !isLocked(t.id)).length;
  const banner = S.lic.premium ? null : h('div', { class: 'plan-banner' }, h('span', { class: 'grow' }, h('b', {}, 'Free plan: '), `${free} of ${catalog.length} items on this page are included. The rest are marked Premium.`), lockButton(go, 'Unlock all'));
  const wrap = page(title, sub, banner, list);
  const rows = new Map(); let statuses = {};

  function row(t) {
    const dot = h('i', { class: 'dot' });
    const locked = isLocked(t.id), isAction = t.kind === 'action';
    let ctl;
    if (locked) ctl = lockButton(go);
    else if (isAction) ctl = h('button', { class: 'btn sm', onclick: async e => {
      if (t.confirm && !(await confirmBox(t.title, t.confirm, t.button || 'Run'))) return;
      e.target.disabled = true;
      try { const r = await call('action_run', { id: t.id }); toast(r.message, r.success ? 'ok' : 'err', t.showsResult ? 9000 : 4200); } catch (x) { toast(String(x), 'err'); }
      e.target.disabled = false; } }, t.button || 'Run');
    else ctl = switchEl(false, async s => {
      const on = !s.classList.contains('on'); s.disabled = true; dot.className = 'dot busy';
      try {
        if (on && t.risk !== 'recommended' && !(await confirmBox(t.title, `${riskWarning(t)}${t.description}\n\n${t.consequences || ''}\n\nRisk: ${t.risk}. You can revert this later.`, 'Apply'))) return;
        const r = await call(on ? 'tweak_apply' : 'tweak_revert', { id: t.id, gameExe: null });
        toast(r.message, r.success ? 'ok' : 'err'); if (r.success) { statuses[t.id] = on ? 'applied' : 'notApplied'; s.set(on); }
        refreshInfo();
      } catch (x) { toast(String(x), 'err'); } finally { s.disabled = false; dot.className = 'dot ' + (statuses[t.id] === 'applied' ? 'on' : 'off'); }
    });
    rows.set(t.id, { dot, ctl, skip: isAction || locked });
    return h('div', { class: 'tw' + (locked ? ' locked' : '') }, dot, h('div', { class: 'grow' },
      h('div', { class: 'row wrap', style: { gap: '8px' } }, h('span', { class: 'title' }, t.title), isAction ? null : badge(t.risk), locked ? premiumBadge() : null, t.requiresAdmin ? h('span', { class: 'badge b-info' }, 'Admin') : null, t.restart !== 'none' ? h('span', { class: 'badge b-info' }, t.restart === 'logout' ? 'Sign-out' : 'Restart') : null),
      h('div', { class: 'desc' }, t.description),
      isAction ? null : h('details', {}, h('summary', {}, 'Details'), h('div', { class: 'more' }, (t.consequences || 'No side effects documented.') + ' · ' + (EFFECT[t.claimedEffect] || '')))), ctl);
  }
  // Free items first, then Premium; switches before tools.
  const sorted = [...catalog].sort((a, b) => (isLocked(a.id) - isLocked(b.id)) || ((a.kind === 'action') - (b.kind === 'action')));
  list.replaceChildren(...(sorted.length ? sorted.map(row) : [h('div', { class: 'empty' }, 'Nothing here yet.')]));
  call('statuses', { gameExe: null }).then(st => { statuses = st; for (const [id, r] of rows) { if (r.skip) continue; const on = st[id] === 'applied'; r.ctl.set(on); r.dot.className = 'dot ' + (on ? 'on' : st[id] === 'unknown' ? '' : 'off'); } }).catch(() => {});
  return wrap;
}
