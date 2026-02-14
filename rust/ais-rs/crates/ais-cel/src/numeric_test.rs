use super::{Decimal, NumericError};
use num_bigint::BigInt;
#[test]
fn parse_and_normalize_decimal() {
    let value = Decimal::parse("12.3400").expect("parse");
    assert_eq!(value.mantissa(), &BigInt::from(1234u64));
    assert_eq!(value.scale(), 2);
    assert_eq!(value.to_string(), "12.34");
}

#[test]
fn add_decimal_with_alignment() {
    let left = Decimal::parse("1.2").expect("parse");
    let right = Decimal::parse("0.03").expect("parse");
    let out = left.add(&right).expect("add");
    assert_eq!(out.to_string(), "1.23");
}

#[test]
fn non_exact_division_is_rejected() {
    let left = Decimal::from_int(1);
    let right = Decimal::from_int(3);
    let err = left.div_exact(&right).expect_err("must fail");
    assert_eq!(err, NumericError::NonExactDivision);
}
