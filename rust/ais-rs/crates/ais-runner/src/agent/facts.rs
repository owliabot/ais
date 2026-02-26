use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactLayer {
    Seed,
    Observed,
    Derived,
}

impl FactLayer {
    fn priority(self) -> u8 {
        match self {
            FactLayer::Seed => 10,
            FactLayer::Observed => 20,
            FactLayer::Derived => 15,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactSource {
    UserInput,
    QueryObserved,
    ConfigDerived,
    RuntimeProvided,
    IntentInferred,
    DerivedComputation,
}

impl FactSource {
    fn priority(self) -> u8 {
        match self {
            FactSource::UserInput => 100,
            FactSource::QueryObserved => 90,
            FactSource::ConfigDerived => 80,
            FactSource::RuntimeProvided => 70,
            FactSource::DerivedComputation => 60,
            FactSource::IntentInferred => 50,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FactEntry {
    pub value: Value,
    pub layer: FactLayer,
    pub source: FactSource,
    pub provenance: String,
    #[serde(default)]
    pub stability: FactStability,
    #[serde(default)]
    pub observed_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FactStability {
    Stable,
    Volatile,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactUpsertResult {
    Inserted,
    Replaced,
    Ignored,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FactStore {
    entries: BTreeMap<String, FactEntry>,
}

#[allow(dead_code)]
impl FactStore {
    pub fn upsert(
        &mut self,
        key: impl Into<String>,
        value: Value,
        layer: FactLayer,
        source: FactSource,
        provenance: impl Into<String>,
    ) -> FactUpsertResult {
        self.upsert_with_observed_at(
            key,
            value,
            layer,
            source,
            provenance,
            Some(current_unix_ms()),
        )
    }

    pub fn upsert_with_observed_at(
        &mut self,
        key: impl Into<String>,
        value: Value,
        layer: FactLayer,
        source: FactSource,
        provenance: impl Into<String>,
        observed_at_ms: Option<u64>,
    ) -> FactUpsertResult {
        let key = key.into();
        let stability = infer_fact_stability(key.as_str(), source);
        let candidate = FactEntry {
            value,
            layer,
            source,
            provenance: normalize_provenance(provenance.into()),
            stability,
            observed_at_ms: observed_at_ms.filter(|_| source == FactSource::QueryObserved),
        };

        let Some(existing) = self.entries.get(key.as_str()) else {
            self.entries.insert(key, candidate);
            return FactUpsertResult::Inserted;
        };

        if should_preserve_intent_fact(existing, &candidate) {
            return FactUpsertResult::Ignored;
        }

        if candidate.source.priority() > existing.source.priority()
            || (candidate.source.priority() == existing.source.priority()
                && candidate.layer.priority() > existing.layer.priority())
        {
            self.entries.insert(key, candidate);
            return FactUpsertResult::Replaced;
        }

        FactUpsertResult::Ignored
    }

    #[allow(dead_code)]
    pub fn merge(&mut self, other: &FactStore) {
        for (key, entry) in &other.entries {
            self.upsert(
                key.clone(),
                entry.value.clone(),
                entry.layer,
                entry.source,
                entry.provenance.clone(),
            );
        }
    }

    #[allow(dead_code)]
    pub fn get(&self, key: &str) -> Option<&FactEntry> {
        self.entries.get(key)
    }

    pub fn has_fresh_volatile_signal(
        &self,
        signal: VolatileFactSignal,
        max_age_ms: u64,
        now_ms: u64,
    ) -> bool {
        self.entries.iter().any(|(key, entry)| {
            entry.stability == FactStability::Volatile
                && entry.source == FactSource::QueryObserved
                && volatile_signal_matches_key(signal, key.as_str())
                && entry
                    .observed_at_ms
                    .is_some_and(|timestamp| now_ms.saturating_sub(timestamp) <= max_age_ms)
        })
    }

    pub fn any_key_ends_with(&self, suffix: &str) -> bool {
        self.entries.keys().any(|key| key.ends_with(suffix))
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn keys(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }

    pub fn to_planning_value(&self) -> Value {
        self.to_projected_planning_value(usize::MAX)
    }

    pub fn to_projected_planning_value(&self, max_entries: usize) -> Value {
        let max_entries = max_entries.max(1);
        let mut facts = Map::<String, Value>::new();
        let mut meta = Map::<String, Value>::new();
        let mut selected_keys = Vec::<String>::new();
        for priority_key in ["owner", "wallet.default"] {
            if self.entries.contains_key(priority_key) {
                selected_keys.push(priority_key.to_string());
            }
        }
        for key in self.entries.keys() {
            if selected_keys.len() >= max_entries {
                break;
            }
            if selected_keys.iter().any(|item| item == key) {
                continue;
            }
            selected_keys.push(key.clone());
        }
        let truncated = self.entries.len().saturating_sub(selected_keys.len());
        for key in selected_keys {
            let Some(entry) = self.entries.get(key.as_str()) else {
                continue;
            };
            facts.insert(key.clone(), entry.value.clone());
            meta.insert(
                key.clone(),
                serde_json::json!({
                    "layer": entry.layer,
                    "source": entry.source,
                    "source_priority": entry.source.priority(),
                    "provenance": entry.provenance,
                    "stability": entry.stability,
                    "observed_at_ms": entry.observed_at_ms,
                }),
            );
        }
        if truncated > 0 {
            meta.insert(
                "_truncated_entries".to_string(),
                Value::Number((truncated as u64).into()),
            );
        }
        Value::Object(
            [
                ("facts".to_string(), Value::Object(facts)),
                ("meta".to_string(), Value::Object(meta)),
            ]
            .into_iter()
            .collect(),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolatileFactSignal {
    Balance,
    Allowance,
}

fn normalize_provenance(raw: String) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        "unknown".to_string()
    } else {
        trimmed.to_string()
    }
}

fn infer_fact_stability(key: &str, source: FactSource) -> FactStability {
    let lowered = key.to_lowercase();
    if lowered.contains("decimal") {
        return FactStability::Stable;
    }
    if matches!(source, FactSource::QueryObserved)
        && (lowered.contains("balance") || lowered.contains("allowance"))
    {
        return FactStability::Volatile;
    }
    FactStability::Unknown
}

fn should_preserve_intent_fact(existing: &FactEntry, candidate: &FactEntry) -> bool {
    existing.source == FactSource::IntentInferred
        && candidate.source == FactSource::QueryObserved
        && candidate.stability == FactStability::Volatile
}

fn volatile_signal_matches_key(signal: VolatileFactSignal, key: &str) -> bool {
    let lowered = key.to_lowercase();
    match signal {
        VolatileFactSignal::Balance => lowered.contains("balance"),
        VolatileFactSignal::Allowance => lowered.contains("allowance"),
    }
}

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn upsert_keeps_higher_priority_source() {
        let mut store = FactStore::default();
        assert_eq!(
            store.upsert(
                "owner",
                json!("0xintent"),
                FactLayer::Seed,
                FactSource::IntentInferred,
                "intent"
            ),
            FactUpsertResult::Inserted
        );
        assert_eq!(
            store.upsert(
                "owner",
                json!("0xquery"),
                FactLayer::Observed,
                FactSource::QueryObserved,
                "query:owner"
            ),
            FactUpsertResult::Replaced
        );
        assert_eq!(
            store.get("owner").and_then(|entry| entry.value.as_str()),
            Some("0xquery")
        );
        assert_eq!(
            store.get("owner").map(|entry| entry.source),
            Some(FactSource::QueryObserved)
        );
    }

    #[test]
    fn upsert_keeps_existing_on_same_priority() {
        let mut store = FactStore::default();
        assert_eq!(
            store.upsert(
                "owner",
                json!("0xfirst"),
                FactLayer::Seed,
                FactSource::RuntimeProvided,
                "runtime.inputs.owner"
            ),
            FactUpsertResult::Inserted
        );
        assert_eq!(
            store.upsert(
                "owner",
                json!("0xsecond"),
                FactLayer::Seed,
                FactSource::RuntimeProvided,
                "runtime.inputs.wallet"
            ),
            FactUpsertResult::Ignored
        );
        assert_eq!(
            store.get("owner").and_then(|entry| entry.value.as_str()),
            Some("0xfirst")
        );
        assert_eq!(
            store.get("owner").map(|entry| entry.provenance.as_str()),
            Some("runtime.inputs.owner")
        );
    }

    #[test]
    fn volatile_query_does_not_override_intent_fact() {
        let mut store = FactStore::default();
        assert_eq!(
            store.upsert(
                "spend.balance",
                json!("100"),
                FactLayer::Seed,
                FactSource::IntentInferred,
                "intent.balance"
            ),
            FactUpsertResult::Inserted
        );
        assert_eq!(
            store.upsert_with_observed_at(
                "spend.balance",
                json!("50"),
                FactLayer::Observed,
                FactSource::QueryObserved,
                "query.balance",
                Some(1_000),
            ),
            FactUpsertResult::Ignored
        );
        let winner = store.get("spend.balance").expect("winner");
        assert_eq!(winner.value, json!("100"));
        assert_eq!(winner.source, FactSource::IntentInferred);
    }

    #[test]
    fn merge_preserves_provenance_of_winner() {
        let mut base = FactStore::default();
        base.upsert(
            "amount.atomic",
            json!("1000000000000000000"),
            FactLayer::Derived,
            FactSource::DerivedComputation,
            "derive:to_atomic",
        );
        let mut incoming = FactStore::default();
        incoming.upsert(
            "amount.atomic",
            json!("1200000000000000000"),
            FactLayer::Observed,
            FactSource::QueryObserved,
            "query:quote.amount",
        );
        base.merge(&incoming);

        let winner = base.get("amount.atomic").expect("winner");
        assert_eq!(winner.value, json!("1200000000000000000"));
        assert_eq!(winner.source, FactSource::QueryObserved);
        assert_eq!(winner.provenance, "query:quote.amount");
    }

    #[test]
    fn planning_value_contains_facts_and_meta() {
        let mut store = FactStore::default();
        store.upsert(
            "token.decimals",
            json!(18),
            FactLayer::Observed,
            FactSource::QueryObserved,
            "query:erc20.decimals",
        );
        let value = store.to_planning_value();
        assert_eq!(value.pointer("/facts/token.decimals"), Some(&json!(18)));
        assert_eq!(
            value.pointer("/meta/token.decimals/provenance"),
            Some(&json!("query:erc20.decimals"))
        );
        assert_eq!(
            value.pointer("/meta/token.decimals/stability"),
            Some(&json!("stable"))
        );
    }

    #[test]
    fn volatile_fact_freshness_uses_observed_time() {
        let now = 1_000_000u64;
        let mut store = FactStore::default();
        store.upsert_with_observed_at(
            "wallet.balance.native",
            json!("100"),
            FactLayer::Observed,
            FactSource::QueryObserved,
            "query:native.balance",
            Some(now.saturating_sub(10_000)),
        );
        assert!(store.has_fresh_volatile_signal(VolatileFactSignal::Balance, 30_000, now));
        assert!(!store.has_fresh_volatile_signal(VolatileFactSignal::Balance, 5_000, now));
    }
}
