use crate::cel::runtime::parser::{parse_expression, ParseError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CelExpressionKind {
    Derivation,
    PolicyPredicate,
    EffectPredicate,
    BoundaryPredicate,
}

#[derive(Debug, Default)]
pub struct CelTypeChecker;

impl CelTypeChecker {
    pub fn validate(&self, _kind: CelExpressionKind, expression: &str) -> Result<(), ParseError> {
        let _ = parse_expression(expression)?;
        Ok(())
    }
}
