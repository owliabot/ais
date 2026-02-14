use bigdecimal::BigDecimal;
use num_bigint::{BigInt, Sign};
use num_traits::{Num, Signed, ToPrimitive, Zero};
use std::cmp::Ordering;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decimal {
    mantissa: BigInt,
    scale: u32,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NumericError {
    #[error("invalid decimal literal: {0}")]
    InvalidDecimalLiteral(String),
    #[error("numeric overflow")]
    Overflow,
    #[error("division by zero")]
    DivisionByZero,
    #[error("non exact division")]
    NonExactDivision,
    #[error("operation not supported for decimal")]
    UnsupportedDecimalOperation,
}

impl Decimal {
    pub fn new(int: i128, scale: u32) -> Self {
        Self::from_bigint_with_scale(BigInt::from(int), scale)
    }

    pub fn from_int(value: i128) -> Self {
        Self::from_bigint_with_scale(BigInt::from(value), 0)
    }

    pub fn parse(input: &str) -> Result<Self, NumericError> {
        let parsed = BigDecimal::from_str(input)
            .map_err(|_| NumericError::InvalidDecimalLiteral(input.to_string()))?;
        Self::from_bigdecimal(parsed)
            .map_err(|_| NumericError::InvalidDecimalLiteral(input.to_string()))
    }

    pub fn add(&self, other: &Decimal) -> Result<Decimal, NumericError> {
        let scale = self.scale.max(other.scale);
        let left = self
            .mantissa
            .clone()
            * pow10((scale - self.scale) as usize);
        let right = other
            .mantissa
            .clone()
            * pow10((scale - other.scale) as usize);
        Ok(Self::normalize(left + right, scale))
    }

    pub fn sub(&self, other: &Decimal) -> Result<Decimal, NumericError> {
        let scale = self.scale.max(other.scale);
        let left = self
            .mantissa
            .clone()
            * pow10((scale - self.scale) as usize);
        let right = other
            .mantissa
            .clone()
            * pow10((scale - other.scale) as usize);
        Ok(Self::normalize(left - right, scale))
    }

    pub fn mul(&self, other: &Decimal) -> Result<Decimal, NumericError> {
        Ok(Self::normalize(
            &self.mantissa * &other.mantissa,
            self.scale + other.scale,
        ))
    }

    pub fn div_exact(&self, other: &Decimal) -> Result<Decimal, NumericError> {
        if other.mantissa.is_zero() {
            return Err(NumericError::DivisionByZero);
        }

        let numerator = &self.mantissa * pow10(other.scale as usize);
        let denominator = &other.mantissa * pow10(self.scale as usize);

        let gcd = bigint_gcd(numerator.clone().abs(), denominator.clone().abs());
        let reduced_num = numerator / &gcd;
        let mut reduced_den = denominator / gcd;
        let negative = reduced_den.sign() == Sign::Minus;
        if negative {
            reduced_den = -reduced_den;
        }

        let (den_without_2, twos) = factor_out(reduced_den, 2u8);
        let (den_without_5, fives) = factor_out(den_without_2, 5u8);
        if den_without_5 != BigInt::from(1u8) {
            return Err(NumericError::NonExactDivision);
        }

        let out_scale = twos.max(fives);
        let mut out_mantissa = reduced_num;
        if twos < out_scale {
            out_mantissa *= pow_small(2u8, out_scale - twos);
        }
        if fives < out_scale {
            out_mantissa *= pow_small(5u8, out_scale - fives);
        }
        Ok(Self::normalize(out_mantissa, out_scale))
    }

    pub fn scale(&self) -> u32 {
        self.scale
    }

    pub fn mantissa(&self) -> &BigInt {
        &self.mantissa
    }

    pub fn is_zero(&self) -> bool {
        self.mantissa.is_zero()
    }

    pub fn is_negative(&self) -> bool {
        self.mantissa.sign() == Sign::Minus
    }

    pub fn abs(&self) -> Self {
        Self {
            mantissa: self.mantissa.clone().abs(),
            scale: self.scale,
        }
    }

    pub fn neg(&self) -> Self {
        Self {
            mantissa: -self.mantissa.clone(),
            scale: self.scale,
        }
    }

    pub fn to_i128_exact(&self) -> Option<i128> {
        if self.scale == 0 {
            return self.mantissa.to_i128();
        }
        None
    }

    pub fn from_str_radix(input: &str, radix: u32) -> Result<Self, NumericError> {
        let parsed = BigInt::from_str_radix(input, radix)
            .map_err(|_| NumericError::InvalidDecimalLiteral(input.to_string()))?;
        Ok(Self::from_bigint_with_scale(parsed, 0))
    }

    pub fn from_scientific(input: &str) -> Result<Self, NumericError> {
        Self::parse(input)
    }

    pub fn from_i128_with_scale(value: i128, scale: u32) -> Self {
        Self::from_bigint_with_scale(BigInt::from(value), scale)
    }

    pub fn try_from_i128_with_scale(value: i128, scale: u32) -> Result<Self, NumericError> {
        Ok(Self::from_i128_with_scale(value, scale))
    }

    pub fn from_i32(value: i32) -> Option<Self> {
        Some(Self::from_bigint_with_scale(BigInt::from(value), 0))
    }

    pub fn from_bigint_with_scale(value: BigInt, scale: u32) -> Self {
        Self::normalize(value, scale)
    }

    pub fn from_bigdecimal(value: BigDecimal) -> Result<Self, NumericError> {
        let (mantissa, exponent) = value.normalized().into_bigint_and_exponent();
        if exponent >= 0 {
            return Ok(Self::normalize(mantissa, exponent as u32));
        }
        let factor = pow10((-exponent) as usize);
        Ok(Self::normalize(mantissa * factor, 0))
    }

    pub fn to_bigdecimal(&self) -> BigDecimal {
        BigDecimal::new(self.mantissa.clone(), self.scale as i64)
    }

    pub fn from_atomic_int(value: BigInt, decimals: u32) -> Self {
        Self::from_bigint_with_scale(value, decimals)
    }

    pub fn to_atomic_int(&self, decimals: u32) -> Result<BigInt, NumericError> {
        if decimals >= self.scale {
            let factor = pow10((decimals - self.scale) as usize);
            return Ok(self.mantissa.clone() * factor);
        }

        let divisor = pow10((self.scale - decimals) as usize);
        if (&self.mantissa % &divisor) != BigInt::zero() {
            return Err(NumericError::NonExactDivision);
        }
        Ok(&self.mantissa / divisor)
    }

    fn normalize(mantissa: BigInt, scale: u32) -> Self {
        if mantissa.is_zero() {
            return Self {
                mantissa: BigInt::zero(),
                scale: 0,
            };
        }
        let mut normalized_mantissa = mantissa;
        let mut normalized_scale = scale;
        while normalized_scale > 0 && (&normalized_mantissa % 10u8).is_zero() {
            normalized_mantissa /= 10u8;
            normalized_scale -= 1;
        }
        Self {
            mantissa: normalized_mantissa,
            scale: normalized_scale,
        }
    }
}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Decimal {
    fn cmp(&self, other: &Self) -> Ordering {
        let scale = self.scale.max(other.scale);
        let left = self
            .mantissa
            .clone()
            * pow10((scale - self.scale) as usize);
        let right = other
            .mantissa
            .clone()
            * pow10((scale - other.scale) as usize);
        left.cmp(&right)
    }
}

impl From<i128> for Decimal {
    fn from(value: i128) -> Self {
        Self::from_int(value)
    }
}

impl TryFrom<&BigInt> for Decimal {
    type Error = NumericError;

    fn try_from(value: &BigInt) -> Result<Self, Self::Error> {
        Ok(Self::from_bigint_with_scale(value.clone(), 0))
    }
}

impl std::fmt::Display for Decimal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.mantissa.is_zero() {
            return write!(f, "0");
        }
        let negative = self.mantissa.sign() == Sign::Minus;
        let abs_digits = self.mantissa.clone().abs().to_string();
        if self.scale == 0 {
            if negative {
                write!(f, "-{}", abs_digits)
            } else {
                write!(f, "{}", abs_digits)
            }
        } else {
            let scale = self.scale as usize;
            let mut rendered = String::new();
            if negative {
                rendered.push('-');
            }
            if abs_digits.len() <= scale {
                rendered.push_str("0.");
                rendered.push_str("0".repeat(scale - abs_digits.len()).as_str());
                rendered.push_str(abs_digits.as_str());
            } else {
                let split = abs_digits.len() - scale;
                rendered.push_str(abs_digits[..split].to_string().as_str());
                rendered.push('.');
                rendered.push_str(abs_digits[split..].to_string().as_str());
            }
            write!(f, "{}", rendered)
        }
    }
}

fn pow10(power: usize) -> BigInt {
    let mut out = BigInt::from(1u8);
    for _ in 0..power {
        out *= 10u8;
    }
    out
}

fn pow_small(base: u8, exp: u32) -> BigInt {
    let mut out = BigInt::from(1u8);
    for _ in 0..exp {
        out *= base;
    }
    out
}

fn factor_out(mut value: BigInt, factor: u8) -> (BigInt, u32) {
    let mut count = 0u32;
    while (&value % factor).is_zero() {
        value /= factor;
        count += 1;
    }
    (value, count)
}

fn bigint_gcd(mut left: BigInt, mut right: BigInt) -> BigInt {
    while !right.is_zero() {
        let rem = left % &right;
        left = right;
        right = rem;
    }
    left.abs()
}

#[cfg(test)]
#[path = "numeric_test.rs"]
mod tests;
