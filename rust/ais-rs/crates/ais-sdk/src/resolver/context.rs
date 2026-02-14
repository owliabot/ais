use serde_json::{Map, Value};
use std::collections::BTreeMap;

use crate::documents::ProtocolDocument;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ResolverError {
    #[error("invalid path: {0}")]
    InvalidPath(String),
    #[error("path not found: {0}")]
    NotFound(String),
    #[error("non-object intermediate at: {0}")]
    NonObjectIntermediate(String),
}

#[derive(Debug, Clone)]
pub struct ResolverContext {
    pub runtime: Value,
    pub protocols: BTreeMap<String, ProtocolDocument>,
}

impl ResolverContext {
    pub fn new() -> Self {
        Self {
            runtime: Value::Object(Map::new()),
            protocols: BTreeMap::new(),
        }
    }

    pub fn with_runtime(runtime: Value) -> Self {
        let runtime = if runtime.is_object() {
            runtime
        } else {
            Value::Object(Map::new())
        };
        Self {
            runtime,
            protocols: BTreeMap::new(),
        }
    }

    pub fn register_protocol(&mut self, protocol: ProtocolDocument) -> Option<ProtocolDocument> {
        let key = protocol_key(&protocol);
        self.protocols.insert(key, protocol)
    }

    pub fn get_ref(&self, path: &str) -> Result<Value, ResolverError> {
        let segments = split_ref_path(path)?;
        let mut current = &self.runtime;
        for segment in segments {
            match segment {
                PathSegment::Key(key) => {
                    current = current
                        .as_object()
                        .and_then(|object| object.get(&key))
                        .ok_or_else(|| ResolverError::NotFound(path.to_string()))?;
                }
                PathSegment::Index(index) => {
                    current = current
                        .as_array()
                        .and_then(|items| items.get(index))
                        .ok_or_else(|| ResolverError::NotFound(path.to_string()))?;
                }
            }
        }
        Ok(current.clone())
    }

    pub fn set_ref(&mut self, path: &str, value: Value) -> Result<(), ResolverError> {
        let segments = split_ref_path(path)?;
        if segments.is_empty() {
            return Err(ResolverError::InvalidPath(path.to_string()));
        }

        if !self.runtime.is_object() {
            self.runtime = Value::Object(Map::new());
        }

        let mut current = &mut self.runtime;
        for segment in &segments[..segments.len() - 1] {
            match segment {
                PathSegment::Key(key) => {
                    let object = current
                        .as_object_mut()
                        .ok_or_else(|| ResolverError::NonObjectIntermediate(path.to_string()))?;
                    current = object
                        .entry(key.clone())
                        .or_insert_with(|| Value::Object(Map::new()));
                }
                PathSegment::Index(_) => return Err(ResolverError::InvalidPath(path.to_string())),
            }
        }

        let parent = current
            .as_object_mut()
            .ok_or_else(|| ResolverError::NonObjectIntermediate(path.to_string()))?;
        match segments.last().expect("already checked non-empty") {
            PathSegment::Key(key) => {
                parent.insert(key.clone(), value);
                Ok(())
            }
            PathSegment::Index(_) => Err(ResolverError::InvalidPath(path.to_string())),
        }
    }
}

impl Default for ResolverContext {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
enum PathSegment {
    Key(String),
    Index(usize),
}

fn split_ref_path(path: &str) -> Result<Vec<PathSegment>, ResolverError> {
    let normalized = path.trim().trim_start_matches('$').trim_start_matches('.');
    if normalized.is_empty() {
        return Ok(Vec::new());
    }

    let mut segments = Vec::new();
    for token in normalized.split('.') {
        if token.is_empty() {
            return Err(ResolverError::InvalidPath(path.to_string()));
        }
        parse_token_with_indexes(token, &mut segments, path)?;
    }
    Ok(segments)
}

fn parse_token_with_indexes(
    token: &str,
    out: &mut Vec<PathSegment>,
    original_path: &str,
) -> Result<(), ResolverError> {
    let bytes = token.as_bytes();
    let mut position = 0usize;

    if bytes.first() != Some(&b'[') {
        let mut key_end = 0usize;
        while key_end < bytes.len() && bytes[key_end] != b'[' {
            key_end += 1;
        }
        let key = &token[..key_end];
        if key.is_empty() {
            return Err(ResolverError::InvalidPath(original_path.to_string()));
        }
        out.push(PathSegment::Key(key.to_string()));
        position = key_end;
    }

    while position < bytes.len() {
        if bytes[position] != b'[' {
            return Err(ResolverError::InvalidPath(original_path.to_string()));
        }
        position += 1;
        let start = position;
        while position < bytes.len() && bytes[position].is_ascii_digit() {
            position += 1;
        }
        if start == position || position >= bytes.len() || bytes[position] != b']' {
            return Err(ResolverError::InvalidPath(original_path.to_string()));
        }
        let index = token[start..position]
            .parse::<usize>()
            .map_err(|_| ResolverError::InvalidPath(original_path.to_string()))?;
        out.push(PathSegment::Index(index));
        position += 1;
    }

    Ok(())
}

fn protocol_key(protocol: &ProtocolDocument) -> String {
    let protocol_id = protocol
        .meta
        .as_object()
        .and_then(|meta| meta.get("protocol"))
        .and_then(Value::as_str)
        .unwrap_or("unknown-protocol");
    let version = protocol
        .meta
        .as_object()
        .and_then(|meta| meta.get("version"))
        .and_then(Value::as_str)
        .unwrap_or("0.0.0");
    format!("{protocol_id}@{version}")
}

#[cfg(test)]
#[path = "context_test.rs"]
mod tests;
