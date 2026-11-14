// Thin wrapper over the Tauri bridge. Without Tauri (plain browser / tests) a small mock backend answers.
const T = window.__TAURI__;
export const isTauri = !!T;

const listeners = new Map();
export async function on(event, fn) {
  if (T) return T.event.listen(event, e => fn(e.payload));
  const a = listeners.get(event) || []; a.push(fn); listeners.set(event, a);
  return () => {};
}
function emitMock(event, payload) { (listeners.get(event) || []).forEach(f => f(payload)); }

export async function call(cmd, args = {}) {
  try { return T ? await T.core.invoke(cmd, args) : await mock(cmd, args); }
  catch (e) {
    // The backend refuses premium items with "PREMIUM: …"; the shell turns that into one upsell prompt.
    if (String(e).startsWith('PREMIUM:')) window.dispatchEvent(new CustomEvent('neox-premium', { detail: String(e).slice(8).trim() }));
    throw e;
  }
}

export const win = {
  min: () => T && T.window.getCurrentWindow().minimize(),
  max: () => T && T.window.getCurrentWindow().toggleMaximize(),
  close: () => T && T.window.getCurrentWindow().close(),
};

// ------------------------------------------------------------------ mock
const sleep = ms => new Promise(r => setTimeout(r, ms));
let mockApplied = new Set(['mouse_accel']);
const mockScores = [{ time: 1731000000, label: 'Before tweaks', cpuOpsPerSec: 1.0e8, memoryMbps: 18000, timerAvgMs: 1.05, timerP99Ms: 1.9 }];
let mockPremium = new URLSearchParams(location.search).has('premium');
const MOCK_FREE = ['mouse_accel', 'tw_1', 'tw_2', 'tw_3', 'tw_4'];
const needPrem = what => { if (!mockPremium) throw 'PREMIUM: ' + what + ' is part of NEOX Premium.'; };
const CATS = ['performance', 'input', 'network', 'privacy', 'services', 'appearance', 'cleanup', 'tools'];
const RISK = ['recommended', 'caution', 'experimental', 'highRisk'];
const mockCatalog = Array.from({ length: 36 }, (_, i) => ({
  id: i === 0 ? 'mouse_accel' : 'tw_' + i, category: CATS[i % CATS.length], title: ['Disable mouse acceleration', 'Disable Game DVR', 'Network throttling off', 'Disable telemetry', 'Cleanup temp files'][i % 5] + (i > 4 ? ' #' + i : ''),
  description: 'Short description of what this does and why it may help.', consequences: 'Revert any time from History.',
  kind: i % 7 === 6 ? 'action' : 'reg', risk: RISK[i % 4 === 3 ? 1 : i % 3 === 0 ? 0 : 0], requiresAdmin: i % 3 === 1, restart: i % 5 === 2 ? 'restart' : 'none',
  claimedEffect: ['unknown', 'small', 'medium', 'hardwareDependent', 'negligible'][i % 5], button: i % 7 === 6 ? 'Run' : null, confirm: null,
}));
async function mock(cmd, a) {
  await sleep(120);
  switch (cmd) {
    case 'app_info': return { version: '3.1.0-mock', elevated: true, firstRunDone: false, settings: { autoUpdate: true, createRestorePointBeforeProfiles: true, gameExePath: null, firstRunDone: false, gameBoost: false, lastAutoTune: null }, pendingRestart: ['mouse_accel'], appliedCount: mockApplied.size, downgradeNotice: new URLSearchParams(location.search).has('downgraded') ? 7 : 0 };
    case 'catalog_list': return mockCatalog;
    case 'statuses': return Object.fromEntries(mockCatalog.map(t => [t.id, mockApplied.has(t.id) ? 'applied' : 'notApplied']));
    case 'applied_ids': return [...mockApplied];
    case 'tweak_apply': if (!MOCK_FREE.includes(a.id)) needPrem('This optimization'); mockApplied.add(a.id); return { success: true, message: 'Applied and verified.', restart: 'none' };
    case 'tweak_revert': mockApplied.delete(a.id); return { success: true, message: 'Reverted.', restart: 'none' };
    case 'action_run': return { success: true, message: 'Done: freed 412 MB.', restart: 'none' };
    case 'health_scan': return { score: 77, label: 'Good', checks: [
      { name: 'Free disk space', detail: 'Free space: 41%', score: 1, weight: 25, fix: 'none', ok: true },
      { name: 'Uptime', detail: 'Up for 9 days — restart recommended', score: .5, weight: 15, fix: 'restart', ok: false },
      { name: 'Startup apps', detail: '14 apps start with Windows', score: .5, weight: 15, fix: 'openStartup', ok: false },
      { name: 'Safe optimizations', detail: '6 of 12 applied', score: .5, weight: 30, fix: 'applySafeProfile', ok: false }] };
    case 'system_snapshot': return { cpu: { name: 'AMD Ryzen 7 5800X', cores: 8, threads: 16, mhz: 3800 }, gpus: [{ name: 'NVIDIA GeForce RTX 3070', driver: '31.0.15', vram: 8589934592, hz: 144 }], disks: [{ model: 'Samsung 980 PRO', type: 'SSD', size: 1e12, status: 'OK' }], volumes: [{ name: 'C:', total: 1e12, free: 4.1e11, fs: 'NTFS' }], os: { caption: 'Windows 11 Pro', version: '10.0', build: '26100', boot: 0, totalKb: 33554432, freeKb: 12e6 }, battery: false };
    case 'restart_windows': return true;
    case 'memory_clean': needPrem('Memory Cleaner'); return { beforeFree: 9e9, afterFree: 17e9, freed: 8e9, total: 32e9 };
    case 'score_list': return mockScores;
    case 'score_run': needPrem('Performance Score'); mockScores.push({ time: Math.floor(Date.now() / 1000), label: a.label || 'Measurement', cpuOpsPerSec: 1.0e8 + mockScores.length * 2e6, memoryMbps: 18000, timerAvgMs: 1.02, timerP99Ms: 1.4 - mockScores.length * 0.1 }); return mockScores;
    case 'score_clear': mockScores.length = 0; return;
    case 'monitor_start': { let n = 0; const t = setInterval(() => emitMock('monitor', { cpuPercent: 20 + 25 * Math.abs(Math.sin(n++ / 4)), ramPercent: 48, ramUsedBytes: 16e9, ramTotalBytes: 32e9 }), 1000); return; }
    case 'monitor_stop': return;
    case 'autotune_plan': return { description: '8 cores / 16 threads · RAM 32 GB · desktop · SSD · discrete GPU', elevated: true, plan: { planned: mockCatalog.slice(1, 7).map(t => ({ id: t.id, title: t.title, reason: 'Safe for your hardware' })), skipped: [{ id: 'x', title: 'Ultimate Performance plan', reason: 'conflicts with the high-performance plan' }], hardware: {} } };
    case 'autotune_run': needPrem('Auto-Tune'); for (const m of ['Benchmark before…', 'Applying…', 'Benchmark after…']) { emitMock('progress', m); await sleep(400); } return { apply: { applied: a.ids.length, skipped: 0, failed: 0, restart: 'none', lines: [] }, appliedIds: a.ids, before: {}, after: {}, deltas: [
      { name: 'CPU throughput', unit: 'Mops/s', before: 100, after: 103, percent: 3, verdict: 'noise', higherIsBetter: true },
      { name: 'Memory bandwidth', unit: 'MB/s', before: 18000, after: 19800, percent: 10, verdict: 'better', higherIsBetter: true },
      { name: 'Timer p99', unit: 'ms', before: 1.9, after: 1.4, percent: -26, verdict: 'better', higherIsBetter: false }] };
    case 'autotune_undo': return 6;
    case 'profiles_list': return [{ id: 'builtin.safe', name: 'Safe', description: 'Only low-risk, reversible tweaks.', builtIn: true, tweakIds: ['tw_1', 'tw_2'] }, { id: 'builtin.max', name: 'Maximum performance', description: 'Everything that raises performance, incl. high-risk items.', builtIn: true, tweakIds: ['tw_1', 'tw_3', 'tw_5'] }];
    case 'license_info': return { hwid: 'NEOX-1A2B-3C4D-5E6F-7A8B', premium: mockPremium, status: mockPremium ? { state: 'active', id: 'L-1', name: 'Tester', exp: 0 } : { state: 'free' }, freeIds: MOCK_FREE, buyUrl: 'https://discord.gg/2h9FUFb99' };
    case 'license_activate': if (a.token.startsWith('NEOX1.') || /^NEOX-[A-Z0-9]{4}-[A-Z0-9]{4}-[A-Z0-9]{4}$/i.test(a.token.trim())) { mockPremium = true; return { state: 'active', id: 'L-1', name: 'Tester', exp: 0 }; } return { state: 'invalid', reason: 'This is not a NEOX license key.' };
    case 'license_trial': mockPremium = true; return { state: 'active', id: 'TRIAL', name: 'Trial', exp: Math.floor(Date.now() / 1000) + 7 * 86400 };
    case 'ack_downgrade': return;
    case 'license_remove': mockPremium = false; return;
    case 'open_buy': window.open('https://discord.gg/2h9FUFb99', '_blank'); return;
    case 'nettune_plan': return { plan: { ethernet: true, wifi: true, planned: [{ id: 'net_throttle', title: 'Turn off network throttling', reason: 'removes the packet limit' }, { id: 'nic_im_off', title: 'NIC: disable interrupt moderation', reason: 'packets reach the CPU immediately' }, { id: 'wlan_perf', title: 'Wi-Fi: maximum performance mode', reason: 'Wi-Fi power mode set to maximum' }], skipped: [{ id: 'qos_limit', title: 'Remove QoS limit', reason: 'already applied' }] }, adapters: [{ name: 'Ethernet', kind: 'ethernet', description: 'Realtek 2.5GbE', speedMbps: 2500 }, { name: 'Wi-Fi', kind: 'wifi', description: 'Intel AX210', speedMbps: 1200 }], elevated: true };
    case 'nettune_run': needPrem('Internet Auto-Tune'); for (const m of ['Measuring latency…', 'Applying…', 'Measuring again…']) { emitMock('progress', m); await sleep(400); } return { apply: { applied: a.ids.length, skipped: 0, failed: 0, restart: 'none', lines: [] }, appliedIds: a.ids, reverted: false, deltas: [{ name: 'Average latency', unit: 'ms', before: 24.1, after: 19.8, percent: -17.8, verdict: 'better', higherIsBetter: false }, { name: 'Jitter', unit: 'ms', before: 3.2, after: 3.0, percent: -6, verdict: 'noise', higherIsBetter: false }] };
    case 'nettune_undo': return 3;
    case 'profile_apply': needPrem('Profiles'); emitMock('progress', 'Applying profile…'); await sleep(500); return { applied: 2, skipped: 0, failed: 0, restart: 'none', lines: ['Disable Game DVR: ok'] };
    case 'history_list': return [{ id: '1', time: new Date().toISOString(), tweakId: 'mouse_accel', title: 'Disable mouse acceleration', action: 'apply', success: true, message: 'Applied and verified.' }];
    case 'baselines_list': return [];
    case 'startup_list': return [{ name: 'Discord', command: 'C:\\Discord\\Update.exe', location: 'Current user', enabled: true, hive: 'HKCU', keyPath: 'Run' }, { name: 'OneDrive', command: 'OneDrive.exe', location: 'Current user', enabled: false, hive: 'HKCU', keyPath: 'Run' }];
    case 'processes_top': return [{ pid: 1, name: 'chrome', workingSetBytes: 1.2e9 }, { pid: 2, name: 'Discord', workingSetBytes: 4e8 }];
    case 'services_list': return [{ name: 'DiagTrack', displayName: 'Connected User Experiences and Telemetry', status: 'Running', startType: 'Automatic', isCritical: false }, { name: 'RpcSs', displayName: 'Remote Procedure Call', status: 'Running', startType: 'Automatic', isCritical: true }];
    case 'storage_analyze': return [{ path: 'C:\\Users', bytes: 2.4e11 }, { path: 'C:\\Windows', bytes: 3.1e10 }, { path: 'C:\\Program Files', bytes: 2.2e10 }];
    case 'net_ping_all': for (const n of ['Cloudflare', 'Google', 'Quad9']) { emitMock('ping-row', { name: n, stats: { host: '1.1.1.1', sent: 10, received: 10, minMs: 8, avgMs: 11.2, maxMs: 17, jitterMs: 2.1, lossPercent: 0 } }); await sleep(300); } return [];
    case 'net_mtu': return 1472;
    case 'net_dns_bench': for (const n of ['Cloudflare', 'Google', 'Quad9']) { emitMock('dns-row', { name: n, primary: '1.1.1.1', secondary: '1.0.0.1', medianMs: 9 + Math.random() * 20, failures: 0 }); await sleep(300); } return [];
    case 'net_dns_set': case 'net_dns_restore': return { success: true, message: 'DNS updated.', restart: 'none' };
    case 'net_adapters': return [{ name: 'Ethernet', guid: '{x}', description: 'Realtek 2.5GbE', speedMbps: 2500, dns: ['1.1.1.1'], addresses: ['192.168.1.5'] }];
    case 'games_list': return [{ name: 'Fortnite', exePath: 'C:\\Games\\Fortnite\\FortniteClient-Win64-Shipping.exe', profileId: null }];
    case 'games_epic': return [{ name: 'Rocket League', exePath: 'D:\\Epic\\RL\\RocketLeague.exe' }];
    case 'gameboost_get': case 'gameboost_set': return { enabled: !!a.enabled, status: a.enabled ? 'Waiting for a game…' : 'Off' };
    case 'fortnite_info': return [true, ['Config found'], false];
    case 'update_check': return { update: null, message: 'You are on the latest version.' };
    case 'settings_set': return { autoUpdate: true, createRestorePointBeforeProfiles: true, gameExePath: null, firstRunDone: true };
    case 'baseline_capture': return { name: a.name, time: new Date().toISOString(), statuses: {} };
    case 'pick_file': return null;
    default: return null;
  }
}
