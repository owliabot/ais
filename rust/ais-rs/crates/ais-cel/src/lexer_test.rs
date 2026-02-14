use super::{tokenize, TokenKind};

#[test]
fn tokenizes_composite_expression() {
    let tokens = tokenize("a.b[0] >= 10 && contains(name, 'x')").expect("tokenize");
    let kinds = tokens.into_iter().map(|token| token.kind).collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            TokenKind::Identifier,
            TokenKind::Dot,
            TokenKind::Identifier,
            TokenKind::LBracket,
            TokenKind::Integer,
            TokenKind::RBracket,
            TokenKind::Ge,
            TokenKind::Integer,
            TokenKind::AndAnd,
            TokenKind::Identifier,
            TokenKind::LParen,
            TokenKind::Identifier,
            TokenKind::Comma,
            TokenKind::String,
            TokenKind::RParen,
            TokenKind::Eof,
        ]
    );
}
