import { call, on, win } from './api.js';
import { h, toast, confirmBox, attachSpotlight, busy } from './ui.js';
import { S, refreshInfo, refreshLicense } from './state.js';

const ICONS = {
  dashboard: '<path d="M3 13h8V3H3zM13 21h8V11h-8zM3 21h8v-6H3zM13 3v6h8V3z"/>',
  autotune: '<path d="M13 2L4 14h7l-1 8 9-12h-7z"/>',
  gaming: '<rect x="2" y="7" width="20" height="11" rx="4"/><path d="M7 10v5M4.5 12.5h5M16 11.5h.01M18.5 14h.01"/>',
  optimizations: '<path d="M4 6h10M18 6h2M4 12h4M12 12h8M4 18h12M20 18h0"/><circle cx="16" cy="6" r="2"/><circle cx="10" cy="12" r="2"/><circle cx="18" cy="18" r="2"/>',
  network: '<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3c3 3 3 15 0 18M12 3c-3 3-3 15 0 18"/>',
  system: '<rect x="3" y="4" width="18" height="12" rx="2"/><path d="M8 20h8M12 16v4"/>',
  startup: '<path d="M12 3v9M6.3 6.3a8 8 0 1 0 11.4 0"/>',
  disk: '<ellipse cx="12" cy="6" rx="8" ry="3"/><path d="M4 6v12c0 1.7 3.6 3 8 3s8-1.3 8-3V6M4 12c0 1.7 3.6 3 8 3s8-1.3 8-3"/>',
  history: '<path d="M3 12a9 9 0 1 0 3-6.7L3 8M3 3v5h5M12 7v5l3 2"/>',
  boost: '<path d="M12 2v4M12 18v4M4.9 4.9l2.8 2.8M16.3 16.3l2.8 2.8M2 12h4M18 12h4M4.9 19.1l2.8-2.8M16.3 7.7l2.8-2.8"/><circle cx="12" cy="12" r="3"/>',
  keyboard: '<rect x="2" y="6" width="20" height="12" rx="2"/><path d="M6 10h.01M10 10h.01M14 10h.01M18 10h.01M7 14h10"/>',
  mouse: '<rect x="6" y="2.5" width="12" height="19" rx="6"/><path d="M12 2.5v7M6 9.5h12"/>',
  premium: '<path d="M12 3l2.6 5.6 6.1.7-4.5 4.2 1.2 6L12 16.6 6.6 19.5l1.2-6L3.3 9.3l6.1-.7z"/>',
  settings: '<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3h0a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5h0a1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8v0a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"/>',
};
const PAGES = [
  ['dashboard', 'Dashboard'], ['premium', 'Premium'], ['autotune', 'Auto-Tune'], ['boost', 'Boost'], ['gaming', 'Gaming'], ['optimizations', 'Optimizations'], ['keyboard', 'Keyboard'], ['mouse', 'Mouse'], ['network', 'Network'],
  ['system', 'System'], ['startup', 'Startup & services'], ['disk', 'Disk'], ['history', 'History'], ['settings', 'Settings'],
];
const loaders = Object.fromEntries(PAGES.map(([id]) => [id, () => import(`./pages/${id}.js`)]));
let current = null, token = 0;

export async function go(id) {
  if (!loaders[id]) id = 'dashboard';
  const my = ++token; current = id; S.cur = id;
  document.querySelectorAll('.nav-item').forEach((b, i) => { const on = b.dataset.id === id; b.classList.toggle('active', on); if (on) { const ind = document.getElementById('nav-ind'); ind.style.opacity = 1; ind.style.transform = `translateY(${b.offsetTop}px)`; } });
  const content = document.getElementById('content');
  try {
    const mod = await loaders[id]();
    const el = await mod.default({ go });
    if (my !== token) return;
    content.replaceChildren(el); content.scrollTop = 0;
  } catch (e) { console.error(e); if (my === token) content.replaceChildren(h('div', { class: 'empty' }, 'Something went wrong: ' + e)); }
}

function updateBars() {
  const bars = document.getElementById('bars'); bars.replaceChildren();
  const i = S.info; if (!i) return;
  if (!i.elevated) bars.append(h('div', { class: 'bar warn' }, h('span', { class: 'grow' }, h('b', {}, 'Limited mode. '), 'Run as administrator to apply most optimizations.'),
    h('button', { class: 'btn sm', onclick: async () => { if (!(await call('restart_as_admin'))) toast('Elevation was cancelled.', 'info'); } }, 'Restart as admin')));
  if (i.pendingRestart && i.pendingRestart.length) bars.append(h('div', { class: 'bar info' }, h('span', { class: 'grow' }, h('b', {}, 'Restart pending. '), `${i.pendingRestart.length} change(s) take effect after you restart Windows.`),
    h('button', { class: 'btn sm primary', onclick: async () => {
      if (!(await confirmBox('Restart Windows now?', 'Save your work first. Windows restarts in 10 seconds (run "shutdown /a" in a terminal to cancel).', 'Restart now'))) return;
      try { if (!(await call('restart_windows'))) toast('Windows did not accept the restart command. Restart it manually.', 'err'); else toast('Restarting in 10 seconds…', 'info', 9000); } catch (e) { toast(String(e), 'err'); } } }, 'Restart now'),
    h('button', { class: 'btn sm', onclick: async () => { await call('clear_pending_restart'); await refreshInfo(); } }, 'Later')));
}
function updateNav() {
  const b = document.querySelector('.nav-item[data-id=premium] span'); if (b) b.textContent = S.lic.premium ? 'Premium ✓' : 'Premium';
  const v = document.getElementById('version'); if (v && S.info) v.textContent = 'v' + S.info.version + (S.lic.premium ? ' · PREMIUM' : '');
}
S.nav = { updateBars, updateNav };
S.go = go;
// One upsell prompt whenever the backend refuses a premium item.
let upsellOpen = false;
window.addEventListener('neox-premium', async e => {
  if (upsellOpen) return; upsellOpen = true;
  const yes = await confirmBox('This is a Premium feature', (e.detail || 'This feature is part of NEOX Premium.') + `\n\nThe free plan includes ${S.lic.freeIds.length} items. Premium unlocks everything.`, 'See Premium');
  upsellOpen = false; if (yes) go('premium');
});

async function boot() {
  const fill = document.getElementById('splash-fill'); fill.style.width = '25%';
  document.getElementById('tb-min').onclick = win.min; document.getElementById('tb-max').onclick = win.max; document.getElementById('tb-close').onclick = win.close;
  const nav = document.getElementById('nav');
  PAGES.forEach(([id, label]) => nav.append(h('button', { class: 'nav-item' + (id === 'premium' ? ' premium' : ''), 'data-id': id, onclick: () => go(id), html: `<svg viewBox="0 0 24 24">${ICONS[id]}</svg><span>${label}</span>` })));
  attachSpotlight(document.body);
  fill.style.width = '60%';
  try { await refreshInfo(); await refreshLicense(); updateNav(); } catch (e) { console.error(e); }
  updateNav();
  fill.style.width = '100%';
  await go('dashboard');
  setTimeout(() => { const sp = document.getElementById('splash'); sp.classList.add('hide'); setTimeout(() => sp.remove(), 700); }, 450);
  await new Promise(r => setTimeout(r, 650));
  await backupWarning();

  if (S.info && S.info.downgradeNotice > 0) {
    await confirmBox('Premium has ended', `Your Premium access ended, so ${S.info.downgradeNotice} change(s) made with Premium were reverted to how they were before. Items from the free plan stay. Activate Premium again to re-apply them.`, 'OK');
    await call('ack_downgrade'); await refreshInfo();
  }
  if (S.info && !S.info.firstRunDone) setTimeout(async () => {
    const yes = await confirmBox('Welcome to NEOX', 'Run Auto-Tune now? It scans your hardware and applies only safe, reversible tweaks — you review the plan first.', 'Open Auto-Tune');
    await call('settings_set', { patch: { firstRunDone: true } }); await refreshInfo(); if (yes) go('autotune');
  }, 900);
  if (S.info?.settings?.autoUpdate) call('update_check').then(r => { if (r?.update) toast(`NEOX ${r.update.version} is available — see Settings.`, 'info', 8000); }).catch(() => {});
}

// Shown on every launch: NEOX targets maximum performance, so the user must acknowledge the risk and be offered a backup.
function backupWarning() {
  return new Promise(resolve => {
    const ok = h('input', { type: 'checkbox', id: 'ack' });
    const go = h('button', { class: 'btn primary', disabled: true }, 'Continue');
    ok.onchange = () => { go.disabled = !ok.checked; };
    const rp = h('button', { class: 'btn' }, 'Create restore point now');
    rp.onclick = () => busy(rp, async () => { rp.textContent = 'Creating…'; try { const r = await call('action_run', { id: 'restore_point' }); toast(r.message, r.success ? 'ok' : 'err'); } catch (e) { toast(String(e), 'err'); } });
    const bg = h('div', { class: 'modal-bg' }, h('div', { class: 'modal' },
      h('h3', {}, 'Back up your system first'),
      h('div', { class: 'body-text' }, 'NEOX is built for maximum performance. Many of its optimizations change deep Windows settings, and some of them lower security protections on purpose.\n\nBefore you apply anything, make a full backup of your system (a disk image or at least a restore point). NEOX and its author are not responsible for any damage, data loss or instability.\n\nEvery change NEOX makes is saved so it can be reverted from History — but a backup is the only real safety net.'),
      h('label', { class: 'row', style: { margin: '14px 0', cursor: 'pointer' } }, ok, h('span', {}, 'I understand and I use NEOX at my own risk.')),
      h('div', { class: 'actions' }, rp, go)));
    go.onclick = () => { bg.remove(); resolve(); };
    document.getElementById('modal-root').append(bg);
  });
}

boot();
