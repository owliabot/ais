use std::{cmp::Ordering, str::FromStr};

use bigdecimal::BigDecimal;
use num_bigint::BigInt;
use num_traits::Zero;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decimal {
    value: BigDecimal,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NumericError {
    #[error("invalid decimal literal: {0}")]
    InvalidDecimalLiteral(String),
    #[error("division by zero")]
    DivisionByZero,
    #[error("non exact division")]
    NonExactDivision,
}

impl Decimal {
    pub fn parse(input: &str) -> Result<Self, NumericError> {
        let parsed = BigDecimal::from_str(input)
            .map_err(|_| NumericError::InvalidDecimalLiteral(input.to_owned()))?;
        Ok(Self::from(parsed))
    }

    pub fn from_bigint(value: BigInt) -> Self {
        Self::from(value)
    }

    pub fn from_atomic_int(value: BigInt, decimals: u32) -> Self {
        Self::from(BigDecimal::new(value, decimals as i64))
    }

    pub fn to_atomic_int(&self, decimals: u32) -> Result<BigInt, NumericError> {
        let (mantissa, scale) = self.normalized_components();
        if decimals >= scale {
            let factor = pow10((decimals - scale) as usize);
            return Ok(mantissa * factor);
        }

        let divisor = pow10((scale - decimals) as usize);
        if (&mantissa % &divisor) != BigInt::zero() {
            return Err(NumericError::NonExactDivision);
        }
        Ok(mantissa / divisor)
    }

    pub fn scale(&self) -> u32 {
        self.normalized_components().1
    }

    pub fn mantissa(&self) -> BigInt {
        self.normalized_components().0
    }

    pub fn is_zero(&self) -> bool {
        self.value.is_zero()
    }

    pub fn is_negative(&self) -> bool {
        self.value < BigDecimal::from(0)
    }

    pub fn add(&self, other: &Decimal) -> Decimal {
        Self::from(BigDecimal::from(self) + BigDecimal::from(other))
    }

    pub fn sub(&self, other: &Decimal) -> Decimal {
        Self::from(BigDecimal::from(self) - BigDecimal::from(other))
    }

    pub fn mul(&self, other: &Decimal) -> Decimal {
        Self::from(BigDecimal::from(self) * BigDecimal::from(other))
    }

    pub fn div(&self, other: &Decimal) -> Result<Decimal, NumericError> {
        if other.is_zero() {
            return Err(NumericError::DivisionByZero);
        }
        Ok(Self::from(BigDecimal::from(self) / BigDecimal::from(other)))
    }

    pub fn neg(&self) -> Decimal {
        Self::from(-BigDecimal::from(self))
    }

    pub fn abs(&self) -> Decimal {
        Self::from(BigDecimal::from(self).abs())
    }

    fn normalized_components(&self) -> (BigInt, u32) {
        let (mantissa, exponent) = self.value.normalized().into_bigint_and_exponent();
        if mantissa.is_zero() {
            return (BigInt::zero(), 0);
        }
        if exponent >= 0 {
            return (mantissa, exponent as u32);
        }
        let factor = pow10((-exponent) as usize);
        (mantissa * factor, 0)
    }
}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Decimal {
    fn cmp(&self, other: &Self) -> Ordering {
        self.value.cmp(&other.value)
    }
}

impl std::fmt::Display for Decimal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.value.normalized())
    }
}

impl From<BigDecimal> for Decimal {
    fn from(value: BigDecimal) -> Self {
        Self {
            value: value.normalized(),
        }
    }
}

impl From<BigInt> for Decimal {
    fn from(value: BigInt) -> Self {
        Self::from(BigDecimal::from(value))
    }
}

impl From<Decimal> for BigDecimal {
    fn from(value: Decimal) -> Self {
        value.value
    }
}

impl From<&Decimal> for BigDecimal {
    fn from(value: &Decimal) -> Self {
        value.value.clone()
    }
}

fn pow10(power: usize) -> BigInt {
    let mut out = BigInt::from(1u8);
    for _ in 0..power {
        out *= 10u8;
    }
    out
}
