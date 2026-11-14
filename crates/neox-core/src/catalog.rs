use crate::models::*;
use serde::Deserialize;
use std::collections::HashMap;

const EMBEDDED: &str = include_str!("../data/tweaks.json");

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogFile { schema_version: i32, tweaks: Vec<TweakDefinition> }

pub struct Catalog {
    items: Vec<TweakDefinition>,
    index: HashMap<String, usize>,
}

impl Catalog {
    pub fn load_embedded() -> Result<Self, String> { Self::from_json(EMBEDDED) }

    pub fn from_json(json: &str) -> Result<Self, String> {
        let file: CatalogFile = serde_json::from_str(json).map_err(|e| format!("Invalid catalog: {e}"))?;
        if file.schema_version != 1 { return Err(format!("Unsupported catalog version: {}", file.schema_version)); }
        let mut index = HashMap::new();
        for (i, t) in file.tweaks.iter().enumerate() {
            if index.insert(t.id.to_lowercase(), i).is_some() { return Err(format!("Duplicate tweak id: {}", t.id)); }
        }
        Ok(Self { items: file.tweaks, index })
    }

    pub fn all(&self) -> &[TweakDefinition] { &self.items }
    pub fn len(&self) -> usize { self.items.len() }
    pub fn is_empty(&self) -> bool { self.items.is_empty() }
    pub fn find(&self, id: &str) -> Option<&TweakDefinition> { self.index.get(&id.to_lowercase()).map(|&i| &self.items[i]) }

    pub fn search(&self, text: &str, category: Option<TweakCategory>, risk: Option<RiskLevel>) -> Vec<&TweakDefinition> {
        let q = text.trim().to_lowercase();
        self.items.iter().filter(|t| {
            category.map_or(true, |c| t.category == c)
                && risk.map_or(true, |r| t.risk == r)
                && (q.is_empty() || t.title.to_lowercase().contains(&q) || t.description.to_lowercase().contains(&q) || t.id.to_lowercase().contains(&q))
        }).collect()
    }
}
