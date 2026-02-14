use crate::ast::{AstNode, BinaryOp, UnaryOp};
use crate::lexer::{tokenize, LexError, Token, TokenKind};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("lex error: {0}")]
    Lex(#[from] LexError),
    #[error("unexpected token at {pos}: expected {expected}, got {found}")]
    UnexpectedToken {
        expected: String,
        found: String,
        pos: usize,
    },
    #[error("invalid number literal at {pos}: {literal}")]
    InvalidNumber { literal: String, pos: usize },
}

pub struct Parser {
    tokens: Vec<Token>,
    index: usize,
}

pub fn parse_expression(input: &str) -> Result<AstNode, ParseError> {
    let tokens = tokenize(input)?;
    Parser::new(tokens).parse()
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, index: 0 }
    }

    pub fn parse(mut self) -> Result<AstNode, ParseError> {
        let expression = self.parse_ternary()?;
        self.expect(TokenKind::Eof)?;
        Ok(expression)
    }

    fn parse_ternary(&mut self) -> Result<AstNode, ParseError> {
        let mut condition = self.parse_or()?;
        if self.match_kind(TokenKind::Question) {
            let then_expr = self.parse_ternary()?;
            self.expect(TokenKind::Colon)?;
            let else_expr = self.parse_ternary()?;
            condition = AstNode::Ternary {
                condition: Box::new(condition),
                then_expr: Box::new(then_expr),
                else_expr: Box::new(else_expr),
            };
        }
        Ok(condition)
    }

    fn parse_or(&mut self) -> Result<AstNode, ParseError> {
        self.parse_binary_chain(|token| matches!(token, TokenKind::OrOr), Self::parse_and, BinaryOp::Or)
    }

    fn parse_and(&mut self) -> Result<AstNode, ParseError> {
        self.parse_binary_chain(|token| matches!(token, TokenKind::AndAnd), Self::parse_equality, BinaryOp::And)
    }

    fn parse_equality(&mut self) -> Result<AstNode, ParseError> {
        let mut node = self.parse_comparison()?;
        loop {
            if self.match_kind(TokenKind::EqEq) {
                let right = self.parse_comparison()?;
                node = AstNode::Binary {
                    left: Box::new(node),
                    op: BinaryOp::Eq,
                    right: Box::new(right),
                };
                continue;
            }
            if self.match_kind(TokenKind::NotEq) {
                let right = self.parse_comparison()?;
                node = AstNode::Binary {
                    left: Box::new(node),
                    op: BinaryOp::Ne,
                    right: Box::new(right),
                };
                continue;
            }
            break;
        }
        Ok(node)
    }

    fn parse_comparison(&mut self) -> Result<AstNode, ParseError> {
        let mut node = self.parse_term()?;
        loop {
            let op = if self.match_kind(TokenKind::Lt) {
                Some(BinaryOp::Lt)
            } else if self.match_kind(TokenKind::Le) {
                Some(BinaryOp::Le)
            } else if self.match_kind(TokenKind::Gt) {
                Some(BinaryOp::Gt)
            } else if self.match_kind(TokenKind::Ge) {
                Some(BinaryOp::Ge)
            } else if self.match_kind(TokenKind::In) {
                Some(BinaryOp::In)
            } else {
                None
            };

            let Some(op) = op else { break };
            let right = self.parse_term()?;
            node = AstNode::Binary {
                left: Box::new(node),
                op,
                right: Box::new(right),
            };
        }
        Ok(node)
    }

    fn parse_term(&mut self) -> Result<AstNode, ParseError> {
        let mut node = self.parse_factor()?;
        loop {
            let op = if self.match_kind(TokenKind::Plus) {
                Some(BinaryOp::Add)
            } else if self.match_kind(TokenKind::Minus) {
                Some(BinaryOp::Sub)
            } else {
                None
            };
            let Some(op) = op else { break };
            let right = self.parse_factor()?;
            node = AstNode::Binary {
                left: Box::new(node),
                op,
                right: Box::new(right),
            };
        }
        Ok(node)
    }

    fn parse_factor(&mut self) -> Result<AstNode, ParseError> {
        let mut node = self.parse_unary()?;
        loop {
            let op = if self.match_kind(TokenKind::Star) {
                Some(BinaryOp::Mul)
            } else if self.match_kind(TokenKind::Slash) {
                Some(BinaryOp::Div)
            } else if self.match_kind(TokenKind::Percent) {
                Some(BinaryOp::Mod)
            } else {
                None
            };
            let Some(op) = op else { break };
            let right = self.parse_unary()?;
            node = AstNode::Binary {
                left: Box::new(node),
                op,
                right: Box::new(right),
            };
        }
        Ok(node)
    }

    fn parse_unary(&mut self) -> Result<AstNode, ParseError> {
        if self.match_kind(TokenKind::Bang) {
            let expr = self.parse_unary()?;
            return Ok(AstNode::Unary {
                op: UnaryOp::Not,
                expr: Box::new(expr),
            });
        }
        if self.match_kind(TokenKind::Minus) {
            let expr = self.parse_unary()?;
            return Ok(AstNode::Unary {
                op: UnaryOp::Neg,
                expr: Box::new(expr),
            });
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<AstNode, ParseError> {
        let mut node = self.parse_primary()?;

        loop {
            if self.match_kind(TokenKind::Dot) {
                let token = self.expect(TokenKind::Identifier)?;
                node = AstNode::Member {
                    object: Box::new(node),
                    property: token.lexeme,
                };
                continue;
            }

            if self.match_kind(TokenKind::LBracket) {
                let index = self.parse_ternary()?;
                self.expect(TokenKind::RBracket)?;
                node = AstNode::Index {
                    object: Box::new(node),
                    index: Box::new(index),
                };
                continue;
            }

            if self.match_kind(TokenKind::LParen) {
                let mut args = Vec::new();
                if !self.check(TokenKind::RParen) {
                    loop {
                        args.push(self.parse_ternary()?);
                        if self.match_kind(TokenKind::Comma) {
                            continue;
                        }
                        break;
                    }
                }
                self.expect(TokenKind::RParen)?;
                node = AstNode::Call {
                    callee: Box::new(node),
                    args,
                };
                continue;
            }

            break;
        }

        Ok(node)
    }

    fn parse_primary(&mut self) -> Result<AstNode, ParseError> {
        let token = self.advance();
        match token.kind {
            TokenKind::Identifier => Ok(AstNode::Identifier(token.lexeme)),
            TokenKind::Integer => Ok(AstNode::Integer(token.lexeme)),
            TokenKind::Decimal => Ok(AstNode::Decimal(token.lexeme)),
            TokenKind::String => Ok(AstNode::String(token.lexeme)),
            TokenKind::True => Ok(AstNode::Bool(true)),
            TokenKind::False => Ok(AstNode::Bool(false)),
            TokenKind::Null => Ok(AstNode::Null),
            TokenKind::LParen => {
                let expr = self.parse_ternary()?;
                self.expect(TokenKind::RParen)?;
                Ok(expr)
            }
            TokenKind::LBracket => {
                let mut items = Vec::new();
                if !self.check(TokenKind::RBracket) {
                    loop {
                        items.push(self.parse_ternary()?);
                        if self.match_kind(TokenKind::Comma) {
                            continue;
                        }
                        break;
                    }
                }
                self.expect(TokenKind::RBracket)?;
                Ok(AstNode::List(items))
            }
            _ => Err(ParseError::UnexpectedToken {
                expected: "primary expression".to_string(),
                found: format!("{:?}", token.kind),
                pos: token.pos,
            }),
        }
    }

    fn parse_binary_chain<F, G>(
        &mut self,
        matcher: F,
        mut parse_operand: G,
        op: BinaryOp,
    ) -> Result<AstNode, ParseError>
    where
        F: Fn(&TokenKind) -> bool,
        G: FnMut(&mut Self) -> Result<AstNode, ParseError>,
    {
        let mut node = parse_operand(self)?;
        while matcher(&self.peek().kind) {
            self.advance();
            let right = parse_operand(self)?;
            node = AstNode::Binary {
                left: Box::new(node),
                op,
                right: Box::new(right),
            };
        }
        Ok(node)
    }

    fn check(&self, kind: TokenKind) -> bool {
        self.peek().kind == kind
    }

    fn match_kind(&mut self, kind: TokenKind) -> bool {
        if self.check(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: TokenKind) -> Result<Token, ParseError> {
        let token = self.advance();
        if token.kind == kind {
            Ok(token)
        } else {
            Err(ParseError::UnexpectedToken {
                expected: format!("{:?}", kind),
                found: format!("{:?}", token.kind),
                pos: token.pos,
            })
        }
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.index]
    }

    fn advance(&mut self) -> Token {
        let token = self.tokens[self.index].clone();
        if self.index + 1 < self.tokens.len() {
            self.index += 1;
        }
        token
    }
}

#[cfg(test)]
#[path = "parser_test.rs"]
mod tests;
