use std::collections::BTreeMap;

use num_bigint::BigInt;
use serde_json::Value;

use crate::cel::runtime::numeric::Decimal;

#[derive(Debug, Clone, PartialEq)]
pub enum CelValue {
    Null,
    Bool(bool),
    Integer(BigInt),
    Decimal(Decimal),
    String(String),
    List(Vec<CelValue>),
    Map(BTreeMap<String, CelValue>),
}

pub type CelContext = BTreeMap<String, CelValue>;

impl From<Value> for CelValue {
    fn from(value: Value) -> Self {
        match value {
            Value::Null => Self::Null,
            Value::Bool(value) => Self::Bool(value),
            Value::Number(number) => {
                let rendered = number.to_string();
                if rendered.contains('.') {
                    Decimal::parse(rendered.as_str())
                        .map(Self::Decimal)
                        .unwrap_or(Self::String(rendered))
                } else {
                    rendered
                        .parse::<BigInt>()
                        .map(Self::Integer)
                        .unwrap_or(Self::String(rendered))
                }
            }
            Value::String(value) => Self::String(value),
            Value::Array(items) => Self::List(items.into_iter().map(Self::from).collect()),
            Value::Object(map) => Self::Map(
                map.into_iter()
                    .map(|(key, value)| (key, Self::from(value)))
                    .collect(),
            ),
        }
    }
}

impl CelValue {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            CelValue::Bool(value) => Some(*value),
            _ => None,
        }
    }
}
