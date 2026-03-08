use serde_json::Value;

use super::super::candidates::{CandidateContext, CandidateSearchRequest};
use super::super::intent_segmented::{
    control_semantics_search_hint_payload, is_control_semantics_query,
};
use super::args::{CandidateDetailArgs, CatalogDiscoverArgs, ListCandidatesFilterArgs};
use crate::error::RunnerError;

pub(super) enum CatalogDiscoverPayload {
    Search(Value),
    Inventory(Value),
}

pub(super) fn decode_candidate_detail_payload(
    arguments: Value,
    candidate_context: Option<&CandidateContext>,
) -> Result<Value, RunnerError> {
    let Some(context) = candidate_context else {
        return Err(RunnerError::Llm(
            "candidate detail tool is unavailable".to_string(),
        ));
    };
    let args: CandidateDetailArgs = serde_json::from_value(arguments)
        .map_err(|error| RunnerError::Llm(format!("invalid get_candidate_detail args: {error}")))?;
    Ok(context.get_details_for_refs(&args.refs))
}

pub(super) fn decode_catalog_discover_payload(
    arguments: Value,
    candidate_context: Option<&CandidateContext>,
) -> Result<CatalogDiscoverPayload, RunnerError> {
    let Some(context) = candidate_context else {
        return Err(RunnerError::Llm(
            "catalog.discover requires workspace candidate context".to_string(),
        ));
    };

    let args: CatalogDiscoverArgs = serde_json::from_value(arguments)
        .map_err(|error| RunnerError::Llm(format!("invalid catalog.discover args: {error}")))?;
    let has_query = args.query.as_ref().is_some_and(|q| !q.trim().is_empty());
    if has_query {
        let query = args.query;
        let payload = if is_control_semantics_query(query.as_deref()) {
            control_semantics_search_hint_payload(
                query.clone(),
                args.kind.clone(),
                args.chain.clone(),
                args.min_risk_level,
                args.max_risk_level,
                args.limit,
            )
        } else {
            context.search_candidates(&CandidateSearchRequest {
                query,
                kind: args.kind,
                chain: args.chain,
                min_risk_level: args.min_risk_level,
                max_risk_level: args.max_risk_level,
                limit: args.limit,
            })
        };
        return Ok(CatalogDiscoverPayload::Search(payload));
    }

    let filter = ListCandidatesFilterArgs {
        chain: args
            .chain
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        protocol: args
            .protocol
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase),
    };
    let payload =
        super::super::intent_segmented::candidate_snapshot(candidate_context, Some(filter));
    Ok(CatalogDiscoverPayload::Inventory(payload))
}
