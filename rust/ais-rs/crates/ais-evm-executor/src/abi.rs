use crate::utils::lit_or_value;
use alloy_dyn_abi::{DynSolType, DynSolValue, FunctionExt as DynFunctionExt, JsonAbiExt, Specifier};
use alloy_json_abi::{Function, Param};
use alloy_primitives::hex;
use alloy_primitives::{Bytes, U256};
use serde_json::{Map, Value};

pub(crate) fn build_calldata(function: &Function, args: &Map<String, Value>) -> Result<Bytes, String> {
    let mut values = Vec::<DynSolValue>::with_capacity(function.inputs.len());
    for input in &function.inputs {
        let input_name = input.name.as_str();
        if input_name.is_empty() {
            return Err("abi input name must be non-empty for object args".to_string());
        }
        let arg = args
            .get(input_name)
            .ok_or_else(|| format!("missing arg for input `{input_name}`"))?;
        let arg = lit_or_value(arg);
        values.push(param_json_to_dyn_value(input, arg)?);
    }
    function
        .abi_encode_input(&values)
        .map(Bytes::from)
        .map_err(|error| format!("abi encode input failed: {error}"))
}

pub(crate) fn decode_outputs(raw: &Bytes, function: &Function) -> Result<Map<String, Value>, String> {
    if function.outputs.is_empty() {
        return Ok(Map::new());
    }
    let decoded_values = function
        .abi_decode_output(raw.as_ref(), true)
        .map_err(|error| format!("abi decode output failed: {error}"))?;
    if decoded_values.len() != function.outputs.len() {
        return Err(format!(
            "abi decode output length mismatch, expected {}, got {}",
            function.outputs.len(),
            decoded_values.len()
        ));
    }

    let mut decoded = Map::<String, Value>::new();
    for (index, output) in function.outputs.iter().enumerate() {
        let output_name = if output.name.is_empty() {
            format!("out_{index}")
        } else {
            output.name.clone()
        };
        decoded.insert(output_name, dyn_value_to_json(&decoded_values[index]));
    }
    Ok(decoded)
}

pub(crate) fn parse_abi_function(abi: &Map<String, Value>) -> Result<Function, String> {
    let mut normalized = abi.clone();
    normalized
        .entry("inputs".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    normalized
        .entry("outputs".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    serde_json::from_value::<Function>(Value::Object(normalized))
        .map_err(|error| format!("invalid function abi: {error}"))
}

fn param_json_to_dyn_value(param: &Param, value: &Value) -> Result<DynSolValue, String> {
    if let Some(suffix) = param.ty.strip_prefix("tuple") {
        return tuple_json_to_dyn_value(&param.components, suffix, value);
    }
    let ty = param
        .resolve()
        .map_err(|error| format!("resolve abi param type `{}` failed: {error}", param.ty))?;
    dyn_value_from_type(&ty, value)
}

fn tuple_json_to_dyn_value(
    components: &[Param],
    suffix: &str,
    value: &Value,
) -> Result<DynSolValue, String> {
    if suffix.is_empty() {
        return tuple_json_base_to_dyn_value(components, value);
    }
    let dims = parse_array_dims(suffix)?;
    tuple_json_array_to_dyn_value(components, &dims, value)
}

fn tuple_json_array_to_dyn_value(
    components: &[Param],
    dims: &[Option<usize>],
    value: &Value,
) -> Result<DynSolValue, String> {
    if dims.is_empty() {
        return tuple_json_base_to_dyn_value(components, value);
    }
    let items = value
        .as_array()
        .ok_or_else(|| "tuple array arg must be array".to_string())?;
    if let Some(expected) = dims[0] {
        if items.len() != expected {
            return Err(format!(
                "tuple fixed array length mismatch, expected {expected}, got {}",
                items.len()
            ));
        }
    }
    let mut values = Vec::<DynSolValue>::with_capacity(items.len());
    for item in items {
        values.push(tuple_json_array_to_dyn_value(components, &dims[1..], item)?);
    }
    Ok(match dims[0] {
        Some(_) => DynSolValue::FixedArray(values),
        None => DynSolValue::Array(values),
    })
}

fn tuple_json_base_to_dyn_value(components: &[Param], value: &Value) -> Result<DynSolValue, String> {
    if let Some(items) = value.as_array() {
        if items.len() != components.len() {
            return Err(format!(
                "tuple arg length mismatch, expected {}, got {}",
                components.len(),
                items.len()
            ));
        }
        let mut values = Vec::<DynSolValue>::with_capacity(items.len());
        for (component, item) in components.iter().zip(items) {
            values.push(param_json_to_dyn_value(component, lit_or_value(item))?);
        }
        return Ok(DynSolValue::Tuple(values));
    }
    let object = value
        .as_object()
        .ok_or_else(|| "tuple arg must be object or array".to_string())?;
    let mut values = Vec::<DynSolValue>::with_capacity(components.len());
    for component in components {
        if component.name.is_empty() {
            return Err("tuple component name must be non-empty when using object args".to_string());
        }
        let item = object
            .get(component.name.as_str())
            .ok_or_else(|| format!("missing tuple field `{}`", component.name))?;
        values.push(param_json_to_dyn_value(component, lit_or_value(item))?);
    }
    Ok(DynSolValue::Tuple(values))
}

fn parse_array_dims(mut suffix: &str) -> Result<Vec<Option<usize>>, String> {
    let mut dims = Vec::<Option<usize>>::new();
    while !suffix.is_empty() {
        if !suffix.starts_with('[') {
            return Err(format!("invalid tuple array suffix `{suffix}`"));
        }
        let close = suffix
            .find(']')
            .ok_or_else(|| format!("invalid tuple array suffix `{suffix}`"))?;
        let size_raw = &suffix[1..close];
        if size_raw.is_empty() {
            dims.push(None);
        } else {
            let size = size_raw
                .parse::<usize>()
                .map_err(|error| format!("invalid tuple array size `{size_raw}`: {error}"))?;
            dims.push(Some(size));
        }
        suffix = &suffix[close + 1..];
    }
    Ok(dims)
}

fn dyn_value_from_type(ty: &DynSolType, value: &Value) -> Result<DynSolValue, String> {
    #[allow(unreachable_patterns)]
    match ty {
        DynSolType::Bool => value
            .as_bool()
            .map(DynSolValue::Bool)
            .ok_or_else(|| "bool arg must be boolean".to_string()),
        DynSolType::Int(bits) => {
            if let Some(number) = value.as_i64() {
                return DynSolType::Int(*bits)
                    .coerce_str(number.to_string().as_str())
                    .map_err(|error| format!("invalid int arg: {error}"));
            }
            let text = value
                .as_str()
                .ok_or_else(|| "int arg must be integer or string".to_string())?;
            DynSolType::Int(*bits)
                .coerce_str(text)
                .map_err(|error| format!("invalid int arg: {error}"))
        }
        DynSolType::Uint(bits) => {
            if let Some(number) = value.as_u64() {
                return Ok(DynSolValue::Uint(U256::from(number), *bits));
            }
            let text = value
                .as_str()
                .ok_or_else(|| "uint arg must be integer or string".to_string())?;
            DynSolType::Uint(*bits)
                .coerce_str(text)
                .map_err(|error| format!("invalid uint arg: {error}"))
        }
        DynSolType::FixedBytes(size) => {
            let text = value
                .as_str()
                .ok_or_else(|| "fixed bytes arg must be hex string".to_string())?;
            DynSolType::FixedBytes(*size)
                .coerce_str(text)
                .map_err(|error| format!("invalid fixed bytes arg: {error}"))
        }
        DynSolType::Address => {
            let text = value
                .as_str()
                .ok_or_else(|| "address arg must be string".to_string())?;
            DynSolType::Address
                .coerce_str(text)
                .map_err(|error| format!("invalid address arg: {error}"))
        }
        DynSolType::Function => {
            let text = value
                .as_str()
                .ok_or_else(|| "function arg must be string".to_string())?;
            DynSolType::Function
                .coerce_str(text)
                .map_err(|error| format!("invalid function arg: {error}"))
        }
        DynSolType::Bytes => {
            if let Some(text) = value.as_str() {
                return DynSolType::Bytes
                    .coerce_str(text)
                    .map_err(|error| format!("invalid bytes arg: {error}"));
            }
            let items = value
                .as_array()
                .ok_or_else(|| "bytes arg must be hex string or byte array".to_string())?;
            let mut bytes = Vec::<u8>::with_capacity(items.len());
            for item in items {
                let Some(number) = item.as_u64() else {
                    return Err("bytes array item must be integer".to_string());
                };
                if number > u8::MAX as u64 {
                    return Err(format!("bytes array item out of range: {number}"));
                }
                bytes.push(number as u8);
            }
            Ok(DynSolValue::Bytes(bytes))
        }
        DynSolType::String => value
            .as_str()
            .map(|text| DynSolValue::String(text.to_string()))
            .ok_or_else(|| "string arg must be string".to_string()),
        DynSolType::Array(inner) => {
            let items = value
                .as_array()
                .ok_or_else(|| "array arg must be array".to_string())?;
            let mut values = Vec::<DynSolValue>::with_capacity(items.len());
            for item in items {
                values.push(dyn_value_from_type(inner, lit_or_value(item))?);
            }
            Ok(DynSolValue::Array(values))
        }
        DynSolType::FixedArray(inner, size) => {
            let items = value
                .as_array()
                .ok_or_else(|| "fixed array arg must be array".to_string())?;
            if items.len() != *size {
                return Err(format!(
                    "fixed array arg length mismatch, expected {size}, got {}",
                    items.len()
                ));
            }
            let mut values = Vec::<DynSolValue>::with_capacity(items.len());
            for item in items {
                values.push(dyn_value_from_type(inner, lit_or_value(item))?);
            }
            Ok(DynSolValue::FixedArray(values))
        }
        DynSolType::Tuple(types) => {
            let items = value
                .as_array()
                .ok_or_else(|| "tuple arg must be array".to_string())?;
            if items.len() != types.len() {
                return Err(format!(
                    "tuple arg length mismatch, expected {}, got {}",
                    types.len(),
                    items.len()
                ));
            }
            let mut values = Vec::<DynSolValue>::with_capacity(items.len());
            for (item, ty) in items.iter().zip(types) {
                values.push(dyn_value_from_type(ty, lit_or_value(item))?);
            }
            Ok(DynSolValue::Tuple(values))
        }
        _ => Err("unsupported dynamic abi type".to_string()),
    }
}

fn dyn_value_to_json(value: &DynSolValue) -> Value {
    match value {
        DynSolValue::Bool(boolean) => Value::Bool(*boolean),
        DynSolValue::Int(number, _) => Value::String(number.to_string()),
        DynSolValue::Uint(number, _) => Value::String(number.to_string()),
        DynSolValue::FixedBytes(word, size) => Value::String(format!("0x{}", hex::encode(&word[..*size]))),
        DynSolValue::Address(address) => Value::String(format!("{address:#x}")),
        DynSolValue::Function(function) => Value::String(format!("0x{}", hex::encode(function.as_slice()))),
        DynSolValue::Bytes(bytes) => Value::String(format!("0x{}", hex::encode(bytes))),
        DynSolValue::String(text) => Value::String(text.clone()),
        DynSolValue::Array(values) | DynSolValue::FixedArray(values) | DynSolValue::Tuple(values) => {
            Value::Array(values.iter().map(dyn_value_to_json).collect())
        }
        #[allow(unreachable_patterns)]
        _ => Value::Null,
    }
}
