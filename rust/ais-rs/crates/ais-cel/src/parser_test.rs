use super::parse_expression;
use crate::ast::{AstNode, BinaryOp};

#[test]
fn parses_operator_precedence() {
    let ast = parse_expression("1 + 2 * 3").expect("parse");
    let AstNode::Binary { left, op, right } = ast else {
        panic!("expected binary");
    };
    assert_eq!(op, BinaryOp::Add);
    assert_eq!(*left, AstNode::Integer("1".to_string()));
    let AstNode::Binary { op, .. } = *right else {
        panic!("expected nested binary");
    };
    assert_eq!(op, BinaryOp::Mul);
}

#[test]
fn parses_member_call_and_index() {
    let ast = parse_expression("foo.bar(1, 2)[0]").expect("parse");
    let AstNode::Index { object, .. } = ast else {
        panic!("expected index");
    };
    let AstNode::Call { callee, args } = *object else {
        panic!("expected call");
    };
    assert_eq!(args.len(), 2);
    let AstNode::Member { property, .. } = *callee else {
        panic!("expected member");
    };
    assert_eq!(property, "bar");
}

#[test]
fn parses_ternary_expression() {
    let ast = parse_expression("a > 1 ? b : c").expect("parse");
    let AstNode::Ternary { .. } = ast else {
        panic!("expected ternary");
    };
}
