use ais_core::{stable_hash_hex, StableJsonOptions};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub(in super::super) const CONTEXT_ENVELOPE_SCHEMA: &str = "ais-agent-context-envelope";
pub(in super::super) const CONTEXT_ENVELOPE_SCHEMA_VERSION: u64 = 1;

const LEGACY_CONTEXT_VERSION: &str = "context_version";
const LEGACY_CONTEXT_HASH: &str = "context_hash";
const LEGACY_CONTEXT_UNCHANGED: &str = "context_unchanged";
const ENVELOPE_FIELD: &str = "context_envelope";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(in super::super) struct ContextEnvelope {
    pub(in super::super) schema: String,
    pub(in super::super) schema_version: u64,
    pub(in super::super) version: u64,
    pub(in super::super) hash: String,
    pub(in super::super) unchanged: bool,
}

impl ContextEnvelope {
    pub(in super::super) fn from_payload(
        payload: &Value,
        version: u64,
        previous_hash: Option<&str>,
    ) -> Self {
        let hash = context_hash(payload);
        let unchanged = previous_hash == Some(hash.as_str());
        Self {
            schema: CONTEXT_ENVELOPE_SCHEMA.to_string(),
            schema_version: CONTEXT_ENVELOPE_SCHEMA_VERSION,
            version,
            hash,
            unchanged,
        }
    }

    #[cfg(test)]
    pub(in super::super) fn from_summary(summary: &Value) -> Option<Self> {
        Self::from_summary_with_options(summary, false)
    }

    #[cfg(test)]
    pub(in super::super) fn from_summary_with_options(
        summary: &Value,
        verify_hash: bool,
    ) -> Option<Self> {
        let payload = payload_from_summary(summary);
        Self::from_summary_envelope_field(summary, &payload, verify_hash)
            .or_else(|| Self::from_legacy_summary(summary, &payload, verify_hash))
    }

    pub(in super::super) fn to_compat_summary(&self, payload: Value) -> Value {
        let mut object = payload.as_object().cloned().unwrap_or_default();
        object.insert(
            LEGACY_CONTEXT_VERSION.to_string(),
            Value::Number(self.version.into()),
        );
        object.insert(
            LEGACY_CONTEXT_HASH.to_string(),
            Value::String(self.hash.clone()),
        );
        object.insert(
            LEGACY_CONTEXT_UNCHANGED.to_string(),
            Value::Bool(self.unchanged),
        );
        let envelope_value = serde_json::to_value(self).unwrap_or_else(|_| {
            Value::Object(Map::from_iter([
                (
                    "schema".to_string(),
                    Value::String(CONTEXT_ENVELOPE_SCHEMA.to_string()),
                ),
                (
                    "schema_version".to_string(),
                    Value::Number(CONTEXT_ENVELOPE_SCHEMA_VERSION.into()),
                ),
                ("version".to_string(), Value::Number(self.version.into())),
                ("hash".to_string(), Value::String(self.hash.clone())),
                ("unchanged".to_string(), Value::Bool(self.unchanged)),
            ]))
        });
        object.insert(ENVELOPE_FIELD.to_string(), envelope_value);
        Value::Object(object)
    }

    #[cfg(test)]
    fn from_summary_envelope_field(
        summary: &Value,
        payload: &Value,
        verify_hash: bool,
    ) -> Option<Self> {
        let envelope_value = summary.get(ENVELOPE_FIELD)?;
        let envelope = serde_json::from_value::<ContextEnvelope>(envelope_value.clone()).ok()?;
        envelope.validate(payload, verify_hash)
    }

    #[cfg(test)]
    fn from_legacy_summary(summary: &Value, payload: &Value, verify_hash: bool) -> Option<Self> {
        let version = summary.get(LEGACY_CONTEXT_VERSION)?.as_u64()?;
        let hash = summary
            .get(LEGACY_CONTEXT_HASH)
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| context_hash(payload));
        let unchanged = summary
            .get(LEGACY_CONTEXT_UNCHANGED)
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Self {
            schema: CONTEXT_ENVELOPE_SCHEMA.to_string(),
            schema_version: CONTEXT_ENVELOPE_SCHEMA_VERSION,
            version,
            hash,
            unchanged,
        }
        .validate(payload, verify_hash)
    }

    #[cfg(test)]
    fn validate(self, payload: &Value, verify_hash: bool) -> Option<Self> {
        if self.schema != CONTEXT_ENVELOPE_SCHEMA {
            return None;
        }
        if self.schema_version != CONTEXT_ENVELOPE_SCHEMA_VERSION {
            return None;
        }
        if verify_hash && self.hash != context_hash(payload) {
            return None;
        }
        Some(self)
    }
}

#[cfg(test)]
pub(in super::super) fn payload_from_summary(summary: &Value) -> Value {
    let mut object = summary.as_object().cloned().unwrap_or_default();
    object.remove(LEGACY_CONTEXT_VERSION);
    object.remove(LEGACY_CONTEXT_HASH);
    object.remove(LEGACY_CONTEXT_UNCHANGED);
    object.remove(ENVELOPE_FIELD);
    Value::Object(object)
}

fn context_hash(payload: &Value) -> String {
    stable_hash_hex(payload, &StableJsonOptions::default())
        .unwrap_or_else(|_| "context-hash-unavailable".to_string())
}

#[cfg(test)]
#[path = "../tests/context_envelope.rs"]
mod tests;
