//! The single place where system changes happen: snapshot -> apply -> verify by read-back -> history.
//! Operations are serialized (one change at a time) so snapshots never mix.

use crate::catalog::Catalog;
use crate::models::*;
use crate::state::{HistoryAction, HistoryEntry, StateStore};
use crate::timefmt;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Handles reversible changes of one or more kinds (registry, services, bcdedit...).
pub trait TweakHandler: Send + Sync {
    fn kinds(&self) -> &'static [TweakKind];
    fn status(&self, tweak: &TweakDefinition, ctx: &TweakContext) -> Result<TweakStatus, String>;
    /// Saves the previous state, applies the change and returns the snapshot needed to undo it.
    fn apply(&self, tweak: &TweakDefinition, ctx: &TweakContext) -> Result<Value, String>;
    fn revert(&self, tweak: &TweakDefinition, snapshot: &Value, ctx: &TweakContext) -> Result<(), String>;
}

/// One-shot actions: cleaning, opening tools, diagnostic commands.
pub trait ActionHandler: Send + Sync {
    fn can_handle(&self, tweak: &TweakDefinition) -> bool;
    fn run(&self, tweak: &TweakDefinition, ctx: &TweakContext) -> Result<String, String>;
}

pub trait Elevation: Send + Sync { fn is_elevated(&self) -> bool; }

pub trait RestorePoints: Send + Sync { fn create(&self, description: &str) -> TweakResult; }

static ID_COUNTER: AtomicU64 = AtomicU64::new(0);
pub fn new_id() -> String {
    let n = ID_COUNTER.fetch_add(1, Ordering::SeqCst);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{t:x}{n:04x}")
}

pub struct Engine {
    catalog: Arc<Catalog>,
    handlers: HashMap<TweakKind, Arc<dyn TweakHandler>>,
    actions: Vec<Arc<dyn ActionHandler>>,
    store: Arc<StateStore>,
    elevation: Arc<dyn Elevation>,
    gate: Mutex<()>,
}

fn truncate(s: &str) -> String {
    if s.chars().count() <= 500 { s.to_string() } else { format!("{}…", s.chars().take(500).collect::<String>()) }
}

impl Engine {
    pub fn new(catalog: Arc<Catalog>, store: Arc<StateStore>, elevation: Arc<dyn Elevation>,
               handlers: Vec<Arc<dyn TweakHandler>>, actions: Vec<Arc<dyn ActionHandler>>) -> Self {
        let mut map = HashMap::new();
        for h in handlers { for k in h.kinds() { map.insert(*k, h.clone()); } }
        Self { catalog, handlers: map, actions, store, elevation, gate: Mutex::new(()) }
    }

    pub fn catalog(&self) -> &Catalog { &self.catalog }
    pub fn store(&self) -> &Arc<StateStore> { &self.store }
    pub fn is_elevated(&self) -> bool { self.elevation.is_elevated() }
    pub fn is_applied(&self, id: &str) -> bool { self.store.read(|s| s.snapshots.contains_key(id)) }

    pub fn status(&self, id: &str, ctx: &TweakContext) -> TweakStatus {
        let Some(t) = self.catalog.find(id) else { return TweakStatus::Unknown };
        if t.is_action() { return TweakStatus::Unknown; }
        let Some(h) = self.handlers.get(&t.kind) else { return TweakStatus::Unknown };
        h.status(t, ctx).unwrap_or(TweakStatus::Unknown)
    }

    pub fn all_statuses(&self, ctx: &TweakContext) -> HashMap<String, TweakStatus> {
        self.catalog.all().iter().filter(|t| !t.is_action()).map(|t| (t.id.clone(), self.status(&t.id, ctx))).collect()
    }

    pub fn apply(&self, id: &str, ctx: &TweakContext) -> TweakResult {
        let Some(t) = self.catalog.find(id) else { return TweakResult::fail("Unknown optimization.") };
        if t.is_action() { return TweakResult::fail("This is a one-time action, not a setting."); }
        if t.requires_admin && !self.elevation.is_elevated() {
            return TweakResult::fail("This change requires running the app as administrator.");
        }
        let Some(handler) = self.handlers.get(&t.kind).cloned() else {
            return TweakResult::fail(format!("No handler for kind {:?}", t.kind));
        };

        let _gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        if self.is_applied(&t.id) { return TweakResult::ok("Already applied by NEOX.", RestartRequirement::None); }

        let before = handler.status(t, ctx).unwrap_or(TweakStatus::Unknown);
        let snapshot = match handler.apply(t, ctx) {
            Ok(s) => s,
            Err(e) => { self.record(t, HistoryAction::Apply, false, &e); return TweakResult::fail(e); }
        };

        // Persist the snapshot BEFORE anything else so undo is always possible.
        self.store.write(|s| { s.snapshots.insert(t.id.clone(), snapshot.clone()); });
        let _ = self.store.save();

        let after = handler.status(t, ctx).unwrap_or(TweakStatus::Unknown);
        if after == TweakStatus::NotApplied && before != TweakStatus::Applied {
            // Read-back did not confirm the change: roll back instead of leaving an unknown state.
            let _ = handler.revert(t, &snapshot, ctx);
            self.store.write(|s| { s.snapshots.remove(&t.id); });
            let msg = "The change was not confirmed by read-back (Windows or a policy may have rejected it). The previous state was restored.";
            self.record(t, HistoryAction::Apply, false, msg);
            return TweakResult::fail(msg);
        }

        if t.restart != RestartRequirement::None {
            let now = crate::timefmt::now_unix();
            self.store.write(|s| if !s.pending_restart.contains(&t.id) { if s.pending_restart.is_empty() { s.pending_restart_at = now; } s.pending_restart.push(t.id.clone()); });
        }
        self.record(t, HistoryAction::Apply, true, "Applied");
        TweakResult::ok("Applied.", t.restart)
    }

    pub fn revert(&self, id: &str, ctx: &TweakContext) -> TweakResult {
        let Some(t) = self.catalog.find(id) else { return TweakResult::fail("Unknown optimization.") };
        let Some(handler) = self.handlers.get(&t.kind).cloned() else {
            return TweakResult::fail(format!("No handler for kind {:?}", t.kind));
        };
        if t.requires_admin && !self.elevation.is_elevated() {
            return TweakResult::fail("Reverting requires running the app as administrator.");
        }
        let _gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        let Some(snapshot) = self.store.read(|s| s.snapshots.get(&t.id).cloned()) else {
            return TweakResult::fail("No backup of this change was saved, so it cannot be reverted safely.");
        };
        if let Err(e) = handler.revert(t, &snapshot, ctx) {
            self.record(t, HistoryAction::Revert, false, &e);
            return TweakResult::fail(e); // snapshot stays, so you can retry
        }
        self.store.write(|s| { s.snapshots.remove(&t.id); s.pending_restart.retain(|p| p != &t.id); });
        self.record(t, HistoryAction::Revert, true, "Reverted");
        TweakResult::ok("Reverted.", t.restart)
    }

    pub fn run_action(&self, id: &str, ctx: &TweakContext) -> TweakResult {
        let Some(t) = self.catalog.find(id) else { return TweakResult::fail("Unknown optimization.") };
        if !t.is_action() { return TweakResult::fail("This is not an action."); }
        if t.requires_admin && !self.elevation.is_elevated() {
            return TweakResult::fail("This action requires running the app as administrator.");
        }
        let Some(h) = self.actions.iter().find(|a| a.can_handle(t)).cloned() else {
            return TweakResult::fail("No handler for this action.");
        };
        let _gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        match h.run(t, ctx) {
            Ok(msg) => { self.record(t, HistoryAction::Run, true, &msg); TweakResult::ok(msg, RestartRequirement::None) }
            Err(e) => { self.record(t, HistoryAction::Run, false, &e); TweakResult::fail(e) }
        }
    }

    /// Drops the pending-restart list when Windows has been rebooted since the first item was added.
    /// `boot_unix` is when the current Windows session started. Returns true when something was cleared.
    pub fn clear_pending_if_rebooted(&self, boot_unix: i64) -> bool {
        let cleared = self.store.read(|s| !s.pending_restart.is_empty() && s.pending_restart_at < boot_unix);
        if cleared { self.store.write(|s| { s.pending_restart.clear(); s.pending_restart_at = 0; }); }
        cleared
    }
    pub fn clear_pending_restart(&self) {
        self.store.write(|s| { s.pending_restart.clear(); s.pending_restart_at = 0; });
        let _ = self.store.save();
    }

    pub(crate) fn record(&self, t: &TweakDefinition, action: HistoryAction, ok: bool, message: &str) {
        self.record_raw(&t.id, &t.title, action, ok, message);
    }

    pub(crate) fn record_raw(&self, id: &str, title: &str, action: HistoryAction, ok: bool, message: &str) {
        self.store.write(|s| {
            s.history.push(HistoryEntry { id: new_id(), time: timefmt::now_rfc3339(), tweak_id: id.into(), title: title.into(),
                                          action, success: ok, message: truncate(message) });
            if s.history.len() > 2000 { let extra = s.history.len() - 2000; s.history.drain(0..extra); }
        });
        let _ = self.store.save();
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    pub struct Admin(pub bool);
    impl Elevation for Admin { fn is_elevated(&self) -> bool { self.0 } }

    /// In-memory fake: tracks an "applied" flag per tweak id. `ignore` makes apply silently fail verification.
    pub struct Fake { pub on: Mutex<HashMap<String, bool>>, pub ignore: bool, pub fail_revert: bool }
    impl Fake { pub fn new() -> Self { Self { on: Mutex::new(HashMap::new()), ignore: false, fail_revert: false } } }
    impl TweakHandler for Fake {
        fn kinds(&self) -> &'static [TweakKind] { &[TweakKind::Reg, TweakKind::PowerIdx, TweakKind::RegPerAdapter, TweakKind::PowerPlan, TweakKind::Service] }
        fn status(&self, t: &TweakDefinition, _: &TweakContext) -> Result<TweakStatus, String> {
            Ok(if *self.on.lock().unwrap().get(&t.id).unwrap_or(&false) { TweakStatus::Applied } else { TweakStatus::NotApplied })
        }
        fn apply(&self, t: &TweakDefinition, _: &TweakContext) -> Result<Value, String> {
            if !self.ignore { self.on.lock().unwrap().insert(t.id.clone(), true); }
            Ok(serde_json::json!({"was": false}))
        }
        fn revert(&self, t: &TweakDefinition, _: &Value, _: &TweakContext) -> Result<(), String> {
            if self.fail_revert { return Err("boom".into()); }
            self.on.lock().unwrap().insert(t.id.clone(), false);
            Ok(())
        }
    }

    pub fn engine(admin: bool, fake: Fake) -> (Engine, Arc<StateStore>, Arc<Fake>) {
        let fake = Arc::new(fake);
        let store = Arc::new(StateStore::in_memory());
        let cat = Arc::new(Catalog::load_embedded().unwrap());
        let e = Engine::new(cat, store.clone(), Arc::new(Admin(admin)), vec![fake.clone() as Arc<dyn TweakHandler>], vec![]);
        (e, store, fake)
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    fn ctx() -> TweakContext { TweakContext::default() }

    #[test]
    fn apply_stores_snapshot_and_history_then_revert_clears_it() {
        let (e, store, fake) = engine(true, Fake::new());
        let r = e.apply("game_mode", &ctx());
        assert!(r.success, "{}", r.message);
        assert!(e.is_applied("game_mode"));
        assert!(*fake.on.lock().unwrap().get("game_mode").unwrap());
        assert_eq!(store.read(|s| s.history.len()), 1);

        let r = e.revert("game_mode", &ctx());
        assert!(r.success, "{}", r.message);
        assert!(!e.is_applied("game_mode"));
        assert!(!*fake.on.lock().unwrap().get("game_mode").unwrap());
    }

    #[test]
    fn unconfirmed_change_is_rolled_back() {
        let mut f = Fake::new();
        f.ignore = true;
        let (e, _, _) = engine(true, f);
        let r = e.apply("game_mode", &ctx());
        assert!(!r.success);
        assert!(!e.is_applied("game_mode"));
    }

    #[test]
    fn admin_only_tweaks_are_blocked_without_elevation() {
        let (e, _, _) = engine(false, Fake::new());
        let admin_tweak = e.catalog().all().iter().find(|t| t.requires_admin && t.kind == TweakKind::Reg).unwrap().id.clone();
        let r = e.apply(&admin_tweak, &ctx());
        assert!(!r.success);
        assert!(r.message.contains("administrator"));
    }

    #[test]
    fn applying_twice_is_idempotent() {
        let (e, store, _) = engine(true, Fake::new());
        assert!(e.apply("game_mode", &ctx()).success);
        let n = store.read(|s| s.history.len());
        let again = e.apply("game_mode", &ctx());
        assert!(again.success);
        assert_eq!(store.read(|s| s.history.len()), n);
    }

    #[test]
    fn failed_revert_keeps_snapshot_so_it_can_be_retried() {
        let mut f = Fake::new();
        f.fail_revert = true;
        let (e, _, _) = engine(true, f);
        assert!(e.apply("game_mode", &ctx()).success);
        assert!(!e.revert("game_mode", &ctx()).success);
        assert!(e.is_applied("game_mode"));
    }

    #[test]
    fn revert_without_snapshot_is_refused() {
        let (e, _, _) = engine(true, Fake::new());
        assert!(!e.revert("game_mode", &ctx()).success);
    }

    #[test]
    fn restart_requirement_is_tracked() {
        let (e, store, _) = engine(true, Fake::new());
        let id = e.catalog().all().iter().find(|t| t.kind == TweakKind::Reg && !t.requires_admin && t.restart != RestartRequirement::None).unwrap().id.clone();
        assert!(e.apply(&id, &ctx()).success);
        assert!(store.read(|s| s.pending_restart.contains(&id)));
        assert!(e.revert(&id, &ctx()).success);
        assert!(store.read(|s| s.pending_restart.is_empty()));
    }

    #[test]
    fn pending_restart_list_is_cleared_after_a_real_reboot_only() {
        let (e, store, _) = engine(true, Fake::new());
        let id = e.catalog().all().iter().find(|t| t.kind == TweakKind::Reg && !t.requires_admin && t.restart != RestartRequirement::None).unwrap().id.clone();
        assert!(e.apply(&id, &ctx()).success);
        let at = store.read(|s| s.pending_restart_at);
        assert!(at > 0 && !store.read(|s| s.pending_restart.is_empty()));
        assert!(!e.clear_pending_if_rebooted(at - 10), "booted before the change: still pending");
        assert!(e.clear_pending_if_rebooted(at + 10), "booted after the change: cleared");
        assert!(store.read(|s| s.pending_restart.is_empty()));
    }
}
