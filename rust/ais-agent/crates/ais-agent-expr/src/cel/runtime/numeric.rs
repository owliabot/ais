use std::{cmp::Ordering, str::FromStr};

use bigdecimal::BigDecimal;
use num_bigint::BigInt;
use num_traits::Zero;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decimal(pub BigDecimal);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NumericError {
    #[error("invalid decimal literal: {0}")]
    InvalidDecimalLiteral(String),
    #[error("division by zero")]
    DivisionByZero,
}

impl Decimal {
    pub fn parse(input: &str) -> Result<Self, NumericError> {
        BigDecimal::from_str(input)
            .map(Self)
            .map_err(|_| NumericError::InvalidDecimalLiteral(input.to_owned()))
    }

    pub fn from_bigint(value: BigInt) -> Self {
        Self(BigDecimal::from(value))
    }

    pub fn add(&self, other: &Decimal) -> Decimal {
        Self(&self.0 + &other.0)
    }

    pub fn sub(&self, other: &Decimal) -> Decimal {
        Self(&self.0 - &other.0)
    }

    pub fn mul(&self, other: &Decimal) -> Decimal {
        Self(&self.0 * &other.0)
    }

    pub fn div(&self, other: &Decimal) -> Result<Decimal, NumericError> {
        if other.0.is_zero() {
            return Err(NumericError::DivisionByZero);
        }
        Ok(Self(&self.0 / &other.0))
    }

    pub fn neg(&self) -> Decimal {
        Self(-&self.0)
    }

    pub fn abs(&self) -> Decimal {
        if self.0 < BigDecimal::from(0) {
            self.neg()
        } else {
            self.clone()
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
        self.0.cmp(&other.0)
    }
}

impl std::fmt::Display for Decimal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
