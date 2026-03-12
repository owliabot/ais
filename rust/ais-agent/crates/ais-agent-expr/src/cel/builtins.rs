//! Builtins supported by the reduced local CEL layer.

pub struct CelBuiltins;

impl CelBuiltins {
    pub const SUPPORTED: &'static [&'static str] = &[
        "size", "contains", "abs", "min", "max", "mul_div", "string", "bool", "type",
    ];
}
