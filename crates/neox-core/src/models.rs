//! Catalog and result models. The catalog is pure data (`data/tweaks.json`); no tweak is hard-coded.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RiskLevel { Recommended, Caution, Experimental, HighRisk }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TweakCategory { Performance, Input, Network, Privacy, Services, Appearance, Cleanup, Tools }

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RestartRequirement { None, Restart, Logout }

/// Effect claimed by the catalog author. NOT a measurement; the UI must not present it as one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EffectClaim { Unknown, Large, Medium, HardwareDependent, Small, Negligible, Tool }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TweakKind { Reg, RegPerAdapter, RegPerGame, PowerIdx, PowerPlan, Service, Task, Bcd, Nic, Cmd, Action }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActionKind { Clean, Open, Info, Command, Native }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TweakStatus { Applied, NotApplied, Unknown }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegValueSpec {
    #[serde(default = "default_hive")]
    pub hive: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(rename = "type", default = "default_type")]
    pub ty: String,
    #[serde(default)]
    pub value: Value,
}
fn default_hive() -> String { "HKCU".into() }
fn default_type() -> String { "dword".into() }

impl RegValueSpec {
    pub fn as_i64(&self) -> i64 {
        match &self.value {
            Value::Number(n) => n.as_i64().unwrap_or(0),
            Value::String(s) => s.parse().unwrap_or(0),
            Value::Bool(b) => i64::from(*b),
            _ => 0,
        }
    }
    pub fn as_string(&self) -> String {
        match &self.value {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerAdapterSpec {
    #[serde(default)]
    pub path_template: String,
    #[serde(default)]
    pub values: Vec<RegValueSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PowerSettingSpec {
    pub subgroup: String,
    pub setting: String,
    pub value: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BcdSpec { pub key: String, pub value: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NicSpec { #[serde(default)] pub keywords: Vec<String>, pub value: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegDwordCheck {
    #[serde(default = "hklm")]
    pub hive: String,
    pub path: String,
    pub name: String,
    pub expected: i64,
}
fn hklm() -> String { "HKLM".into() }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CmdCheck {
    #[serde(default)]
    pub reg_dword: Option<RegDwordCheck>,
    #[serde(default)]
    pub ps: Option<String>,
    #[serde(default)]
    pub equals_false: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CmdSpec {
    #[serde(default)]
    pub on: Vec<Vec<String>>,
    #[serde(default)]
    pub off: Vec<Vec<String>>,
    #[serde(default)]
    pub has_check: bool,
    #[serde(default)]
    pub check: Option<CmdCheck>,
}

/// One catalog entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TweakDefinition {
    pub id: String,
    pub category: TweakCategory,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub consequences: String,
    pub kind: TweakKind,
    pub risk: RiskLevel,
    #[serde(default)]
    pub requires_admin: bool,
    pub restart: RestartRequirement,
    pub claimed_effect: EffectClaim,
    /// Optional page the item belongs to ("keyboard" or "mouse"). Items with a section are shown on that page, not in Optimizations.
    #[serde(default)]
    pub section: Option<String>,

    // reg*
    #[serde(default)] pub values: Option<Vec<RegValueSpec>>,
    #[serde(default)] pub per_adapter: Option<PerAdapterSpec>,
    #[serde(default)] pub per_game: Option<Vec<RegValueSpec>>,
    #[serde(default)] pub post: Option<String>,
    // power
    #[serde(default)] pub power: Option<PowerSettingSpec>,
    #[serde(default)] pub plan: Option<String>,
    // system
    #[serde(default)] pub service: Option<String>,
    #[serde(default)] pub tasks: Option<Vec<String>>,
    #[serde(default)] pub bcd: Option<BcdSpec>,
    #[serde(default)] pub nic: Option<NicSpec>,
    #[serde(default)] pub cmd: Option<CmdSpec>,
    // action
    #[serde(default)] pub action: Option<ActionKind>,
    #[serde(default)] pub button: Option<String>,
    #[serde(default)] pub confirm: Option<String>,
    #[serde(default)] pub shows_result: bool,
    #[serde(default)] pub patterns: Option<Vec<String>>,
    #[serde(default)] pub target: Option<String>,
    #[serde(default)] pub commands: Option<Vec<Vec<String>>>,
    #[serde(default)] pub success_message: Option<String>,
    #[serde(default)] pub timeout_seconds: Option<u32>,
    #[serde(default)] pub native: Option<String>,
}

impl TweakDefinition {
    pub fn is_action(&self) -> bool { self.kind == TweakKind::Action }
    pub fn is_simple(&self) -> bool {
        matches!(self.kind, TweakKind::Reg | TweakKind::RegPerAdapter | TweakKind::PowerIdx | TweakKind::PowerPlan)
    }
}

/// Outcome of an engine operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TweakResult {
    pub success: bool,
    pub message: String,
    pub restart: RestartRequirement,
}

impl TweakResult {
    pub fn ok(message: impl Into<String>, restart: RestartRequirement) -> Self {
        Self { success: true, message: message.into(), restart }
    }
    pub fn fail(message: impl Into<String>) -> Self {
        Self { success: false, message: message.into(), restart: RestartRequirement::None }
    }
}

/// Extra context for operations (currently only the game exe for per-game tweaks).
#[derive(Debug, Clone, Default)]
pub struct TweakContext { pub game_exe_path: Option<String> }
