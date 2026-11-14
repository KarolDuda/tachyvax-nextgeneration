import { call } from '../api.js';
import { h, page, toast, busy, fmtTime } from '../ui.js';
import { S, refreshLicense, getCatalog } from '../state.js';

const FEATURES = (all, free) => [
  [`All ${all} optimizations`, `The free plan has ${free}. Premium unlocks every performance, input, network and cleanup item, including the high-impact ones.`],
  ['Auto-Tune', 'Scans your hardware, applies the right set, and measures before and after.'],
  ['Internet Auto-Tune', 'Detects Ethernet and Wi-Fi and applies the network items that fit each, then measures latency, jitter and loss.'],
  ['Profiles', 'Maximum performance, Esports, Network boost, Streaming and Laptop. Plus your own profiles with import and export.'],
  ['Game Boost', 'Switches to a performance power plan only while your game runs, then restores your plan.'],
  ['Memory Cleaner', 'One click empties idle working sets and the standby list, handing a big block of RAM back before a game or heavy task.'],
  ['Performance Score', 'Measures CPU, memory and timer jitter and keeps the history, so you see what every change really did.'],
  ['Game tools', 'Fortnite low-latency config and baseline snapshots with before/after comparison.'],
];
const ROWS = (all, free) => [
  ['Optimizations, keyboard and mouse tweaks', String(free), 'All ' + all], ['Memory Cleaner', '—', '✔'], ['Performance Score history', '—', '✔'], ['Auto-Tune', '—', '✔'], ['Internet Auto-Tune (Wi-Fi + Ethernet)', '—', '✔'],
  ['Profiles (load, create, import, export)', '—', '✔'], ['Game Boost', '—', '✔'], ['Fortnite optimizer', '—', '✔'],
  ['Baselines & comparison', '—', '✔'], ['DNS switcher', '✔', '✔'], ['Undo / revert everything', '✔', '✔'], ['Restore point, history, disk analyzer', '✔', '✔'],
];

export default async function premium({ go }) {
  const lic = await refreshLicense();
  const st = lic.status;
  const all = (await getCatalog()).length, free = lic.freeIds.length;
  const buy = h('button', { class: 'btn gold lg' }, 'Buy Premium version');
  const trial = h('button', { class: 'btn lg' }, 'Start 7-day free trial');
  trial.onclick = () => busy(trial, async () => {
    trial.textContent = 'Starting…';
    const r = await call('license_trial');
    if (r.state === 'active') { toast('Trial started. Everything is unlocked for 7 days.', 'ok'); go('premium'); } else toast(r.reason || 'The trial could not be started.', 'err', 7000);
  });
  buy.onclick = () => call('open_buy').catch(() => toast('Open https://discord.gg/2h9FUFb99 in your browser.', 'info'));

  const hero = h('div', { class: 'prem-hero' },
    h('div', { class: 'eyebrow gold-text' }, 'NEOX PREMIUM'),
    h('h1', {}, lic.premium ? 'Premium is active' : 'Everything NEOX can do. Unlocked.'),
    h('p', { class: 'muted', style: { maxWidth: '640px' } }, lic.premium
      ? (st.id === 'TRIAL' ? `Free trial. ${Math.max(0, Math.ceil((st.exp * 1000 - Date.now()) / 864e5))} day(s) left. When it ends, everything you applied with Premium is reverted automatically.` : `Licensed to ${st.name}. Bound to this PC.` + (st.exp ? ` Valid until ${fmtTime(st.exp * 1000)}.` : ' Lifetime license.'))
      : `Free gives you ${free} safe items, network repair and the DNS switcher. Premium adds every optimization, the full Keyboard and Mouse pages, Auto-Tune, Internet Auto-Tune, Memory Cleaner, Performance Score, profiles and game tools.`),
    lic.premium && st.id !== 'TRIAL' ? null : h('div', { class: 'row wrap', style: { marginTop: '16px' } }, buy, lic.premium ? null : trial, h('span', { class: 'small muted' }, lic.premium ? 'Opens our Discord. Open a ticket in the Purchase channel.' : 'Trial: no payment, one per PC. After 7 days Premium changes are reverted automatically.')));

  const activate = activateCard(lic, go);

  const how = lic.premium ? null : h('div', { class: 'card' }, h('div', { class: 'h2' }, 'How to get Premium'),
    h('ol', { class: 'steps', style: { padding: 0, margin: '12px 0 0' } },
      h('li', {}, h('span', {}, 'Press “Buy Premium version” and join our Discord.')),
      h('li', {}, h('span', {}, 'Open a ticket in the Purchase channel and pay. You get an activation code.')),
      h('li', {}, h('span', {}, 'Open Settings (or this page), paste the code and press Activate.')),
      h('li', {}, h('span', {}, 'NEOX reads your PC\'s hardware ID and binds the code to it. Moving to a new PC? Open a ticket in Help.'))));

  const feats = h('div', { class: 'prem-grid' }, FEATURES(all, free).map(([t, d]) => h('div', { class: 'prem-feat' }, h('b', {}, t), h('span', { class: 'small muted' }, d))));
  const table = h('div', { class: 'card' }, h('table', { class: 'cmp' },
    h('thead', {}, h('tr', {}, h('th', {}, ''), h('th', {}, 'Free'), h('th', { class: 'gold-text' }, 'Premium'))),
    h('tbody', {}, ROWS(all, free).map(r => h('tr', {}, h('td', {}, r[0]), h('td', { class: r[1] === '—' ? 'n' : '' }, r[1]), h('td', { class: 'y' }, r[2]))))));
  return page('Premium', null, hero, how, activate, feats, table);
}

// Shared by the Premium page and Settings: shows the hardware id and takes an activation code or license key.
export function activateCard(lic, go) {
  const st = lic.status;
  const hw = h('div', { class: 'hwid' }, lic.hwid || '…');
  const copy = h('button', { class: 'btn sm', onclick: async () => { try { await navigator.clipboard.writeText(lic.hwid); toast('Hardware ID copied.', 'ok'); } catch { toast('Select the ID and copy it manually.', 'info'); } } }, 'Copy');
  const key = h('textarea', { class: 'key', placeholder: 'Activation code (NEOX-XXXX-XXXX-XXXX) or license key' });
  const act = h('button', { class: 'btn gold' }, 'Activate');
  act.onclick = () => busy(act, async () => {
    const r = await call('license_activate', { token: key.value.trim() });
    if (r.state === 'active') { toast('Premium activated. Thank you!', 'ok'); S.nav.updateNav && S.nav.updateNav(); go(S.cur || 'premium'); }
    else toast(r.reason || 'That key could not be activated.', 'err', 7000);
  });

  return h('div', { class: 'card col', style: { gap: '12px' } },
    h('div', { class: 'h2' }, lic.premium ? 'License' : 'Activation code'),
    h('div', { class: 'small muted' }, 'NEOX reads your hardware ID (from the motherboard, not your IP address) and the code is bound to it on first use. One code works on one PC.'),
    h('div', { class: 'row' }, hw, copy),
    lic.premium ? h('div', { class: 'row' }, h('span', { class: 'grow muted' }, `License ${st.id}`),
        h('button', { class: 'btn sm danger', onclick: async () => { await call('license_remove'); toast('License removed from this PC.', 'info'); go('premium'); } }, 'Remove from this PC'))
      : [key, h('div', { class: 'row' }, act)]);
}


