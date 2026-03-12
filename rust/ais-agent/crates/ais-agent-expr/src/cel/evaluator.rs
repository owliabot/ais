use crate::cel::{
    runtime::evaluator::{EvalError, RuntimeCelEvaluator},
    scope::CelScope,
};

#[derive(Debug, thiserror::Error)]
pub enum CelEvaluationError {
    #[error(transparent)]
    Eval(#[from] EvalError),
    #[error("expression did not evaluate to bool")]
    ExpectedBool,
}

#[derive(Debug, Default)]
pub struct CelEvaluator {
    runtime: RuntimeCelEvaluator,
}

impl CelEvaluator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn evaluate_value(
        &mut self,
        expression: &str,
        scope: &CelScope,
    ) -> Result<crate::cel::runtime::value::CelValue, CelEvaluationError> {
        Ok(self.runtime.evaluate(expression, scope.bindings())?)
    }

    pub fn evaluate_bool(
        &mut self,
        expression: &str,
        scope: &CelScope,
    ) -> Result<bool, CelEvaluationError> {
        self.evaluate_value(expression, scope)?
            .as_bool()
            .ok_or(CelEvaluationError::ExpectedBool)
    }
}
