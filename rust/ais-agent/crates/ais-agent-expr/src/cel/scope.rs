use std::collections::BTreeMap;

use serde_json::Value;

use crate::cel::runtime::value::{CelContext, CelValue};

#[derive(Debug, Clone, Default)]
pub struct CelScope {
    bindings: CelContext,
}

impl CelScope {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_value(&mut self, key: impl Into<String>, value: CelValue) {
        self.bindings.insert(key.into(), value);
    }

    pub fn insert_json(&mut self, key: impl Into<String>, value: Value) {
        self.insert_value(key, CelValue::from(value));
    }

    pub fn from_json_map(values: BTreeMap<String, Value>) -> Self {
        let mut scope = Self::new();
        for (key, value) in values {
            scope.insert_json(key, value);
        }
        scope
    }

    pub fn bindings(&self) -> &CelContext {
        &self.bindings
    }
}
