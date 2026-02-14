use alloy_primitives::{Address, U256};
use serde_json::{Map, Value};
use std::str::FromStr;

pub(crate) fn value_or_lit_as_str<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a str, String> {
    let value = object
        .get(key)
        .ok_or_else(|| format!("missing field `{key}`"))?;
    if let Some(text) = value.as_str() {
        return Ok(text);
    }
    value
        .as_object()
        .and_then(|obj| obj.get("lit"))
        .and_then(Value::as_str)
        .ok_or_else(|| format!("field `{key}` must be string or {{lit: string}}"))
}

pub(crate) fn parse_address(text: &str) -> Result<Address, String> {
    Address::from_str(text).map_err(|error| error.to_string())
}

pub(crate) fn parse_u256(value: &Value) -> Result<U256, String> {
    if let Some(text) = value.as_str() {
        return U256::from_str(text).map_err(|error| error.to_string());
    }
    if let Some(number) = value.as_u64() {
        return Ok(U256::from(number));
    }
    Err("uint arg must be string or integer number".to_string())
}

pub(crate) fn optional_u64_field(execution: &Map<String, Value>, key: &str) -> Result<Option<u64>, String> {
    execution
        .get(key)
        .map(lit_or_value)
        .map(parse_u64)
        .transpose()
}

pub(crate) fn parse_u64(value: &Value) -> Result<u64, String> {
    if let Some(number) = value.as_u64() {
        return Ok(number);
    }
    if let Some(text) = value.as_str() {
        return text.parse::<u64>().map_err(|error| error.to_string());
    }
    Err("value must be u64 number or numeric string".to_string())
}

pub(crate) fn optional_u128_field(execution: &Map<String, Value>, key: &str) -> Result<Option<u128>, String> {
    execution
        .get(key)
        .map(lit_or_value)
        .map(parse_u128)
        .transpose()
}

pub(crate) fn parse_u128(value: &Value) -> Result<u128, String> {
    if let Some(number) = value.as_u64() {
        return Ok(number as u128);
    }
    if let Some(text) = value.as_str() {
        return text.parse::<u128>().map_err(|error| error.to_string());
    }
    Err("value must be u128 number or numeric string".to_string())
}

pub(crate) fn parse_u256_quantity(input: &str) -> Result<U256, String> {
    if input == "0x" {
        return Ok(U256::ZERO);
    }
    if let Some(hex) = input.strip_prefix("0x") {
        if hex.is_empty() {
            return Ok(U256::ZERO);
        }
        return U256::from_str_radix(hex, 16).map_err(|error| error.to_string());
    }
    U256::from_str(input).map_err(|error| error.to_string())
}

pub(crate) fn lit_or_value(value: &Value) -> &Value {
    value
        .as_object()
        .and_then(|object| object.get("lit"))
        .unwrap_or(value)
}

pub(crate) fn normalize_evm_rpc_params(method: &str, params: Value) -> Result<Value, String> {
    if params.is_array() {
        return Ok(params);
    }
    if params.is_null() {
        return Ok(Value::Array(Vec::new()));
    }
    let Some(object) = params.as_object() else {
        return Ok(params);
    };

    match method {
        "eth_blockNumber" => Ok(Value::Array(Vec::new())),
        "eth_getBalance" => {
            if let Some(array) = object.get("array").and_then(Value::as_array) {
                if array.is_empty() {
                    return Err("evm_rpc eth_getBalance params.array must contain address".to_string());
                }
                let address = value_to_string(&array[0]).ok_or_else(|| {
                    "evm_rpc eth_getBalance params.array[0] must be string".to_string()
                })?;
                let block = array
                    .get(1)
                    .cloned()
                    .unwrap_or_else(|| Value::String("latest".to_string()));
                return Ok(Value::Array(vec![Value::String(address), block]));
            }
            let address = object
                .get("address")
                .or_else(|| object.get("account"))
                .or_else(|| object.get("addr"))
                .and_then(value_to_string)
                .ok_or_else(|| "evm_rpc eth_getBalance params.address must be string".to_string())?;
            let block = object
                .get("block")
                .or_else(|| object.get("block_tag"))
                .or_else(|| object.get("blockTag"))
                .map(lit_or_value)
                .cloned()
                .unwrap_or_else(|| Value::String("latest".to_string()));
            Ok(Value::Array(vec![Value::String(address.to_string()), block]))
        }
        "eth_getTransactionReceipt" => {
            let tx_hash = object
                .get("tx_hash")
                .or_else(|| object.get("hash"))
                .or_else(|| object.get("txHash"))
                .and_then(value_to_string)
                .ok_or_else(|| {
                    "evm_rpc eth_getTransactionReceipt params.tx_hash must be string".to_string()
                })?;
            Ok(Value::Array(vec![Value::String(tx_hash.to_string())]))
        }
        "eth_getLogs" | "eth_simulateV1" => Ok(Value::Array(vec![Value::Object(object.clone())])),
        "eth_call" => {
            let tx = object
                .get("tx")
                .or_else(|| object.get("call"))
                .or_else(|| object.get("request"))
                .cloned()
                .unwrap_or_else(|| Value::Object(object.clone()));
            let block = object
                .get("block")
                .or_else(|| object.get("block_tag"))
                .or_else(|| object.get("blockTag"))
                .cloned()
                .unwrap_or_else(|| Value::String("latest".to_string()));
            Ok(Value::Array(vec![tx, block]))
        }
        _ => Ok(params),
    }
}

fn value_to_string(value: &Value) -> Option<String> {
    let value = lit_or_value(value);
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    value
        .as_object()
        .and_then(|object| object.get("value"))
        .and_then(Value::as_str)
        .map(|text| text.to_string())
}
