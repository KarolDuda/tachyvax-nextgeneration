import { call, on } from '../api.js';
import { h, page, toast, busy, confirmBox, lockButton, premiumBadge } from '../ui.js';
import { S, refreshLicense, refreshInfo, getCatalog } from '../state.js';

export default async function network({ go }) {
  await refreshLicense();
  const P = S.lic.premium;
  const pings = h('div', { class: 'stack' }), dns = h('div', { class: 'stack' }), info = h('div', { class: 'card' }, h('div', { class: 'muted' }, 'Loading…'));
  const status = h('div', { class: 'small muted mono' });
  const PRESETS = [['Cloudflare', '1.1.1.1', '1.0.0.1'], ['Google', '8.8.8.8', '8.8.4.4'], ['Quad9', '9.9.9.9', '149.112.112.112'], ['AdGuard', '94.140.14.14', '94.140.15.15']];
  const setDns = (name, a, b) => async () => { if (!(await confirmBox('Set DNS to ' + name + '?', 'Applied to your active network adapters. Your current DNS is saved and can be restored with "Restore my DNS".', 'Apply'))) return; const r = await call('net_dns_set', { servers: [a, b] }); toast(r.message, r.success ? 'ok' : 'err'); };
  const dnsCard = h('div', { class: 'card col', style: { gap: '10px' } }, h('div', { class: 'h2' }, 'Set DNS'), h('div', { class: 'small muted' }, 'Free. Pick a provider in one click, or run the benchmark below to find the fastest for you.'),
    h('div', { class: 'row wrap' }, PRESETS.map(([n, a, b]) => h('button', { class: 'btn', onclick: setDns(n, a, b) }, n)), h('button', { class: 'btn', onclick: async () => { const r = await call('net_dns_restore'); toast(r.message, r.success ? 'ok' : 'err'); } }, 'Automatic (restore mine)')));
  const tune = h('div', { class: 'stack' });
  const wrap = page('Network', 'Measure first, change second: latency to common servers, DNS speed, and a path MTU probe.', info, repairCard(), dnsCard, internetTune(tune, go, P),
    h('div', { class: 'row' }, h('h3', { class: 'h2 grow' }, 'Latency'), runBtn('Run ping test', 'net_ping_all', pings)), pings,
    h('div', { class: 'row' }, h('h3', { class: 'h2 grow' }, 'DNS benchmark'), runBtn('Run DNS benchmark', 'net_dns_bench', dns),
      h('button', { class: 'btn', onclick: async () => { const r = await call('net_dns_restore'); toast(r.message, r.success ? 'ok' : 'err'); } }, 'Restore my DNS')), dns, status);

  function runBtn(label, cmd, box) { const b = h('button', { class: 'btn primary' }, label); b.onclick = () => busy(b, async () => { box.replaceChildren(); b.textContent = 'Running…'; try { await call(cmd); } catch (e) { toast(String(e), 'err'); } }); return b; }
  await on('progress', m => { if (document.body.contains(status)) status.textContent = m; });
  await on('ping-row', r => { if (!document.body.contains(pings)) return; const s = r.stats;
    pings.append(h('div', { class: 'delta' }, h('b', { class: 'grow' }, r.name), h('span', { class: 'mono muted' }, s.host), h('span', {}, s.received ? s.avgMs.toFixed(1) + ' ms' : 'no reply'), h('span', { class: 'small muted' }, `jitter ${s.jitterMs.toFixed(1)} ms · loss ${s.lossPercent.toFixed(0)}%`))); });
  const dnsRows = [];
  await on('dns-row', r => { if (!document.body.contains(dns)) return; dnsRows.push(r); dnsRows.sort((a, b) => (a.medianMs ?? 1e9) - (b.medianMs ?? 1e9));
    dns.replaceChildren(...dnsRows.map((x, i) => h('div', { class: 'delta' }, h('b', { class: 'grow' }, (i === 0 ? '★ ' : '') + x.name), h('span', { class: 'mono muted' }, x.primary + ' / ' + x.secondary), h('span', {}, x.medianMs == null ? 'failed' : x.medianMs.toFixed(1) + ' ms'),
      h('button', { class: 'btn sm', onclick: async () => { if (!(await confirmBox('Set DNS to ' + x.name + '?', 'Applied to active adapters. Your current DNS is saved and can be restored with "Restore my DNS".', 'Apply'))) return; const r = await call('net_dns_set', { servers: [x.primary, x.secondary] }); toast(r.message, r.success ? 'ok' : 'err'); } }, 'Use')))); });

  Promise.all([call('net_adapters'), call('net_mtu')]).then(([ad, mtu]) => info.replaceChildren(h('div', { class: 'eyebrow' }, 'Active adapters'),
    ...ad.map(a => h('div', { class: 'row', style: { marginTop: '8px' } }, h('b', { class: 'grow' }, a.name + ' · ' + a.description), h('span', { class: 'muted small' }, `${a.speedMbps} Mbps · DNS ${a.dns.join(', ') || 'auto'}`))),
    h('div', { class: 'small muted', style: { marginTop: '8px' } }, mtu ? `Largest working payload without fragmentation: ${mtu} bytes (MTU ${mtu + 28}).` : 'MTU probe unavailable.'))).catch(() => {});
  return wrap;
}

// Free: undoes every network tweak NEOX applied and puts TCP back to the Windows defaults.
function repairCard() {
  const b = h('button', { class: 'btn primary' }, 'Repair my internet');
  b.onclick = () => busy(b, async () => {
    if (!(await confirmBox('Repair network settings?', 'NEOX will undo every network optimization it applied (including Internet Auto-Tune and profiles), reset TCP auto-tuning, RSS and RSC to the Windows defaults and flush the DNS cache.\n\nThe connection may drop for a few seconds. Needs administrator rights.', 'Repair'))) return;
    b.textContent = 'Repairing…';
    let undone = 0;
    try { undone += await call('nettune_undo'); } catch { /* nothing recorded */ }
    try {
      const [applied, catalog] = await Promise.all([call('applied_ids'), getCatalog()]);
      const net = new Set(catalog.filter(t => t.category === 'network' && t.kind !== 'action').map(t => t.id));
      ['svc_wsaif', 'svc_lmhosts'].forEach(id => { if (catalog.some(t => t.id === id)) net.add(id); });
      for (const id of applied) if (net.has(id)) { try { const r = await call('tweak_revert', { id, gameExe: null }); if (r.success) undone++; } catch { /* keep going */ } }
    } catch (e) { toast(String(e), 'err'); }
    let ok = true, msg = `Undid ${undone} network tweak(s).`;
    try { const r = await call('action_run', { id: 'net_defaults_reset' }); ok = r.success; msg += ' ' + r.message; } catch (e) { ok = false; msg += ' ' + e; }
    toast(msg, ok ? 'ok' : 'err', 9000); refreshInfo();
  });
  return h('div', { class: 'card col', style: { gap: '10px' } }, h('div', { class: 'h2' }, 'Internet feels slow or broken?'),
    h('div', { class: 'small muted' }, 'Free. Undoes every network change NEOX made and restores the Windows TCP defaults. Try this first, then measure again with the ping test below.'), h('div', { class: 'row' }, b));
}

function internetTune(root, go, P) {
  const box = h('div', { class: 'card col', style: { gap: '12px', borderColor: 'rgba(245,199,107,.3)' } },
    h('div', { class: 'row' }, h('div', { class: 'grow' }, h('div', { class: 'h2' }, 'Internet Auto-Tune ', P ? null : premiumBadge()),
      h('div', { class: 'small muted' }, 'Detects whether you use Ethernet or Wi-Fi, applies only the matching network optimizations, and measures latency, jitter and packet loss before and after.')),
      ), root);
  const out = h('div', { class: 'stack' }), status = h('div', { class: 'small muted mono' });
  on('progress', m => { if (document.body.contains(status)) status.textContent = m; });
  call('nettune_plan').then(v => {
    const chosen = new Set(v.plan.planned.map(p => p.id));
    const kinds = v.adapters.map(a => `${a.kind === 'wifi' ? 'Wi-Fi' : a.kind === 'ethernet' ? 'Ethernet' : 'Network'}: ${a.name}`).join(' · ') || 'No active adapter found';
    const run = h('button', { class: 'btn gold' }, 'Tune my internet');
    run.onclick = () => busy(run, async () => {
      if (!chosen.size) return toast('Select at least one item.', 'err');
      if (!(await confirmBox('Tune your internet?', `${chosen.size} network settings will be applied. The connection may drop for a few seconds. Everything can be undone.`, 'Run'))) return;
      run.textContent = 'Working…'; out.replaceChildren();
      try {
        const r = await call('nettune_run', { ids: [...chosen], measure: true });
        status.textContent = '';
        out.append(h('div', { class: 'h2' }, r.reverted ? 'Reverted: no improvement' : `${r.apply.applied} applied` + (r.apply.failed ? `, ${r.apply.failed} failed` : '')));
        if (r.reverted) out.append(h('p', { class: 'small' }, 'Latency got worse after the changes, so NEOX undid them. Your connection is back as it was.'));
        r.deltas.forEach(d => { const good = d.verdict === 'better', bad = d.verdict === 'worse';
          out.append(h('div', { class: 'delta' }, h('div', { class: 'grow' }, h('b', {}, d.name), h('div', { class: 'small muted' }, `${d.before.toFixed(1)} → ${d.after.toFixed(1)} ${d.unit}`)),
            h('div', { class: 'pct ' + (good ? 'up' : bad ? 'down' : 'flat') }, (d.percent > 0 ? '+' : '') + d.percent.toFixed(1) + '%'),
            h('span', { class: 'badge ' + (good ? 'b-recommended' : bad ? 'b-highRisk' : 'b-info') }, good ? 'Better' : bad ? 'Worse' : 'Noise'))); });
        out.append(h('p', { class: 'small muted' }, 'Ping results vary from minute to minute. “Noise” means the change is within normal variation. Some items need a restart.'),
          h('button', { class: 'btn danger sm', onclick: async () => { const k = await call('nettune_undo'); toast(`Reverted ${k} setting(s).`, 'ok'); go('network'); } }, 'Undo Internet Auto-Tune'));
      } catch (e) { toast(String(e), 'err'); }
    });
    root.replaceChildren(...[h('div', { class: 'small muted' }, kinds),
      ...v.plan.planned.map(p => h('label', { class: 'tw', style: { cursor: 'pointer' } }, h('input', { type: 'checkbox', checked: true, onchange: e => e.target.checked ? chosen.add(p.id) : chosen.delete(p.id) }),
        h('div', { class: 'grow' }, h('div', { class: 'title' }, p.title), h('div', { class: 'desc' }, p.reason)))),
      v.plan.planned.length ? null : h('div', { class: 'empty' }, 'Everything suitable is already applied.'),
      h('div', { class: 'row' }, P ? run : [lockButton(go, 'Internet Auto-Tune is Premium'), h('span', { class: 'small muted' }, 'Preview is free.')], status), out].filter(Boolean));
  }).catch(() => root.replaceChildren(h('div', { class: 'muted' }, 'Could not detect network adapters.')));
  return box;
}
