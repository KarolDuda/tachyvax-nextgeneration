//! Network diagnostics: ping statistics, MTU discovery, DNS benchmark (raw UDP, std only) and DNS switching with backup.

use crate::adapters::Adapters;
use crate::registry::{RegData, RegistryApi};
use crate::runner::{powershell, Runner};
use neox_core::models::{RestartRequirement, TweakResult};
use neox_core::state::StateStore;
use serde::Serialize;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

pub const PING_TARGETS: &[(&str, &str)] = &[
    ("Cloudflare", "1.1.1.1"), ("Google", "8.8.8.8"),
    ("AWS Frankfurt (EU)", "ec2.eu-central-1.amazonaws.com"), ("AWS Ireland (EU)", "ec2.eu-west-1.amazonaws.com"),
    ("AWS London (EU)", "ec2.eu-west-2.amazonaws.com"), ("AWS Paris (EU)", "ec2.eu-west-3.amazonaws.com"),
    ("AWS Stockholm (EU)", "ec2.eu-north-1.amazonaws.com"), ("AWS Virginia (NA-East)", "ec2.us-east-1.amazonaws.com"),
];

pub const DNS_PROVIDERS: &[(&str, &str, &str)] = &[
    ("Cloudflare", "1.1.1.1", "1.0.0.1"), ("Google", "8.8.8.8", "8.8.4.4"), ("Quad9", "9.9.9.9", "149.112.112.112"),
    ("OpenDNS", "208.67.222.222", "208.67.220.220"), ("AdGuard", "94.140.14.14", "94.140.15.15"),
    ("CleanBrowsing", "185.228.168.9", "185.228.169.9"), ("Control D", "76.76.2.0", "76.76.10.0"), ("DNS.WATCH", "84.200.69.80", "84.200.70.40"),
];

pub const DNS_TEST_DOMAINS: &[&str] = &["epicgames.com", "fortnite.com", "unrealengine.com", "amazonaws.com", "akamai.net", "cloudflare.com"];

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PingStats { pub host: String, pub sent: u32, pub received: u32, pub min_ms: f64, pub avg_ms: f64, pub max_ms: f64, pub jitter_ms: f64, pub loss_percent: f64 }

/// `samples`: one entry per probe, `None` = lost.
pub fn ping_stats(host: &str, samples: &[Option<f64>]) -> PingStats {
    let sent = samples.len() as u32;
    let t: Vec<f64> = samples.iter().flatten().copied().collect();
    let loss = if sent == 0 { 0.0 } else { 100.0 * f64::from(sent - t.len() as u32) / f64::from(sent) };
    if t.is_empty() { return PingStats { host: host.into(), sent, received: 0, min_ms: 0.0, avg_ms: 0.0, max_ms: 0.0, jitter_ms: 0.0, loss_percent: loss }; }
    let avg = t.iter().sum::<f64>() / t.len() as f64;
    let jitter = if t.len() > 1 { t.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f64>() / (t.len() - 1) as f64 } else { 0.0 };
    PingStats { host: host.into(), sent, received: t.len() as u32, min_ms: t.iter().copied().fold(f64::MAX, f64::min), avg_ms: avg,
                max_ms: t.iter().copied().fold(0.0, f64::max), jitter_ms: jitter, loss_percent: loss }
}

fn valid_host(h: &str) -> bool { !h.is_empty() && h.len() < 254 && h.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') }

/// Locale-independent ping through .NET (PowerShell): prints the round-trip in ms or `x` per probe.
pub fn ping(run: &dyn Runner, host: &str, count: u32) -> Result<PingStats, String> {
    if !valid_host(host) { return Err("Invalid host name.".into()); }
    let script = format!("$p=New-Object System.Net.NetworkInformation.Ping; 1..{count} | ForEach-Object {{ try {{ $r=$p.Send('{host}',2000); \
if($r.Status -eq 'Success'){{$r.RoundtripTime}}else{{'x'}} }} catch {{'x'}}; Start-Sleep -Milliseconds 100 }}");
    let r = run.run(&powershell(&script), 60 + u64::from(count) * 3);
    let samples: Vec<Option<f64>> = r.output.lines().map(str::trim).filter(|l| !l.is_empty()).map(|l| l.parse::<f64>().ok()).collect();
    if samples.is_empty() && !r.success() { return Err("Ping failed to run.".into()); }
    Ok(ping_stats(host, &samples))
}

/// Largest packet without fragmentation (MTU), or `None` when the host does not answer ping.
pub fn find_mtu(run: &dyn Runner, host: &str) -> Option<u32> {
    if !valid_host(host) { return None; }
    let script = format!("$p=New-Object System.Net.NetworkInformation.Ping; $o=New-Object System.Net.NetworkInformation.PingOptions(64,$true); \
function Ok($n){{ try {{ $p.Send('{host}',1500,(New-Object byte[] $n),$o).Status -eq 'Success' }} catch {{ $false }} }}; \
if(-not (Ok 32)){{ 'none' }} elseif(Ok 1472){{ 1500 }} else {{ $lo=32;$hi=1472; while($hi-$lo -gt 1){{ $m=[int](($lo+$hi)/2); if(Ok $m){{$lo=$m}}else{{$hi=$m}} }}; $lo+28 }}");
    let r = run.run(&powershell(&script), 90);
    r.output.lines().last()?.trim().parse().ok()
}

// ---------------------------------------------------------------- DNS benchmark

pub fn build_dns_query(id: u16, domain: &str) -> Vec<u8> {
    let mut p = vec![(id >> 8) as u8, id as u8, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in domain.split('.') { p.push(label.len() as u8); p.extend_from_slice(label.as_bytes()); }
    p.extend_from_slice(&[0, 0, 1, 0, 1]);
    p
}

/// A reply is valid when it carries our id and RCODE 0.
pub fn is_valid_reply(id: u16, reply: &[u8]) -> bool { reply.len() >= 12 && reply[0] == (id >> 8) as u8 && reply[1] == id as u8 && (reply[3] & 0x0F) == 0 }

fn random_id() -> u16 {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(1);
    (n ^ (n >> 16)) as u16
}

/// One DNS A query over UDP; response time in ms.
pub fn query_dns(server: &str, domain: &str, timeout_ms: u64) -> Option<f64> {
    let ip: Ipv4Addr = server.parse().ok()?;
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.set_read_timeout(Some(Duration::from_millis(timeout_ms))).ok()?;
    let id = random_id();
    let t = Instant::now();
    sock.send_to(&build_dns_query(id, domain), SocketAddr::from((ip, 53))).ok()?;
    let mut buf = [0u8; 512];
    let n = sock.recv(&mut buf).ok()?;
    is_valid_reply(id, &buf[..n]).then(|| t.elapsed().as_secs_f64() * 1000.0)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DnsBenchResult { pub name: String, pub primary: String, pub secondary: String, pub median_ms: Option<f64>, pub failures: u32 }

pub fn bench_dns(provider: (&str, &str, &str), rounds: u32, cancelled: &dyn Fn() -> bool) -> DnsBenchResult {
    let (mut times, mut fails) = (Vec::new(), 0u32);
    'outer: for d in DNS_TEST_DOMAINS {
        for _ in 0..rounds {
            if cancelled() { break 'outer; }
            match query_dns(provider.1, d, 1500) { Some(ms) => times.push(ms), None => fails += 1 }
        }
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    DnsBenchResult { name: provider.0.into(), primary: provider.1.into(), secondary: provider.2.into(),
                     median_ms: if times.is_empty() { None } else { Some(times[times.len() / 2]) }, failures: fails }
}

// ---------------------------------------------------------------- DNS switching

fn split_servers(s: &str) -> Vec<String> { s.split([',', ' ', ';']).filter(|x| !x.is_empty()).map(String::from).collect() }

/// (servers, is_manual) for an adapter, from the registry (NameServer = manual, DhcpNameServer = automatic).
pub fn adapter_dns(reg: &dyn RegistryApi, guid: &str) -> (Vec<String>, bool) {
    let key = format!("SYSTEM\\CurrentControlSet\\Services\\Tcpip\\Parameters\\Interfaces\\{{{guid}}}");
    let get = |n: &str| match reg.read("HKLM", &key, Some(n)) { Ok(Some(RegData::Str(s))) | Ok(Some(RegData::ExpandStr(s))) => split_servers(&s), _ => vec![] };
    let manual = get("NameServer");
    if !manual.is_empty() { (manual, true) } else { (get("DhcpNameServer"), false) }
}

pub fn set_dns(run: &dyn Runner, adapters: &dyn Adapters, reg: &dyn RegistryApi, store: &StateStore, servers: &[String]) -> TweakResult {
    if servers.is_empty() || !servers.iter().all(|s| s.parse::<Ipv4Addr>().is_ok()) { return TweakResult::fail("Invalid DNS address."); }
    let mut done = 0;
    for a in adapters.active() {
        let (cur, manual) = adapter_dns(reg, &a.guid);
        store.write(|s| { s.dns_backups.entry(a.guid.clone()).or_insert_with(|| if manual { cur.clone() } else { vec![] }); });
        let _ = store.save();
        let list = servers.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(",");
        let r = run.run(&powershell(&format!("Set-DnsClientServerAddress -InterfaceAlias '{}' -ServerAddresses ({list}) -ErrorAction Stop", a.name.replace('\'', "''"))), 30);
        if r.success() { done += 1; }
    }
    run.run(&["ipconfig".to_string(), "/flushdns".to_string()], 30);
    if done > 0 { TweakResult::ok(format!("DNS set to {} ({done} adapters).", servers.join(" / ")), RestartRequirement::None) }
    else { TweakResult::fail("Could not set DNS (administrator rights required).") }
}

pub fn restore_dns(run: &dyn Runner, adapters: &dyn Adapters, store: &StateStore) -> TweakResult {
    if store.read(|s| s.dns_backups.is_empty()) { return TweakResult::fail("No saved DNS backup."); }
    let mut done = 0;
    for a in adapters.active() {
        let Some(old) = store.read(|s| s.dns_backups.get(&a.guid).cloned()) else { continue };
        let alias = a.name.replace('\'', "''");
        let script = if old.is_empty() { format!("Set-DnsClientServerAddress -InterfaceAlias '{alias}' -ResetServerAddresses -ErrorAction Stop") }
            else { format!("Set-DnsClientServerAddress -InterfaceAlias '{alias}' -ServerAddresses ({}) -ErrorAction Stop", old.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(",")) };
        if run.run(&powershell(&script), 30).success() { store.write(|s| { s.dns_backups.remove(&a.guid); }); done += 1; }
    }
    let _ = store.save();
    if done > 0 { TweakResult::ok(format!("Previous DNS restored ({done} adapters)."), RestartRequirement::None) } else { TweakResult::fail("Could not restore DNS.") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::AdapterInfo;
    use crate::registry::MemRegistry;
    use crate::runner::fake::Script;
    use crate::runner::RunResult;

    struct Nic;
    impl Adapters for Nic {
        fn active(&self) -> Vec<AdapterInfo> { vec![AdapterInfo { name: "Eth'1".into(), guid: "G1".into(), description: "".into(), speed_mbps: 0, dns: vec![], addresses: vec![], kind: "ethernet".into() }] }
    }

    #[test]
    fn ping_stats_compute_loss_and_jitter() {
        let s = ping_stats("h", &[Some(10.0), Some(14.0), None, Some(12.0)]);
        assert_eq!((s.sent, s.received), (4, 3));
        assert_eq!(s.loss_percent, 25.0);
        assert_eq!(s.min_ms, 10.0);
        assert_eq!(s.max_ms, 14.0);
        assert_eq!(s.jitter_ms, 3.0);
        let dead = ping_stats("h", &[None, None]);
        assert_eq!(dead.loss_percent, 100.0);
        assert_eq!(dead.avg_ms, 0.0);
    }

    #[test]
    fn ping_parses_probe_output_and_rejects_bad_hosts() {
        let run = Script::new();
        run.on("powershell", RunResult::ok("12\nx\n14\n"));
        let s = ping(&run, "1.1.1.1", 3).unwrap();
        assert_eq!((s.sent, s.received), (3, 2));
        assert!(ping(&run, "x'; rm -rf /", 3).is_err());
    }

    #[test]
    fn mtu_parsing() {
        let run = Script::new();
        run.on("powershell", RunResult::ok("1492"));
        assert_eq!(find_mtu(&run, "1.1.1.1"), Some(1492));
        let run = Script::new();
        run.on("powershell", RunResult::ok("none"));
        assert_eq!(find_mtu(&run, "1.1.1.1"), None);
    }

    #[test]
    fn dns_query_packet_is_well_formed() {
        let q = build_dns_query(0xABCD, "a.bc");
        assert_eq!(&q[..4], &[0xAB, 0xCD, 0x01, 0x00]);
        assert_eq!(&q[12..], &[1, b'a', 2, b'b', b'c', 0, 0, 1, 0, 1]);
        assert!(is_valid_reply(0xABCD, &[0xAB, 0xCD, 0x81, 0x80, 0, 0, 0, 0, 0, 0, 0, 0]));
        assert!(!is_valid_reply(0xABCD, &[0xAB, 0xCD, 0x81, 0x83, 0, 0, 0, 0, 0, 0, 0, 0])); // NXDOMAIN
        assert!(!is_valid_reply(0xABCD, &[0x00, 0x00, 0x81, 0x80, 0, 0, 0, 0, 0, 0, 0, 0])); // wrong id
        assert!(!is_valid_reply(1, &[1, 2]));
    }

    #[test]
    fn dns_query_talks_to_a_local_udp_server() {
        // Only the loopback is used here; real servers are never contacted by tests.
        let srv = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = srv.local_addr().unwrap().port();
        let h = std::thread::spawn(move || {
            let mut b = [0u8; 512];
            let (n, from) = srv.recv_from(&mut b).unwrap();
            let mut reply = b[..n].to_vec();
            reply[2] = 0x81; reply[3] = 0x80;
            srv.send_to(&reply, from).unwrap();
        });
        // query_dns always uses port 53, so exercise the same building blocks manually.
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        sock.set_read_timeout(Some(Duration::from_millis(1000))).unwrap();
        sock.send_to(&build_dns_query(7, "example.com"), ("127.0.0.1", port)).unwrap();
        let mut buf = [0u8; 512];
        let n = sock.recv(&mut buf).unwrap();
        assert!(is_valid_reply(7, &buf[..n]));
        h.join().unwrap();
        assert!(query_dns("not-an-ip", "x.com", 50).is_none());
    }

    #[test]
    fn set_dns_validates_backs_up_and_restores() {
        let reg = MemRegistry::new();
        reg.write("HKLM", "SYSTEM\\CurrentControlSet\\Services\\Tcpip\\Parameters\\Interfaces\\{G1}", Some("NameServer"), &RegData::Str("9.9.9.9,8.8.4.4".into())).unwrap();
        let store = StateStore::in_memory();
        let run = Script::new();
        assert!(!set_dns(&run, &Nic, &reg, &store, &["not an ip".into()]).success);
        assert!(!set_dns(&run, &Nic, &reg, &store, &[]).success);
        assert!(set_dns(&run, &Nic, &reg, &store, &["1.1.1.1".into(), "1.0.0.1".into()]).success);
        assert_eq!(store.read(|s| s.dns_backups["G1"].clone()), vec!["9.9.9.9".to_string(), "8.8.4.4".to_string()]);
        assert!(run.calls().iter().any(|c| c.contains("'Eth''1'") && c.contains("'1.1.1.1','1.0.0.1'")));
        // a second change must not overwrite the first backup
        assert!(set_dns(&run, &Nic, &reg, &store, &["8.8.8.8".into()]).success);
        assert_eq!(store.read(|s| s.dns_backups["G1"].len()), 2);
        assert!(restore_dns(&run, &Nic, &store).success);
        assert!(store.read(|s| s.dns_backups.is_empty()));
        assert!(!restore_dns(&run, &Nic, &store).success);
    }

    #[test]
    fn dhcp_adapters_are_restored_with_a_reset() {
        let reg = MemRegistry::new();
        let store = StateStore::in_memory();
        let run = Script::new();
        assert!(set_dns(&run, &Nic, &reg, &store, &["1.1.1.1".into()]).success);
        assert!(store.read(|s| s.dns_backups["G1"].is_empty()));
        restore_dns(&run, &Nic, &store);
        assert!(run.calls().iter().any(|c| c.contains("-ResetServerAddresses")));
    }
}
