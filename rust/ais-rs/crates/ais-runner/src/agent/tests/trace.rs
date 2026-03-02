use super::*;

#[test]
fn compact_value_normalizes_whitespace() {
    assert_eq!(compact_value(" a \n  b\t c "), "a b c");
}
