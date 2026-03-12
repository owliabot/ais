//! CEL-shaped expression support with deliberately reduced scope.

pub mod builtins;
pub mod evaluator;
mod runtime;
pub mod scope;
pub mod typing;

pub use evaluator::{CelEvaluationError, CelEvaluator};
pub use runtime::value::CelValue;
pub use scope::CelScope;
pub use typing::{CelExpressionKind, CelTypeChecker};

#[cfg(test)]
mod tests;
