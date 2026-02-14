use serde_json::Value;

use crate::documents::ProtocolDocument;
use crate::resolver::ResolverContext;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionRef {
    pub protocol: String,
    pub version: String,
    pub action: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryRef {
    pub protocol: String,
    pub version: String,
    pub query: String,
}

#[derive(Debug, Clone)]
pub struct ResolvedActionRef<'a> {
    pub reference: ActionRef,
    pub protocol: &'a ProtocolDocument,
    pub action_spec: &'a Value,
}

#[derive(Debug, Clone)]
pub struct ResolvedQueryRef<'a> {
    pub reference: QueryRef,
    pub protocol: &'a ProtocolDocument,
    pub query_spec: &'a Value,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReferenceError {
    #[error("invalid reference format: {value}")]
    InvalidFormat { value: String },
    #[error("protocol not found: {protocol}@{version}")]
    ProtocolNotFound { protocol: String, version: String },
    #[error("action not found: {action} in {protocol}@{version}")]
    ActionNotFound {
        protocol: String,
        version: String,
        action: String,
    },
    #[error("query not found: {query} in {protocol}@{version}")]
    QueryNotFound {
        protocol: String,
        version: String,
        query: String,
    },
}

pub fn parse_action_ref(input: &str) -> Result<ActionRef, ReferenceError> {
    let (protocol, version, leaf) = parse_reference_common(input)?;
    Ok(ActionRef {
        protocol,
        version,
        action: leaf,
    })
}

pub fn parse_query_ref(input: &str) -> Result<QueryRef, ReferenceError> {
    let (protocol, version, leaf) = parse_reference_common(input)?;
    Ok(QueryRef {
        protocol,
        version,
        query: leaf,
    })
}

pub fn resolve_action_ref<'a>(
    context: &'a ResolverContext,
    input: &str,
) -> Result<ResolvedActionRef<'a>, ReferenceError> {
    let reference = parse_action_ref(input)?;
    let key = format!("{}@{}", reference.protocol, reference.version);
    let protocol = context
        .protocols
        .get(&key)
        .ok_or_else(|| ReferenceError::ProtocolNotFound {
            protocol: reference.protocol.clone(),
            version: reference.version.clone(),
        })?;
    let action_spec =
        protocol
            .actions
            .get(&reference.action)
            .ok_or_else(|| ReferenceError::ActionNotFound {
                protocol: reference.protocol.clone(),
                version: reference.version.clone(),
                action: reference.action.clone(),
            })?;

    Ok(ResolvedActionRef {
        reference,
        protocol,
        action_spec,
    })
}

pub fn resolve_query_ref<'a>(
    context: &'a ResolverContext,
    input: &str,
) -> Result<ResolvedQueryRef<'a>, ReferenceError> {
    let reference = parse_query_ref(input)?;
    let key = format!("{}@{}", reference.protocol, reference.version);
    let protocol = context
        .protocols
        .get(&key)
        .ok_or_else(|| ReferenceError::ProtocolNotFound {
            protocol: reference.protocol.clone(),
            version: reference.version.clone(),
        })?;
    let query_spec =
        protocol
            .queries
            .get(&reference.query)
            .ok_or_else(|| ReferenceError::QueryNotFound {
                protocol: reference.protocol.clone(),
                version: reference.version.clone(),
                query: reference.query.clone(),
            })?;

    Ok(ResolvedQueryRef {
        reference,
        protocol,
        query_spec,
    })
}

fn parse_reference_common(input: &str) -> Result<(String, String, String), ReferenceError> {
    let Some((protocol_version, leaf)) = input.split_once('/') else {
        return Err(ReferenceError::InvalidFormat {
            value: input.to_string(),
        });
    };
    let Some((protocol, version)) = protocol_version.split_once('@') else {
        return Err(ReferenceError::InvalidFormat {
            value: input.to_string(),
        });
    };

    if protocol.is_empty() || version.is_empty() || leaf.is_empty() {
        return Err(ReferenceError::InvalidFormat {
            value: input.to_string(),
        });
    }

    Ok((protocol.to_string(), version.to_string(), leaf.to_string()))
}

#[cfg(test)]
#[path = "reference_test.rs"]
mod tests;
