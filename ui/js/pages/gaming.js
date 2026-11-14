import { call } from '../api.js';
import { h, page, switchEl, toast, confirmBox, busy, skeleton, lockButton, premiumBadge } from '../ui.js';
import { S, refreshInfo, refreshLicense } from '../state.js';

export default async function gaming({ go }) {
  await refreshLicense();
  const P = S.lic.premium;
  const body = h('div', { class: 'stack' }, skeleton(4, 90));
  const wrap = page('Gaming & profiles', 'Profiles are sets of tweaks. Game Boost switches to a performance power plan only while a game from your list runs, then restores yours.', body);
  const [profiles, games, boost, epic, fn] = await Promise.all([call('profiles_list'), call('games_list'), call('gameboost_get'), call('games_epic'), call('fortnite_info')]);

  const boostStatus = h('div', { class: 'small muted' }, boost.status);
  const boostSw = !P ? lockButton(go) : switchEl(boost.enabled, async s => { const on = !s.classList.contains('on'); const r = await call('gameboost_set', { enabled: on }); s.set(r.enabled); boostStatus.textContent = r.status; });
  const boostCard = h('div', { class: 'card hover row' }, h('div', { class: 'grow' }, h('div', { class: 'h2' }, 'Game Boost ', P ? null : premiumBadge()), h('div', { class: 'muted' }, 'Active only while a listed game is running; your original power plan is restored afterwards, even after a crash.'), boostStatus), boostSw);

  const profCards = profiles.map(p => h('div', { class: 'card hover', style: p.id === 'builtin.max' ? { borderColor: 'var(--border-hi)' } : null }, h('div', { class: 'row' }, h('div', { class: 'grow' }, h('div', { class: 'h2' }, p.name), h('div', { class: 'muted small' }, p.description + ` · ${p.tweakIds.length} tweaks`)),
    !P ? lockButton(go) : h('button', { class: 'btn sm primary', onclick: async e => busy(e.target, async () => {
      if (!(await confirmBox(`Apply "${p.name}"?`, (p.id === 'builtin.max' ? 'This includes HIGH-RISK items that lower security (CPU mitigations, VBS/HVCI, hypervisor). Make sure you have a full backup.\n\n' : '') + `${p.tweakIds.length} tweaks. A restore point is created first when enabled in Settings. Every change can be reverted from History.`, 'Apply'))) return;
      const r = await call('profile_apply', { id: p.id, gameExe: S.info.settings.gameExePath }); toast(`Applied ${r.applied}, skipped ${r.skipped}, failed ${r.failed}.`, r.failed ? 'err' : 'ok'); refreshInfo(); }) }, 'Apply'),
    p.builtIn || !P ? null : [h('button', { class: 'btn sm', onclick: async () => { const t = await call('profile_export', { id: p.id }); toast(t ? 'Exported to ' + t : 'Export cancelled.', t ? 'ok' : 'info'); } }, 'Export'),
      h('button', { class: 'btn sm danger', onclick: async e => { await call('profile_delete', { id: p.id }); toast('Profile deleted.', 'ok'); e.target.closest('.card').remove(); } }, 'Delete')])));

  const gameRows = h('div', { class: 'stack' }, games.length ? games.map(g => gameRow(g, true)) : h('div', { class: 'empty' }, 'No games added yet.'));
  function gameRow(g, own) { return h('div', { class: 'tw' }, h('div', { class: 'grow' }, h('div', { class: 'title' }, g.name), h('div', { class: 'desc mono' }, g.exePath)),
    own ? h('button', { class: 'btn sm danger', onclick: async e => { await call('game_remove', { exePath: g.exePath }); e.target.closest('.tw').remove(); } }, 'Remove')
        : h('button', { class: 'btn sm', onclick: async e => { await call('game_add', { name: g.name, exePath: g.exePath }); toast(g.name + ' added.', 'ok'); e.target.disabled = true; } }, 'Add')); }
  const addBtn = h('button', { class: 'btn', onclick: async () => { const p = await call('pick_file', { filter: 'exe' }); if (!p) return; const name = p.split(/[\\/]/).pop().replace(/\.exe$/i, ''); await call('game_add', { name, exePath: p }); gameRows.append(gameRow({ name, exePath: p }, true)); } }, 'Add game (.exe)…');
  const exeBtn = h('button', { class: 'btn sm', onclick: async () => { const p = await call('pick_file', { filter: 'exe' }); if (p) { await call('settings_set', { patch: { gameExePath: p } }); await refreshInfo(); toast('Game for per-game tweaks set.', 'ok'); } } }, 'Choose game for per-game tweaks');

  const [fnFound, fnLines, fnApplied] = fn || [false, [], false];
  const fnCard = h('div', { class: 'card hover' }, h('div', { class: 'h2' }, 'Fortnite settings ', P ? null : premiumBadge()), h('div', { class: 'muted small' }, fnFound ? fnLines.join(' · ') || 'Config found.' : 'Fortnite config not found.'),
    h('div', { class: 'row', style: { marginTop: '10px' } }, h('button', { class: 'btn sm', disabled: !fnFound || !P, onclick: async () => { const r = await call('fortnite_apply'); toast(r.message, r.success ? 'ok' : 'err'); } }, 'Apply low-latency config'),
      h('button', { class: 'btn sm', disabled: !fnFound, onclick: async () => { const r = await call('fortnite_restore'); toast(r.message, r.success ? 'ok' : 'err'); } }, 'Restore original')));

  body.replaceChildren(boostCard, h('h3', { class: 'h2' }, 'Profiles'), h('div', { class: 'stack' }, profCards),
    h('div', { class: 'row' }, h('h3', { class: 'h2 grow' }, 'Your games'), exeBtn, addBtn), gameRows,
    epic.length ? [h('h3', { class: 'h2' }, 'Detected (Epic Games)'), h('div', { class: 'stack' }, epic.map(g => gameRow(g, false)))] : null, fnCard);
  return wrap;
}
