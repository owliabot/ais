use crate::ast::{AstNode, BinaryOp, UnaryOp};
use crate::numeric::{Decimal, NumericError};
use crate::parser::{parse_expression, ParseError};
use num_bigint::{BigInt, Sign};
use num_traits::{Signed, ToPrimitive, Zero};
use regex::Regex;
use std::collections::{BTreeMap, HashMap};

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

pub fn evaluate_expression(expression: &str, context: &CelContext) -> Result<CelValue, EvalError> {
    let ast = parse_expression(expression)?;
    evaluate_ast(&ast, context)
}

pub fn evaluate_ast(ast: &AstNode, context: &CelContext) -> Result<CelValue, EvalError> {
    match ast {
        AstNode::Null => Ok(CelValue::Null),
        AstNode::Bool(value) => Ok(CelValue::Bool(*value)),
        AstNode::Integer(value) => Ok(CelValue::Integer(
            value
                .parse::<BigInt>()
                .map_err(|_| EvalError::TypeMismatch("invalid integer literal".to_string()))?,
        )),
        AstNode::Decimal(raw) => Ok(CelValue::Decimal(Decimal::parse(raw)?)),
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
        AstNode::Unary { op, expr } => {
            let value = evaluate_ast(expr, context)?;
            evaluate_unary(*op, value)
        }
        AstNode::Binary { left, op, right } => {
            let left = evaluate_ast(left, context)?;
            let right = evaluate_ast(right, context)?;
            evaluate_binary(left, *op, right)
        }
        AstNode::Ternary {
            condition,
            then_expr,
            else_expr,
        } => {
            let condition = evaluate_ast(condition, context)?;
            let cond = as_bool(&condition)?;
            if cond {
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
                    "member access requires map/object".to_string(),
                )),
            }
        }
        AstNode::Index { object, index } => {
            let object = evaluate_ast(object, context)?;
            let index = evaluate_ast(index, context)?;
            evaluate_index(object, index)
        }
        AstNode::Call { callee, args } => evaluate_call(callee, args, context),
    }
}

#[derive(Debug, Default)]
pub struct CELEvaluator {
    cache: HashMap<String, AstNode>,
}

impl CELEvaluator {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    pub fn evaluate(&mut self, expression: &str, context: &CelContext) -> Result<CelValue, EvalError> {
        let ast = if let Some(ast) = self.cache.get(expression) {
            ast.clone()
        } else {
            let parsed = parse_expression(expression)?;
            self.cache.insert(expression.to_string(), parsed.clone());
            parsed
        };

        evaluate_ast(&ast, context)
    }

    pub fn cached_expressions(&self) -> usize {
        self.cache.len()
    }
}

fn evaluate_unary(op: UnaryOp, value: CelValue) -> Result<CelValue, EvalError> {
    match op {
        UnaryOp::Not => Ok(CelValue::Bool(!as_bool(&value)?)),
        UnaryOp::Neg => match value {
            CelValue::Integer(number) => Ok(CelValue::Integer(-number)),
            CelValue::Decimal(decimal) => Ok(CelValue::Decimal(decimal.neg())),
            _ => Err(EvalError::TypeMismatch("neg requires numeric value".to_string())),
        },
    }
}

fn evaluate_binary(left: CelValue, op: BinaryOp, right: CelValue) -> Result<CelValue, EvalError> {
    match op {
        BinaryOp::And => Ok(CelValue::Bool(as_bool(&left)? && as_bool(&right)?)),
        BinaryOp::Or => Ok(CelValue::Bool(as_bool(&left)? || as_bool(&right)?)),
        BinaryOp::Eq => Ok(CelValue::Bool(left == right)),
        BinaryOp::Ne => Ok(CelValue::Bool(left != right)),
        BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => evaluate_compare(left, op, right),
        BinaryOp::In => match right {
            CelValue::List(items) => Ok(CelValue::Bool(items.contains(&left))),
            _ => Err(EvalError::TypeMismatch("`in` right side must be list".to_string())),
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
            decimal_from_bigint(&left)?.cmp(&right)
        }
        (CelValue::Decimal(left), CelValue::Integer(right)) => {
            left.cmp(&decimal_from_bigint(&right)?)
        }
        (CelValue::String(left), CelValue::Integer(right)) => {
            match parse_numeric_string(left.as_str())? {
                Numeric::Integer(left) => left.cmp(&right),
                Numeric::Decimal(left) => left.cmp(&decimal_from_bigint(&right)?),
            }
        }
        (CelValue::Integer(left), CelValue::String(right)) => {
            match parse_numeric_string(right.as_str())? {
                Numeric::Integer(right) => left.cmp(&right),
                Numeric::Decimal(right) => decimal_from_bigint(&left)?.cmp(&right),
            }
        }
        (CelValue::String(left), CelValue::Decimal(right)) => {
            match parse_numeric_string(left.as_str())? {
                Numeric::Integer(left) => decimal_from_bigint(&left)?.cmp(&right),
                Numeric::Decimal(left) => left.cmp(&right),
            }
        }
        (CelValue::Decimal(left), CelValue::String(right)) => {
            match parse_numeric_string(right.as_str())? {
                Numeric::Integer(right) => left.cmp(&decimal_from_bigint(&right)?),
                Numeric::Decimal(right) => left.cmp(&right),
            }
        }
        (CelValue::String(left), CelValue::String(right)) => {
            match (
                parse_numeric_string(left.as_str()),
                parse_numeric_string(right.as_str()),
            ) {
                (Ok(Numeric::Integer(left)), Ok(Numeric::Integer(right))) => left.cmp(&right),
                (Ok(Numeric::Integer(left)), Ok(Numeric::Decimal(right))) => {
                    decimal_from_bigint(&left)?.cmp(&right)
                }
                (Ok(Numeric::Decimal(left)), Ok(Numeric::Integer(right))) => {
                    left.cmp(&decimal_from_bigint(&right)?)
                }
                (Ok(Numeric::Decimal(left)), Ok(Numeric::Decimal(right))) => left.cmp(&right),
                _ => left.cmp(&right),
            }
        }
        _ => {
            return Err(EvalError::TypeMismatch(
                "comparison requires compatible types".to_string(),
            ))
        }
    };

    let result = match op {
        BinaryOp::Lt => ordering.is_lt(),
        BinaryOp::Le => ordering.is_le(),
        BinaryOp::Gt => ordering.is_gt(),
        BinaryOp::Ge => ordering.is_ge(),
        _ => unreachable!("called only for compare ops"),
    };

    Ok(CelValue::Bool(result))
}

fn evaluate_arithmetic(left: CelValue, op: BinaryOp, right: CelValue) -> Result<CelValue, EvalError> {
    if op == BinaryOp::Add {
        if let (CelValue::String(left), CelValue::String(right)) = (&left, &right) {
            return Ok(CelValue::String(format!("{left}{right}")));
        }
    }

    let left_num = as_numeric(left)?;
    let right_num = as_numeric(right)?;

    match (left_num, right_num) {
        (Numeric::Integer(left), Numeric::Integer(right)) => {
            evaluate_integer_arithmetic(left.clone(), op, right.clone()).or_else(|_| {
                evaluate_decimal_arithmetic(decimal_from_bigint(&left)?, op, decimal_from_bigint(&right)?)
            })
        }
        (Numeric::Integer(left), Numeric::Decimal(right)) => {
            evaluate_decimal_arithmetic(decimal_from_bigint(&left)?, op, right)
        }
        (Numeric::Decimal(left), Numeric::Integer(right)) => {
            evaluate_decimal_arithmetic(left, op, decimal_from_bigint(&right)?)
        }
        (Numeric::Decimal(left), Numeric::Decimal(right)) => evaluate_decimal_arithmetic(left, op, right),
    }
}

fn evaluate_integer_arithmetic(left: BigInt, op: BinaryOp, right: BigInt) -> Result<CelValue, EvalError> {
    match op {
        BinaryOp::Add => Ok(CelValue::Integer(left + right)),
        BinaryOp::Sub => Ok(CelValue::Integer(left - right)),
        BinaryOp::Mul => Ok(CelValue::Integer(left * right)),
        BinaryOp::Div => {
            if right.is_zero() {
                return Err(EvalError::Numeric(NumericError::DivisionByZero));
            }
            if (&left % &right).is_zero() {
                Ok(CelValue::Integer(left / right))
            } else {
                Err(EvalError::Numeric(NumericError::NonExactDivision))
            }
        }
        BinaryOp::Mod => {
            if right.is_zero() {
                return Err(EvalError::Numeric(NumericError::DivisionByZero));
            }
            Ok(CelValue::Integer(left % right))
        }
        _ => Err(EvalError::TypeMismatch("unsupported integer operation".to_string())),
    }
}

fn evaluate_decimal_arithmetic(left: Decimal, op: BinaryOp, right: Decimal) -> Result<CelValue, EvalError> {
    let out = match op {
        BinaryOp::Add => left.add(&right)?,
        BinaryOp::Sub => left.sub(&right)?,
        BinaryOp::Mul => left.mul(&right)?,
        BinaryOp::Div => left.div_exact(&right)?,
        BinaryOp::Mod => return Err(EvalError::Numeric(NumericError::UnsupportedDecimalOperation)),
        _ => {
            return Err(EvalError::TypeMismatch(
                "unsupported decimal operation".to_string(),
            ))
        }
    };

    if out.scale() == 0 {
        Ok(CelValue::Integer(out.mantissa().clone()))
    } else {
        Ok(CelValue::Decimal(out))
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
            "index access requires list[int] or map[string]".to_string(),
        )),
    }
}

fn as_bool(value: &CelValue) -> Result<bool, EvalError> {
    match value {
        CelValue::Bool(value) => Ok(*value),
        _ => Err(EvalError::TypeMismatch("expected bool".to_string())),
    }
}

#[derive(Debug, Clone)]
enum Numeric {
    Integer(BigInt),
    Decimal(Decimal),
}

fn as_numeric(value: CelValue) -> Result<Numeric, EvalError> {
    match value {
        CelValue::Integer(value) => Ok(Numeric::Integer(value)),
        CelValue::Decimal(value) => Ok(Numeric::Decimal(value)),
        CelValue::String(value) => parse_numeric_string(value.as_str()),
        _ => Err(EvalError::TypeMismatch("expected numeric value".to_string())),
    }
}

fn parse_numeric_string(value: &str) -> Result<Numeric, EvalError> {
    if let Ok(integer) = value.parse::<BigInt>() {
        return Ok(Numeric::Integer(integer));
    }
    if let Ok(decimal) = Decimal::parse(value) {
        return Ok(Numeric::Decimal(decimal));
    }
    Err(EvalError::TypeMismatch(
        "expected numeric value".to_string(),
    ))
}

fn evaluate_call(callee: &AstNode, args: &[AstNode], context: &CelContext) -> Result<CelValue, EvalError> {
    let name = resolve_callee_name(callee)?;
    let mut values = Vec::with_capacity(args.len());
    for arg in args {
        values.push(evaluate_ast(arg, context)?);
    }
    evaluate_builtin(name.as_str(), &values)
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
        "startsWith" => builtin_starts_with(args),
        "endsWith" => builtin_ends_with(args),
        "matches" => builtin_matches(args),
        "lower" => builtin_lower(args),
        "upper" => builtin_upper(args),
        "trim" => builtin_trim(args),
        "abs" => builtin_abs(args),
        "min" => builtin_min(args),
        "max" => builtin_max(args),
        "ceil" => builtin_ceil(args),
        "floor" => builtin_floor(args),
        "round" => builtin_round(args),
        "mul_div" => builtin_mul_div(args),
        "int" => builtin_int(args),
        "uint" => builtin_uint(args),
        "double" => builtin_double(args),
        "string" => builtin_string(args),
        "bool" => builtin_bool(args),
        "type" => builtin_type(args),
        "exists" => builtin_exists(args),
        "all" => builtin_all(args),
        "to_atomic" => builtin_to_atomic(args),
        "to_human" => builtin_to_human(args),
        _ => Err(EvalError::UnsupportedCall),
    }
}

fn builtin_size(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 1, "size")?;
    let size = match &args[0] {
        CelValue::String(value) => value.chars().count(),
        CelValue::List(value) => value.len(),
        CelValue::Map(value) => value.len(),
        _ => return Err(EvalError::TypeMismatch("size expects string/list/map".to_string())),
    };
    Ok(CelValue::Integer(BigInt::from(size)))
}

fn builtin_contains(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 2, "contains")?;
    match (&args[0], &args[1]) {
        (CelValue::String(value), CelValue::String(sub)) => Ok(CelValue::Bool(value.contains(sub))),
        (CelValue::List(items), needle) => Ok(CelValue::Bool(items.contains(needle))),
        _ => Err(EvalError::TypeMismatch("contains expects (string,string) or (list,any)".to_string())),
    }
}

fn builtin_starts_with(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 2, "startsWith")?;
    let value = as_string_value(&args[0])?;
    let prefix = as_string_value(&args[1])?;
    Ok(CelValue::Bool(value.starts_with(prefix.as_str())))
}

fn builtin_ends_with(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 2, "endsWith")?;
    let value = as_string_value(&args[0])?;
    let suffix = as_string_value(&args[1])?;
    Ok(CelValue::Bool(value.ends_with(suffix.as_str())))
}

fn builtin_matches(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 2, "matches")?;
    let value = as_string_value(&args[0])?;
    let pattern = as_string_value(&args[1])?;
    let regex = Regex::new(pattern.as_str()).map_err(|err| EvalError::TypeMismatch(format!("invalid regex: {err}")))?;
    Ok(CelValue::Bool(regex.is_match(value.as_str())))
}

fn builtin_lower(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 1, "lower")?;
    Ok(CelValue::String(as_string_value(&args[0])?.to_lowercase()))
}

fn builtin_upper(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 1, "upper")?;
    Ok(CelValue::String(as_string_value(&args[0])?.to_uppercase()))
}

fn builtin_trim(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 1, "trim")?;
    Ok(CelValue::String(as_string_value(&args[0])?.trim().to_string()))
}

fn builtin_abs(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 1, "abs")?;
    match &args[0] {
        CelValue::Integer(value) => Ok(CelValue::Integer(value.abs())),
        CelValue::Decimal(value) => Ok(CelValue::Decimal(value.abs())),
        _ => Err(EvalError::TypeMismatch("abs expects numeric value".to_string())),
    }
}

fn builtin_min(args: &[CelValue]) -> Result<CelValue, EvalError> {
    if args.is_empty() {
        return Err(EvalError::TypeMismatch("min expects at least one argument".to_string()));
    }
    let mut current = as_decimal_from_any(&args[0])?;
    for arg in &args[1..] {
        let value = as_decimal_from_any(arg)?;
        if value < current {
            current = value;
        }
    }
    Ok(decimal_to_value(current))
}

fn builtin_max(args: &[CelValue]) -> Result<CelValue, EvalError> {
    if args.is_empty() {
        return Err(EvalError::TypeMismatch("max expects at least one argument".to_string()));
    }
    let mut current = as_decimal_from_any(&args[0])?;
    for arg in &args[1..] {
        let value = as_decimal_from_any(arg)?;
        if value > current {
            current = value;
        }
    }
    Ok(decimal_to_value(current))
}

fn builtin_ceil(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 1, "ceil")?;
    let decimal = as_decimal_from_any(&args[0])?;
    let value = decimal_ceil_to_int(&decimal)?;
    Ok(CelValue::Integer(value))
}

fn builtin_floor(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 1, "floor")?;
    let decimal = as_decimal_from_any(&args[0])?;
    let value = decimal_floor_to_int(&decimal)?;
    Ok(CelValue::Integer(value))
}

fn builtin_round(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 1, "round")?;
    let decimal = as_decimal_from_any(&args[0])?;
    let value = decimal_round_to_int(&decimal)?;
    Ok(CelValue::Integer(value))
}

fn builtin_mul_div(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 3, "mul_div")?;
    let a = as_integer_strict(&args[0], "mul_div arg0 must be integer")?;
    let b = as_integer_strict(&args[1], "mul_div arg1 must be integer")?;
    let denom = as_integer_strict(&args[2], "mul_div arg2 must be integer")?;
    if denom.is_zero() {
        return Err(EvalError::Numeric(NumericError::DivisionByZero));
    }
    Ok(CelValue::Integer((a * b) / denom))
}

fn builtin_int(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 1, "int")?;
    let value = as_integer_coerce(&args[0])?;
    Ok(CelValue::Integer(value))
}

fn builtin_uint(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 1, "uint")?;
    let value = as_integer_coerce(&args[0])?;
    Ok(CelValue::Integer(value.abs()))
}

fn builtin_double(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 1, "double")?;
    let decimal = as_decimal_from_any(&args[0])?;
    Ok(CelValue::Decimal(decimal))
}

fn builtin_string(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 1, "string")?;
    Ok(CelValue::String(value_to_string(&args[0])?))
}

fn builtin_bool(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 1, "bool")?;
    Ok(CelValue::Bool(value_truthy(&args[0])?))
}

fn builtin_type(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 1, "type")?;
    let kind = match args[0] {
        CelValue::Null => "null",
        CelValue::Bool(_) => "bool",
        CelValue::Integer(_) => "int",
        CelValue::Decimal(_) => "decimal",
        CelValue::String(_) => "string",
        CelValue::List(_) => "list",
        CelValue::Map(_) => "map",
    };
    Ok(CelValue::String(kind.to_string()))
}

fn builtin_exists(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 1, "exists")?;
    let CelValue::List(items) = &args[0] else {
        return Err(EvalError::TypeMismatch("exists expects list".to_string()));
    };
    for item in items {
        if value_truthy(item)? {
            return Ok(CelValue::Bool(true));
        }
    }
    Ok(CelValue::Bool(false))
}

fn builtin_all(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 1, "all")?;
    let CelValue::List(items) = &args[0] else {
        return Err(EvalError::TypeMismatch("all expects list".to_string()));
    };
    for item in items {
        if !value_truthy(item)? {
            return Ok(CelValue::Bool(false));
        }
    }
    Ok(CelValue::Bool(true))
}

fn builtin_to_atomic(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 2, "to_atomic")?;
    let amount = as_decimal_from_any(&args[0])?;
    let decimals = extract_decimals(&args[1])?;
    let out = amount.to_atomic_int(decimals).map_err(EvalError::Numeric)?;
    Ok(CelValue::Integer(out))
}

fn builtin_to_human(args: &[CelValue]) -> Result<CelValue, EvalError> {
    ensure_arity(args, 2, "to_human")?;
    let atomic = as_integer_coerce(&args[0])?;
    let decimals = extract_decimals(&args[1])?;
    let decimal = Decimal::from_atomic_int(atomic, decimals).to_bigdecimal().normalized();
    Ok(CelValue::String(decimal.to_string()))
}

fn ensure_arity(args: &[CelValue], expected: usize, name: &str) -> Result<(), EvalError> {
    if args.len() != expected {
        return Err(EvalError::TypeMismatch(format!("{name} expects {expected} args, got {}", args.len())));
    }
    Ok(())
}

fn as_string_value(value: &CelValue) -> Result<String, EvalError> {
    match value {
        CelValue::String(value) => Ok(value.clone()),
        _ => Err(EvalError::TypeMismatch("expected string".to_string())),
    }
}

fn as_decimal_from_any(value: &CelValue) -> Result<Decimal, EvalError> {
    match value {
        CelValue::Integer(value) => decimal_from_bigint(value),
        CelValue::Decimal(value) => Ok(value.clone()),
        CelValue::String(value) => Ok(Decimal::parse(value)?),
        _ => Err(EvalError::TypeMismatch("expected numeric/string".to_string())),
    }
}

fn as_integer_strict(value: &CelValue, message: &str) -> Result<BigInt, EvalError> {
    match value {
        CelValue::Integer(value) => Ok(value.clone()),
        CelValue::Decimal(decimal) if decimal.scale() == 0 => Ok(decimal.mantissa().clone()),
        _ => Err(EvalError::TypeMismatch(message.to_string())),
    }
}

fn as_integer_coerce(value: &CelValue) -> Result<BigInt, EvalError> {
    match value {
        CelValue::Integer(value) => Ok(value.clone()),
        CelValue::Decimal(decimal) if decimal.scale() == 0 => Ok(decimal.mantissa().clone()),
        CelValue::Decimal(_) => Err(EvalError::Numeric(NumericError::NonExactDivision)),
        CelValue::String(value) => value
            .parse::<BigInt>()
            .map_err(|_| EvalError::TypeMismatch("string is not integer".to_string())),
        CelValue::Bool(value) => Ok(if *value { BigInt::from(1u8) } else { BigInt::from(0u8) }),
        _ => Err(EvalError::TypeMismatch("cannot coerce to int".to_string())),
    }
}

fn value_to_string(value: &CelValue) -> Result<String, EvalError> {
    match value {
        CelValue::Null => Ok("null".to_string()),
        CelValue::Bool(value) => Ok(value.to_string()),
        CelValue::Integer(value) => Ok(value.to_string()),
        CelValue::Decimal(value) => Ok(value.to_string()),
        CelValue::String(value) => Ok(value.clone()),
        CelValue::List(_) | CelValue::Map(_) => Err(EvalError::TypeMismatch(
            "string conversion for list/map is not supported".to_string(),
        )),
    }
}

fn value_truthy(value: &CelValue) -> Result<bool, EvalError> {
    match value {
        CelValue::Null => Ok(false),
        CelValue::Bool(value) => Ok(*value),
        CelValue::Integer(value) => Ok(!value.is_zero()),
        CelValue::Decimal(value) => Ok(!value.is_zero()),
        CelValue::String(value) => Ok(!value.is_empty()),
        CelValue::List(items) => Ok(!items.is_empty()),
        CelValue::Map(map) => Ok(!map.is_empty()),
    }
}

fn decimal_to_value(value: Decimal) -> CelValue {
    if value.scale() == 0 {
        CelValue::Integer(value.mantissa().clone())
    } else {
        CelValue::Decimal(value)
    }
}

fn decimal_floor_to_int(value: &Decimal) -> Result<BigInt, EvalError> {
    if value.scale() == 0 {
        return Ok(value.mantissa().clone());
    }
    let divisor = pow10(value.scale() as usize)?;
    let mantissa = value.mantissa().clone();
    let quotient = &mantissa / &divisor;
    let remainder = &mantissa % &divisor;
    if !remainder.is_zero() && value.is_negative() {
        Ok(quotient - 1u8)
    } else {
        Ok(quotient)
    }
}

fn decimal_ceil_to_int(value: &Decimal) -> Result<BigInt, EvalError> {
    if value.scale() == 0 {
        return Ok(value.mantissa().clone());
    }
    let divisor = pow10(value.scale() as usize)?;
    let mantissa = value.mantissa().clone();
    let quotient = &mantissa / &divisor;
    let remainder = &mantissa % &divisor;
    if !remainder.is_zero() && !value.is_negative() {
        Ok(quotient + 1u8)
    } else {
        Ok(quotient)
    }
}

fn decimal_round_to_int(value: &Decimal) -> Result<BigInt, EvalError> {
    if value.scale() == 0 {
        return Ok(value.mantissa().clone());
    }
    let divisor = pow10(value.scale() as usize)?;
    let mantissa = value.mantissa().clone();
    let quotient = &mantissa / &divisor;
    let remainder = &mantissa % &divisor;
    let abs_remainder = remainder.abs();
    if (abs_remainder * 2u8) >= divisor {
        if !value.is_negative() {
            Ok(quotient + 1u8)
        } else {
            Ok(quotient - 1u8)
        }
    } else {
        Ok(quotient)
    }
}

fn pow10(power: usize) -> Result<BigInt, EvalError> {
    let mut out = BigInt::from(1u8);
    for _ in 0..power {
        out *= 10u8;
    }
    Ok(out)
}

fn extract_decimals(value: &CelValue) -> Result<u32, EvalError> {
    let decimals = match value {
        CelValue::Integer(value) => value.clone(),
        CelValue::Map(map) => map
            .get("decimals")
            .ok_or_else(|| EvalError::TypeMismatch("asset map missing decimals".to_string()))
            .and_then(as_integer_coerce)?,
        _ => {
            return Err(EvalError::TypeMismatch(
                "decimals must be integer or map{decimals}".to_string(),
            ))
        }
    };
    if decimals.sign() == Sign::Minus {
        return Err(EvalError::TypeMismatch("decimals must be >= 0".to_string()));
    }
    decimals
        .to_u32()
        .ok_or_else(|| EvalError::TypeMismatch("decimals out of range".to_string()))
}

fn decimal_from_bigint(value: &BigInt) -> Result<Decimal, EvalError> {
    Decimal::try_from(value).map_err(EvalError::Numeric)
}

#[cfg(test)]
#[path = "evaluator_test.rs"]
mod tests;
