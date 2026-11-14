//! Active physical network adapters (Ethernet / Wi-Fi), discovered with PowerShell.

use crate::runner::{powershell, Runner};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdapterInfo { pub name: String, pub guid: String, pub description: String, pub speed_mbps: u64, pub dns: Vec<String>, pub addresses: Vec<String>,
    /// "ethernet", "wifi" or "other" (from NdisPhysicalMedium: 14 = 802.3, 9 = native 802.11).
    pub kind: String }

pub trait Adapters: Send + Sync { fn active(&self) -> Vec<AdapterInfo>; }

pub struct PsAdapters { pub runner: std::sync::Arc<dyn Runner> }

const SCRIPT: &str = "Get-NetAdapter -Physical | Where-Object {$_.Status -eq 'Up'} | ForEach-Object { \
$ip = Get-NetIPAddress -InterfaceIndex $_.ifIndex -ErrorAction SilentlyContinue | ForEach-Object { $_.IPAddress }; \
$dns = (Get-DnsClientServerAddress -InterfaceIndex $_.ifIndex -ErrorAction SilentlyContinue).ServerAddresses; \
[pscustomobject]@{N=$_.Name;G=$_.InterfaceGuid;D=$_.InterfaceDescription;S=[int64]($_.Speed / 1000000);M=[int]$_.NdisPhysicalMedium;A=@($ip);DNS=@($dns)} } | ConvertTo-Json -Compress -Depth 3";

fn strings(v: &Value) -> Vec<String> {
    match v { Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(), Value::String(s) => vec![s.clone()], _ => vec![] }
}

pub fn parse(output: &str) -> Vec<AdapterInfo> {
    let (Some(a), Some(b)) = (output.find(|c| c == '[' || c == '{'), output.rfind(|c| c == ']' || c == '}')) else { return vec![] };
    let Ok(v) = serde_json::from_str::<Value>(&output[a..=b]) else { return vec![] };
    let items = match v { Value::Array(a) => a, o @ Value::Object(_) => vec![o], _ => vec![] };
    items.iter().filter_map(|i| {
        let name = i["N"].as_str()?.to_string();
        Some(AdapterInfo {
            name,
            guid: i["G"].as_str().unwrap_or("").trim_matches(|c| c == '{' || c == '}').to_string(),
            description: i["D"].as_str().unwrap_or("").to_string(),
            speed_mbps: i["S"].as_u64().unwrap_or(0),
            addresses: strings(&i["A"]), dns: strings(&i["DNS"]),
            kind: match i["M"].as_u64() { Some(14) => "ethernet", Some(9) => "wifi", _ => "other" }.into(),
        })
    }).filter(|a| !a.guid.is_empty()).collect()
}

impl Adapters for PsAdapters {
    fn active(&self) -> Vec<AdapterInfo> {
        let r = self.runner.run(&powershell(SCRIPT), 40);
        if r.success() { parse(&r.output) } else { vec![] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_single_and_multiple_adapters() {
        let one = r#"{"N":"Ethernet","G":"{ABC-1}","D":"Intel","S":1000,"A":"192.168.0.2","DNS":["1.1.1.1","1.0.0.1"]}"#;
        let a = parse(one);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].guid, "ABC-1");
        assert_eq!(a[0].dns.len(), 2);
        assert_eq!(a[0].addresses, vec!["192.168.0.2".to_string()]);
        let two = format!("[{one},{one}]");
        assert_eq!(parse(&two).len(), 2);
        assert!(parse("garbage").is_empty());
        assert_eq!(a[0].kind, "other");
        let w = parse(r#"{"N":"Wi-Fi","G":"{W}","D":"Intel AX","S":866,"M":9,"A":[],"DNS":[]}"#);
        assert_eq!(w[0].kind, "wifi");
        assert_eq!(parse(r#"{"N":"E","G":"{E}","D":"x","S":1,"M":14}"#)[0].kind, "ethernet");
    }
}
