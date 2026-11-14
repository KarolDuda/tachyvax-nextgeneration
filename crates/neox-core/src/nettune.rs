//! Internet Auto-Tune: picks network optimizations that fit the connection type (Ethernet / Wi-Fi).
//! The plan is data only; measuring and applying is done by the app layer.

use crate::autotune::PlannedTweak;
use crate::catalog::Catalog;
use serde::Serialize;

/// Network items that can make the Internet slower or look broken (lower throughput, extra DNS round trips,
/// wrong "No Internet" status, broken proxy discovery, lost NAT traversal). They stay available to apply by hand with a
/// warning, but no profile and no Auto-Tune ever applies them.
pub const NET_UNSAFE: &[&str] = &[
    "net_connectivity_probe_off", "tcp_offload_off", "dns_negcache", "netbios_off", "ipv4_pref", "tcp_heuristics_off", "tcp_timestamps_off",
    "nic_im_off", "nic_lso_off", "tcp_rsc_off", "teredo_off", "svc_wsaif",
];

/// Items that help any connection.
const COMMON: &[(&str, &str)] = &[
    ("net_throttle", "removes Windows' limit on network packet processing while media plays"),
    ("qos_limit", "gives back bandwidth reserved for QoS"),
    ("llmnr_off", "stops useless name broadcasts on the network"),
    ("wpad_off", "skips automatic proxy detection, which slows first connections"),
    ("nagle", "sends small packets immediately instead of waiting; lowers latency in games"),
];
const ETHERNET: &[(&str, &str)] = &[
    ("nic_eee_off", "Energy-Efficient Ethernet off: no wake-up delay on the link"),
    ("nic_flow_off", "flow control off: no pause frames that stall traffic"),
];
const WIFI: &[(&str, &str)] = &[
    ("wlan_perf", "Wi-Fi power mode set to maximum performance"),
    ("netadapter_nosave", "network adapters keep full power instead of sleeping between packets"),
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetPlan { pub ethernet: bool, pub wifi: bool, pub planned: Vec<PlannedTweak>, pub skipped: Vec<PlannedTweak> }

pub fn plan(catalog: &Catalog, ethernet: bool, wifi: bool, elevated: bool, applied: &dyn Fn(&str) -> bool) -> NetPlan {
    let mut wanted: Vec<(&str, &str)> = COMMON.to_vec();
    if ethernet { wanted.extend_from_slice(ETHERNET); }
    if wifi { wanted.extend_from_slice(WIFI); }
    let (mut planned, mut skipped) = (Vec::new(), Vec::new());
    for (id, reason) in wanted {
        let Some(t) = catalog.find(id) else { continue };
        let item = |r: &str| PlannedTweak { id: t.id.clone(), title: t.title.clone(), reason: r.into() };
        if applied(id) { skipped.push(item(crate::autotune::ALREADY_APPLIED)); }
        else if t.requires_admin && !elevated { skipped.push(item("requires administrator")); }
        else { planned.push(item(reason)); }
    }
    NetPlan { ethernet, wifi, planned, skipped }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cat() -> Catalog { Catalog::load_embedded().unwrap() }

    #[test]
    fn every_referenced_id_exists_in_the_catalog() {
        let c = cat();
        for (id, _) in COMMON.iter().chain(ETHERNET).chain(WIFI) { assert!(c.find(id).is_some(), "{id}"); }
    }
    #[test]
    fn ethernet_gets_nic_items_and_wifi_gets_wlan_items() {
        let c = cat();
        let e = plan(&c, true, false, true, &|_| false);
        assert!(e.planned.iter().any(|p| p.id == "nic_eee_off")); assert!(!e.planned.iter().any(|p| p.id == "wlan_perf"));
        let w = plan(&c, false, true, true, &|_| false);
        assert!(w.planned.iter().any(|p| p.id == "wlan_perf")); assert!(!w.planned.iter().any(|p| p.id == "nic_eee_off"));
        let both = plan(&c, true, true, true, &|_| false);
        assert!(both.planned.len() > e.planned.len());
    }
    #[test]
    fn internet_auto_tune_never_picks_items_that_can_slow_the_connection() {
        let c = cat();
        for id in NET_UNSAFE { assert!(c.find(id).is_some(), "{id} must exist"); }
        let p = plan(&c, true, true, true, &|_| false);
        for t in p.planned.iter().chain(p.skipped.iter()) { assert!(!NET_UNSAFE.contains(&t.id.as_str()), "{}", t.id); }
    }
    #[test]
    fn applied_and_admin_only_items_are_skipped_with_a_reason() {
        let c = cat();
        let p = plan(&c, true, false, false, &|id| id == "net_throttle");
        assert!(p.skipped.iter().any(|s| s.id == "net_throttle" && s.reason == crate::autotune::ALREADY_APPLIED));
        assert!(p.skipped.iter().any(|s| s.reason == "requires administrator"));
        assert!(!p.planned.iter().any(|s| c.find(&s.id).unwrap().requires_admin));
    }
}
