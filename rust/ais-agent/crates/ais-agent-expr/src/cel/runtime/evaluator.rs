use std::collections::HashMap;

use num_bigint::{BigInt, Sign};
use num_traits::{Signed, ToPrimitive, Zero};

use crate::cel::runtime::{
    ast::{AstNode, BinaryOp, UnaryOp},
    numeric::{Decimal, NumericError},
    parser::{parse_expression, ParseError},
    value::{CelContext, CelValue},
};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum EvalError {
    #[error("parse error: {0}")]
    Parse(#[from] ParseError),
    #[error("numeric error: {0}")]
    Numeric(#[from] NumericError),
    #[error("undefined identifier: {0}")]
    UndefinedIdentifier(String),
    #[error("invalid member access: {0}")]
    InvalidMemberAccess(String),
    #[error("invalid index access")]
    InvalidIndexAccess,
    #[error("type mismatch: {0}")]
    TypeMismatch(String),
    #[error("unsupported call expression")]
    UnsupportedCall,
}

#[derive(Debug, Default)]
pub struct RuntimeCelEvaluator {
    cache: HashMap<String, AstNode>,
}

impl RuntimeCelEvaluator {
    pub fn evaluate(
        &mut self,
        expression: &str,
        context: &CelContext,
    ) -> Result<CelValue, EvalError> {
        let ast = if let Some(ast) = self.cache.get(expression) {
            ast.clone()
        } else {
            let parsed = parse_expression(expression)?;
            self.cache.insert(expression.to_owned(), parsed.clone());
            parsed
        };

        evaluate_ast(&ast, context)
    }
}

pub fn evaluate_ast(ast: &AstNode, context: &CelContext) -> Result<CelValue, EvalError> {
    match ast {
        AstNode::Null => Ok(CelValue::Null),
        AstNode::Bool(value) => Ok(CelValue::Bool(*value)),
        AstNode::Integer(value) => {
            Ok(CelValue::Integer(value.parse::<BigInt>().map_err(
                |_| EvalError::TypeMismatch("invalid integer literal".to_owned()),
            )?))
        }
        AstNode::Decimal(value) => Ok(CelValue::Decimal(Decimal::parse(value)?)),
        AstNode::String(value) => Ok(CelValue::String(value.clone())),
        AstNode::Identifier(name) => context
            .get(name)
            .cloned()
            .ok_or_else(|| EvalError::UndefinedIdentifier(name.clone())),
        AstNode::List(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(evaluate_ast(item, context)?);
            }
            Ok(CelValue::List(out))
        }
        AstNode::Unary { op, expr } => evaluate_unary(*op, evaluate_ast(expr, context)?),
        AstNode::Binary { left, op, right } => evaluate_binary(
            evaluate_ast(left, context)?,
            *op,
            evaluate_ast(right, context)?,
        ),
        AstNode::Ternary {
            condition,
            then_expr,
            else_expr,
        } => {
            if as_bool(&evaluate_ast(condition, context)?)? {
                evaluate_ast(then_expr, context)
            } else {
                evaluate_ast(else_expr, context)
            }
        }
        AstNode::Member { object, property } => {
            let object = evaluate_ast(object, context)?;
            match object {
                CelValue::Map(map) => map
                    .get(property)
                    .cloned()
                    .ok_or_else(|| EvalError::InvalidMemberAccess(property.clone())),
                _ => Err(EvalError::TypeMismatch(
                    "member access requires map/object".to_owned(),
                )),
            }
        }
        AstNode::Index { object, index } => evaluate_index(
            evaluate_ast(object, context)?,
            evaluate_ast(index, context)?,
        ),
        AstNode::Call { callee, args } => {
            let name = resolve_callee_name(callee)?;
            let mut values = Vec::with_capacity(args.len());
            for arg in args {
                values.push(evaluate_ast(arg, context)?);
            }
            evaluate_builtin(name.as_str(), &values)
        }
    }
}

fn evaluate_unary(op: UnaryOp, value: CelValue) -> Result<CelValue, EvalError> {
    match op {
        UnaryOp::Not => Ok(CelValue::Bool(!as_bool(&value)?)),
        UnaryOp::Neg => match value {
            CelValue::Integer(number) => Ok(CelValue::Integer(-number)),
            CelValue::Decimal(number) => Ok(CelValue::Decimal(number.neg())),
            _ => Err(EvalError::TypeMismatch(
                "neg requires numeric value".to_owned(),
            )),
        },
    }
}

fn evaluate_binary(left: CelValue, op: BinaryOp, right: CelValue) -> Result<CelValue, EvalError> {
    match op {
        BinaryOp::And => Ok(CelValue::Bool(as_bool(&left)? && as_bool(&right)?)),
        BinaryOp::Or => Ok(CelValue::Bool(as_bool(&left)? || as_bool(&right)?)),
        BinaryOp::Eq => Ok(CelValue::Bool(left == right)),
        BinaryOp::Ne => Ok(CelValue::Bool(left != right)),
        BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
            evaluate_compare(left, op, right)
        }
        BinaryOp::In => match right {
            CelValue::List(items) => Ok(CelValue::Bool(items.contains(&left))),
            _ => Err(EvalError::TypeMismatch(
                "`in` right side must be list".to_owned(),
            )),
        },
        BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => {
            evaluate_arithmetic(left, op, right)
        }
    }
}

fn evaluate_compare(left: CelValue, op: BinaryOp, right: CelValue) -> Result<CelValue, EvalError> {
    let ordering = match (left, right) {
        (CelValue::Integer(left), CelValue::Integer(right)) => left.cmp(&right),
        (CelValue::Decimal(left), CelValue::Decimal(right)) => left.cmp(&right),
        (CelValue::Integer(left), CelValue::Decimal(right)) => {
            Decimal::from_bigint(left).cmp(&right)
        }
        (CelValue::Decimal(left), CelValue::Integer(right)) => {
            left.cmp(&Decimal::from_bigint(right))
        }
        (CelValue::String(left), CelValue::String(right)) => {
            match (
                numeric_from_string(left.as_str()),
                numeric_from_string(right.as_str()),
            ) {
                (Ok(left), Ok(right)) => match (left, right) {
                    (Numeric::Integer(left), Numeric::Integer(right)) => left.cmp(&right),
                    (Numeric::Integer(left), Numeric::Decimal(right)) => {
                        Decimal::from_bigint(left).cmp(&right)
                    }
                    (Numeric::Decimal(left), Numeric::Integer(right)) => {
                        left.cmp(&Decimal::from_bigint(right))
                    }
                    (Numeric::Decimal(left), Numeric::Decimal(right)) => left.cmp(&right),
                },
                _ => left.cmp(&right),
            }
        }
        (CelValue::String(left), CelValue::Integer(right)) => {
            numeric_from_string(left.as_str())?.cmp_integer(&right)?
        }
        (CelValue::Integer(left), CelValue::String(right)) => {
            numeric_from_string(right.as_str())?.reverse_cmp_integer(&left)?
        }
        (CelValue::String(left), CelValue::Decimal(right)) => {
            numeric_from_string(left.as_str())?.cmp_decimal(&right)?
        }
        (CelValue::Decimal(left), CelValue::String(right)) => {
            numeric_from_string(right.as_str())?.reverse_cmp_decimal(&left)?
        }
        _ => {
            return Err(EvalError::TypeMismatch(
                "comparison requires compatible types".to_owned(),
            ))
        }
    };

    let result = match op {
        BinaryOp::Lt => ordering.is_lt(),
        BinaryOp::Le => ordering.is_le(),
        BinaryOp::Gt => ordering.is_gt(),
        BinaryOp::Ge => ordering.is_ge(),
        _ => unreachable!(),
    };
    Ok(CelValue::Bool(result))
}

fn evaluate_arithmetic(
    left: CelValue,
    op: BinaryOp,
    right: CelValue,
) -> Result<CelValue, EvalError> {
    if op == BinaryOp::Add {
        if let (CelValue::String(left), CelValue::String(right)) = (&left, &right) {
            return Ok(CelValue::String(format!("{left}{right}")));
        }
    }

    let left_num = as_numeric(left)?;
    let right_num = as_numeric(right)?;

    match (left_num, right_num) {
        (Numeric::Integer(left), Numeric::Integer(right)) => match op {
            BinaryOp::Add => Ok(CelValue::Integer(left + right)),
            BinaryOp::Sub => Ok(CelValue::Integer(left - right)),
            BinaryOp::Mul => Ok(CelValue::Integer(left * right)),
            BinaryOp::Div => {
                if right.is_zero() {
                    return Err(EvalError::Numeric(NumericError::DivisionByZero));
                }
                Ok(CelValue::Integer(left / right))
            }
            BinaryOp::Mod => {
                if right.is_zero() {
                    return Err(EvalError::Numeric(NumericError::DivisionByZero));
                }
                Ok(CelValue::Integer(left % right))
            }
            _ => Err(EvalError::TypeMismatch(
                "unsupported integer operation".to_owned(),
            )),
        },
        (left, right) => {
            let left = left.into_decimal();
            let right = right.into_decimal();
            let out = match op {
                BinaryOp::Add => left.add(&right),
                BinaryOp::Sub => left.sub(&right),
                BinaryOp::Mul => left.mul(&right),
                BinaryOp::Div => left.div(&right)?,
                BinaryOp::Mod => {
                    return Err(EvalError::TypeMismatch(
                        "decimal modulo is not supported".to_owned(),
                    ))
                }
                _ => unreachable!(),
            };
            Ok(CelValue::Decimal(out))
        }
    }
}

fn evaluate_index(object: CelValue, index: CelValue) -> Result<CelValue, EvalError> {
    match (object, index) {
        (CelValue::List(items), CelValue::Integer(index)) => {
            if index.sign() == Sign::Minus {
                return Err(EvalError::InvalidIndexAccess);
            }
            let Some(index) = index.to_usize() else {
                return Err(EvalError::InvalidIndexAccess);
            };
            items
                .get(index)
                .cloned()
                .ok_or(EvalError::InvalidIndexAccess)
        }
        (CelValue::Map(map), CelValue::String(key)) => map
            .get(&key)
            .cloned()
            .ok_or(EvalError::InvalidMemberAccess(key)),
        _ => Err(EvalError::TypeMismatch(
            "index access requires list[int] or map[string]".to_owned(),
        )),
    }
}

fn resolve_callee_name(callee: &AstNode) -> Result<String, EvalError> {
    match callee {
        AstNode::Identifier(name) => Ok(name.clone()),
        AstNode::Member { property, .. } => Ok(property.clone()),
        _ => Err(EvalError::UnsupportedCall),
    }
}

fn evaluate_builtin(name: &str, args: &[CelValue]) -> Result<CelValue, EvalError> {
    match name {
        "size" => builtin_size(args),
        "contains" => builtin_contains(args),
        "abs" => builtin_abs(args),
        "min" => builtin_min_max(args, true),
        "max" => builtin_min_max(args, false),
        "mul_div" => builtin_mul_div(args),
        "to_atomic" => builtin_to_atomic(args),
        "to_unit" => builtin_to_unit(args),
        "int" => builtin_int(args),
        "string" => builtin_string(args),
        "bool" => builtin_bool(args),
        "type" => builtin_type(args),
        _ => Err(EvalError::UnsupportedCall),
    }
}

fn builtin_size(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 1, "size")?;
    let size = match &args[0] {
        CelValue::String(value) => value.chars().count(),
        CelValue::List(value) => value.len(),
        CelValue::Map(value) => value.len(),
        _ => {
            return Err(EvalError::TypeMismatch(
                "size expects string/list/map".to_owned(),
            ))
        }
    };
    Ok(CelValue::Integer(BigInt::from(size)))
}

fn builtin_contains(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 2, "contains")?;
    match (&args[0], &args[1]) {
        (CelValue::String(value), CelValue::String(sub)) => Ok(CelValue::Bool(value.contains(sub))),
        (CelValue::List(items), needle) => Ok(CelValue::Bool(items.contains(needle))),
        _ => Err(EvalError::TypeMismatch(
            "contains expects (string,string) or (list,any)".to_owned(),
        )),
    }
}

fn builtin_abs(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 1, "abs")?;
    match &args[0] {
        CelValue::Integer(value) => Ok(CelValue::Integer(value.abs())),
        CelValue::Decimal(value) => Ok(CelValue::Decimal(value.abs())),
        CelValue::String(value) => match numeric_from_string(value.as_str())? {
            Numeric::Integer(value) => Ok(CelValue::Integer(value.abs())),
            Numeric::Decimal(value) => Ok(CelValue::Decimal(value.abs())),
        },
        _ => Err(EvalError::TypeMismatch(
            "abs expects numeric value".to_owned(),
        )),
    }
}

fn builtin_min_max(args: &[CelValue], min: bool) -> Result<CelValue, EvalError> {
    ensure_arity(args, 2, if min { "min" } else { "max" })?;
    let ordering = evaluate_compare(args[0].clone(), BinaryOp::Lt, args[1].clone())?;
    let take_left = matches!(ordering, CelValue::Bool(true));
    match (min, take_left) {
        (true, true) | (false, false) => Ok(args[0].clone()),
        _ => Ok(args[1].clone()),
    }
}

fn builtin_mul_div(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 3, "mul_div")?;
    let left = as_numeric(args[0].clone())?.into_decimal();
    let mul = as_numeric(args[1].clone())?.into_decimal();
    let div = as_numeric(args[2].clone())?.into_decimal();
    Ok(CelValue::Decimal(left.mul(&mul).div(&div)?))
}

fn builtin_to_atomic(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 2, "to_atomic")?;
    let amount = as_numeric(args[0].clone())?.into_decimal();
    let decimals = extract_decimals(&args[1])?;
    Ok(CelValue::Integer(amount.to_atomic_int(decimals)?))
}

fn builtin_to_unit(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 2, "to_unit")?;
    let atomic = as_integer_coerce(&args[0])?;
    let decimals = extract_decimals(&args[1])?;
    Ok(decimal_to_value(Decimal::from_atomic_int(atomic, decimals)))
}

fn builtin_int(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 1, "int")?;
    Ok(CelValue::Integer(as_integer_coerce(&args[0])?))
}

fn builtin_string(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 1, "string")?;
    Ok(CelValue::String(match &args[0] {
        CelValue::Null => "null".to_owned(),
        CelValue::Bool(value) => value.to_string(),
        CelValue::Integer(value) => value.to_string(),
        CelValue::Decimal(value) => value.to_string(),
        CelValue::String(value) => value.clone(),
        CelValue::List(_) => "list".to_owned(),
        CelValue::Map(_) => "map".to_owned(),
    }))
}

fn builtin_bool(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 1, "bool")?;
    match &args[0] {
        CelValue::Bool(value) => Ok(CelValue::Bool(*value)),
        CelValue::String(value) => match value.as_str() {
            "true" => Ok(CelValue::Bool(true)),
            "false" => Ok(CelValue::Bool(false)),
            _ => Err(EvalError::TypeMismatch(
                "bool expects true/false string".to_owned(),
            )),
        },
        _ => Err(EvalError::TypeMismatch(
            "bool expects bool or string".to_owned(),
        )),
    }
}

fn builtin_type(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 1, "type")?;
    let kind = match &args[0] {
        CelValue::Null => "null",
        CelValue::Bool(_) => "bool",
        CelValue::Integer(_) => "int",
        CelValue::Decimal(_) => "decimal",
        CelValue::String(_) => "string",
        CelValue::List(_) => "list",
        CelValue::Map(_) => "map",
    };
    Ok(CelValue::String(kind.to_owned()))
}

fn ensure_arity(args: &[CelValue], expected: usize, name: &str) -> Result<(), EvalError> {
    if args.len() == expected {
        Ok(())
    } else {
        Err(EvalError::TypeMismatch(format!(
            "{name} expects {expected} arguments"
        )))
    }
}

fn as_bool(value: &CelValue) -> Result<bool, EvalError> {
    value
        .as_bool()
        .ok_or_else(|| EvalError::TypeMismatch("expected bool".to_owned()))
}

fn decimal_to_value(value: Decimal) -> CelValue {
    if value.scale() == 0 {
        CelValue::Integer(value.mantissa())
    } else {
        CelValue::Decimal(value)
    }
}

enum Numeric {
    Integer(BigInt),
    Decimal(Decimal),
}

impl Numeric {
    fn into_decimal(self) -> Decimal {
        match self {
            Numeric::Integer(value) => Decimal::from_bigint(value),
            Numeric::Decimal(value) => value,
        }
    }

    fn cmp_integer(&self, right: &BigInt) -> Result<std::cmp::Ordering, EvalError> {
        match self {
            Numeric::Integer(left) => Ok(left.cmp(right)),
            Numeric::Decimal(left) => Ok(left.cmp(&Decimal::from_bigint(right.clone()))),
        }
    }

    fn reverse_cmp_integer(&self, left: &BigInt) -> Result<std::cmp::Ordering, EvalError> {
        self.cmp_integer(left).map(|ordering| ordering.reverse())
    }

    fn cmp_decimal(&self, right: &Decimal) -> Result<std::cmp::Ordering, EvalError> {
        match self {
            Numeric::Integer(left) => Ok(Decimal::from_bigint(left.clone()).cmp(right)),
            Numeric::Decimal(left) => Ok(left.cmp(right)),
        }
    }

    fn reverse_cmp_decimal(&self, left: &Decimal) -> Result<std::cmp::Ordering, EvalError> {
        self.cmp_decimal(left).map(|ordering| ordering.reverse())
    }
}

fn as_numeric(value: CelValue) -> Result<Numeric, EvalError> {
    match value {
        CelValue::Integer(value) => Ok(Numeric::Integer(value)),
        CelValue::Decimal(value) => Ok(Numeric::Decimal(value)),
        CelValue::String(value) => numeric_from_string(value.as_str()),
        _ => Err(EvalError::TypeMismatch("expected numeric value".to_owned())),
    }
}

fn numeric_from_string(value: &str) -> Result<Numeric, EvalError> {
    if let Ok(integer) = value.parse::<BigInt>() {
        return Ok(Numeric::Integer(integer));
    }
    if let Ok(decimal) = Decimal::parse(value) {
        return Ok(Numeric::Decimal(decimal));
    }
    Err(EvalError::TypeMismatch("expected numeric value".to_owned()))
}

fn as_integer_coerce(value: &CelValue) -> Result<BigInt, EvalError> {
    match value {
        CelValue::Integer(value) => Ok(value.clone()),
        CelValue::Decimal(decimal) if decimal.scale() == 0 => Ok(decimal.mantissa()),
        CelValue::Decimal(_) => Err(EvalError::Numeric(NumericError::NonExactDivision)),
        CelValue::String(value) => value
            .parse::<BigInt>()
            .map_err(|_| EvalError::TypeMismatch("string is not integer".to_owned())),
        CelValue::Bool(value) => Ok(if *value {
            BigInt::from(1u8)
        } else {
            BigInt::from(0u8)
        }),
        _ => Err(EvalError::TypeMismatch("cannot coerce to int".to_owned())),
    }
}

fn extract_decimals(value: &CelValue) -> Result<u32, EvalError> {
    let decimals = match value {
        CelValue::Integer(value) => value.clone(),
        CelValue::Map(map) => map
            .get("decimals")
            .ok_or_else(|| EvalError::TypeMismatch("asset map missing decimals".to_owned()))
            .and_then(as_integer_coerce)?,
        _ => {
            return Err(EvalError::TypeMismatch(
                "decimals must be integer or map{decimals}".to_owned(),
            ))
        }
    };
    if decimals.sign() == Sign::Minus {
        return Err(EvalError::TypeMismatch("decimals must be >= 0".to_owned()));
    }
    decimals
        .to_u32()
        .ok_or_else(|| EvalError::TypeMismatch("decimals out of range".to_owned()))
}
