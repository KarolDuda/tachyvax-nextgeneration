use neox_core::catalog::Catalog;
use neox_core::models::*;

fn cat() -> Catalog { Catalog::load_embedded().expect("catalog loads") }

#[test]
fn loads_all_283_items_with_unique_ids() {
    let c = cat();
    assert_eq!(c.len(), 283);
}

#[test]
fn every_item_has_title_description_and_consequences() {
    for t in cat().all() {
        assert!(!t.title.trim().is_empty(), "{}", t.id);
        assert!(!t.description.trim().is_empty(), "{}", t.id);
        if !t.is_action() { assert!(!t.consequences.trim().is_empty(), "{}", t.id); }
    }
}

#[test]
fn every_item_carries_the_data_its_kind_needs() {
    for t in cat().all() {
        let id = &t.id;
        match t.kind {
            TweakKind::Reg => assert!(t.values.as_ref().is_some_and(|v| !v.is_empty()), "{id}"),
            TweakKind::RegPerAdapter => assert!(t.per_adapter.as_ref().is_some_and(|v| !v.values.is_empty()), "{id}"),
            TweakKind::RegPerGame => assert!(t.per_game.as_ref().is_some_and(|v| !v.is_empty()), "{id}"),
            TweakKind::PowerIdx => assert!(t.power.is_some(), "{id}"),
            TweakKind::PowerPlan => assert!(t.plan.as_ref().is_some_and(|v| !v.is_empty()), "{id}"),
            TweakKind::Service => assert!(t.service.as_ref().is_some_and(|v| !v.is_empty()), "{id}"),
            TweakKind::Task => assert!(t.tasks.as_ref().is_some_and(|v| !v.is_empty()), "{id}"),
            TweakKind::Bcd => assert!(t.bcd.is_some(), "{id}"),
            TweakKind::Nic => assert!(t.nic.is_some(), "{id}"),
            TweakKind::Cmd => { let c = t.cmd.as_ref().expect(id); assert!(!c.on.is_empty() && !c.off.is_empty(), "{id}"); }
            TweakKind::Action => match t.action.expect(id) {
                ActionKind::Clean => assert!(t.patterns.as_ref().is_some_and(|v| !v.is_empty()), "{id}"),
                ActionKind::Open | ActionKind::Info => assert!(t.target.as_ref().is_some_and(|v| !v.is_empty()), "{id}"),
                ActionKind::Command => assert!(t.commands.as_ref().is_some_and(|v| !v.is_empty()), "{id}"),
                ActionKind::Native => assert!(t.native.as_ref().is_some_and(|v| !v.is_empty()), "{id}"),
            },
        }
    }
}

#[test]
fn critical_services_are_never_touched() {
    for s in ["wuauserv", "WinDefend", "Sense", "mpssvc", "SecurityHealthService", "EventLog", "RpcSs"] {
        assert!(!cat().all().iter().any(|t| t.service.as_deref().is_some_and(|x| x.eq_ignore_ascii_case(s))), "{s}");
    }
}

#[test]
fn nothing_disables_defender_or_windows_update_via_registry() {
    let bad = ["DisableAntiSpyware", "DisableRealtimeMonitoring", "NoAutoUpdate", "DisableWindowsUpdateAccess"];
    for t in cat().all() {
        for v in t.values.iter().flatten() {
            assert!(!bad.iter().any(|b| v.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(b))), "{}", t.id);
        }
    }
}

#[test]
fn services_are_never_marked_recommended() {
    assert!(!cat().all().iter().any(|t| t.kind == TweakKind::Service && t.risk == RiskLevel::Recommended));
}

#[test]
fn high_risk_items_exist_and_explain_consequences() {
    let c = cat();
    let high: Vec<_> = c.all().iter().filter(|t| t.risk == RiskLevel::HighRisk).collect();
    assert!(!high.is_empty());
    for t in high { assert!(t.consequences.chars().count() > 30, "{}", t.id); }
}

#[test]
fn destructive_actions_ask_for_confirmation() {
    let c = cat();
    for id in ["event_logs", "recycle", "temp_clean"] {
        assert!(c.find(id).expect(id).confirm.as_ref().is_some_and(|x| !x.is_empty()), "{id}");
    }
}

#[test]
fn search_matches_title_text() {
    let c = cat();
    assert!(!c.search("hibernat", None, None).is_empty());
    assert!(c.search("zzzzzzzz-does-not-exist", None, None).is_empty());
}

#[test]
fn max_profile_covers_the_high_risk_performance_items() {
    let ps = neox_core::profiles::built_in(&cat());
    let m = ps.iter().find(|p| p.id == "builtin.max").expect("max profile");
    for id in ["cpu_mitigations_off", "hvci_off", "vbs_off", "hypervisor_off", "tsc_enhanced", "dynamic_tick"] {
        assert!(m.tweak_ids.iter().any(|x| x == id), "{id}");
    }
    assert!(!m.tweak_ids.iter().any(|x| x == "ultimate_plan"));
    assert!(m.tweak_ids.len() > 30);
}

#[test]
fn every_free_id_exists_in_the_catalog() {
    let c = cat();
    for id in neox_core::license::FREE_IDS { assert!(c.find(id).is_some(), "{id}"); }
}

#[test]
fn free_plan_is_safe_only() {
    let c = cat();
    for id in neox_core::license::FREE_IDS {
        let t = c.find(id).unwrap();
        assert!(matches!(t.risk, RiskLevel::Recommended | RiskLevel::Caution), "{id}");
    }
}

#[test]
fn keyboard_and_mouse_pages_have_free_and_premium_items() {
    let c = cat();
    for section in ["keyboard", "mouse"] {
        let items: Vec<_> = c.all().iter().filter(|t| t.section.as_deref() == Some(section)).collect();
        let free = items.iter().filter(|t| neox_core::license::is_free(&t.id)).count();
        assert!(free >= 3, "{section}: free items");
        assert!(items.len() - free >= 3, "{section}: premium items");
    }
    for t in c.all() { assert!(matches!(t.section.as_deref(), None | Some("keyboard") | Some("mouse")), "{}", t.id); }
}

#[test]
fn risky_items_carry_a_real_warning() {
    let c = cat();
    for t in c.all().iter().filter(|t| matches!(t.risk, RiskLevel::HighRisk | RiskLevel::Experimental) && !t.is_action()) {
        assert!(t.consequences.chars().count() > 20, "{}", t.id);
    }
}

#[test]
fn internet_slowing_items_are_not_marked_recommended() {
    let c = cat();
    for id in neox_core::nettune::NET_UNSAFE {
        let t = c.find(id).unwrap();
        assert!(!matches!(t.risk, RiskLevel::Recommended), "{id}");
    }
}
