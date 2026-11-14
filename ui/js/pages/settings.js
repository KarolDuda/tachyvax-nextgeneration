import { call, on } from '../api.js';
import { h, page, switchEl, toast, busy, confirmBox, meter } from '../ui.js';
import { S, refreshInfo, refreshLicense } from '../state.js';
import { activateCard } from './premium.js';

export default async function settings({ go }) {
  const lic = await refreshLicense();
  const i = S.info, st = i.settings;
  const toggle = (title, desc, key) => h('div', { class: 'card row' }, h('div', { class: 'grow' }, h('div', { class: 'h2' }, title), h('div', { class: 'muted small' }, desc)),
    switchEl(st[key], async s => { const v = !s.classList.contains('on'); await call('settings_set', { patch: { [key]: v } }); s.set(v); await refreshInfo(); }));
  const msg = h('div', { class: 'muted small' }, 'Version ' + i.version);
  const upd = h('div', { class: 'card' }, h('div', { class: 'h2' }, 'Updates'), msg);
  const bar = h('div', { style: { display: 'none', marginTop: '10px' } }, meter(0));
  const check = h('button', { class: 'btn primary', style: { marginTop: '12px' } }, 'Check for updates');
  const install = h('button', { class: 'btn gold', style: { marginTop: '12px', marginLeft: '8px', display: 'none' } }, 'Install');
  on('update-progress', p => { if (document.body.contains(bar)) bar.replaceChildren(meter(p * 100)); });
  async function doInstall(u) {
    if (!(await confirmBox('Install ' + u.version + '?', (u.notes || '').slice(0, 600) + '\n\nNEOX downloads the installer, closes, updates itself and starts again. Windows may ask for permission.', 'Install'))) return;
    install.disabled = check.disabled = true; bar.style.display = 'block'; msg.textContent = 'Downloading ' + u.version + '…';
    try {
      const x = await call('update_install');
      if (x.success) msg.textContent = 'Installing. NEOX will restart…';
      else { toast(x.message, 'err', 7000); msg.textContent = x.message; install.disabled = check.disabled = false; bar.style.display = 'none'; }
    } catch (e) { toast(String(e), 'err'); install.disabled = check.disabled = false; bar.style.display = 'none'; }
  }
  check.onclick = () => busy(check, async () => {
    check.textContent = 'Checking…'; install.style.display = 'none';
    try {
      const r = await call('update_check'); msg.textContent = r.message;
      if (r.update) { install.textContent = 'Install ' + r.update.version; install.style.display = ''; install.onclick = () => doInstall(r.update); }
    } catch (e) { msg.textContent = String(e); }
  });
  upd.append(check, install, bar);
  const admin = h('div', { class: 'card row' }, h('div', { class: 'grow' }, h('div', { class: 'h2' }, 'Administrator rights'), h('div', { class: 'muted small' }, i.elevated ? 'NEOX is running as administrator.' : 'Many optimizations need administrator rights.')),
    i.elevated ? h('span', { class: 'badge b-recommended' }, 'Active') : h('button', { class: 'btn', onclick: async () => { if (!(await call('restart_as_admin'))) toast('Elevation was cancelled.', 'info'); } }, 'Restart as admin'));
  return page('Settings', 'NEOX by spinprint · spinprint.pl', admin, activateCard(lic, go), toggle('Restore point before profiles', 'Creates a Windows restore point before applying a profile (when Windows allows it).', 'createRestorePointBeforeProfiles'),
    toggle('Check for updates automatically', 'Looks for new versions on GitHub at startup. Downloads are verified before they run.', 'autoUpdate'), upd,
    h('div', { class: 'card' }, h('div', { class: 'h2' }, 'About'), h('p', { class: 'muted' }, 'NEOX applies documented, reversible Windows settings and tells you honestly what each one does. It does not promise FPS numbers. Everything is stored locally.')));
}
