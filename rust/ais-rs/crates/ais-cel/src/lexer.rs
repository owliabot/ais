#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub lexeme: String,
    pub pos: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    Identifier,
    Integer,
    Decimal,
    String,
    True,
    False,
    Null,
    In,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Dot,
    Question,
    Colon,
    Bang,
    EqEq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    AndAnd,
    OrOr,
    Eof,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LexError {
    #[error("unexpected character '{ch}' at {pos}")]
    UnexpectedCharacter { ch: char, pos: usize },
    #[error("unterminated string at {pos}")]
    UnterminatedString { pos: usize },
    #[error("invalid escape sequence at {pos}")]
    InvalidEscape { pos: usize },
}

pub fn tokenize(input: &str) -> Result<Vec<Token>, LexError> {
    let chars: Vec<char> = input.chars().collect();
    let mut index = 0usize;
    let mut tokens = Vec::new();

    while index < chars.len() {
        let ch = chars[index];
        if ch.is_whitespace() {
            index += 1;
            continue;
        }

        if ch.is_ascii_alphabetic() || ch == '_' {
            tokens.push(consume_identifier(&chars, &mut index));
            continue;
        }

        if ch.is_ascii_digit() {
            tokens.push(consume_number(&chars, &mut index));
            continue;
        }

        if ch == '\'' || ch == '"' {
            tokens.push(consume_string(&chars, &mut index)?);
            continue;
        }

        let token = tokenize_symbol(&chars, &mut index)?;
        tokens.push(token);
        index += 1;
    }

    tokens.push(Token {
        kind: TokenKind::Eof,
        lexeme: String::new(),
        pos: chars.len(),
    });

    Ok(tokens)
}

fn consume_identifier(chars: &[char], index: &mut usize) -> Token {
    let start = *index;
    *index += 1;
    while *index < chars.len() && (chars[*index].is_ascii_alphanumeric() || chars[*index] == '_') {
        *index += 1;
    }
    let lexeme: String = chars[start..*index].iter().collect();
    let kind = match lexeme.as_str() {
        "true" => TokenKind::True,
        "false" => TokenKind::False,
        "null" => TokenKind::Null,
        "in" => TokenKind::In,
        _ => TokenKind::Identifier,
    };
    Token {
        kind,
        lexeme,
        pos: start,
    }
}

fn consume_number(chars: &[char], index: &mut usize) -> Token {
    let start = *index;
    *index += 1;
    while *index < chars.len() && chars[*index].is_ascii_digit() {
        *index += 1;
    }

    let mut kind = TokenKind::Integer;
    if *index < chars.len() && chars[*index] == '.' {
        let dot = *index;
        *index += 1;
        if *index >= chars.len() || !chars[*index].is_ascii_digit() {
            *index = dot;
        } else {
            kind = TokenKind::Decimal;
            while *index < chars.len() && chars[*index].is_ascii_digit() {
                *index += 1;
            }
        }
    }

    let lexeme: String = chars[start..*index].iter().collect();
    Token {
        kind,
        lexeme,
        pos: start,
    }
}

fn consume_string(chars: &[char], index: &mut usize) -> Result<Token, LexError> {
    let quote = chars[*index];
    let start = *index;
    *index += 1;
    let mut out = String::new();
    let mut terminated = false;

    while *index < chars.len() {
        let current = chars[*index];
        if current == quote {
            *index += 1;
            terminated = true;
            break;
        }
        if current == '\\' {
            *index += 1;
            if *index >= chars.len() {
                return Err(LexError::UnterminatedString { pos: start });
            }
            let escaped = chars[*index];
            let decoded = decode_escape(escaped, *index)?;
            out.push(decoded);
            *index += 1;
            continue;
        }
        out.push(current);
        *index += 1;
    }

    if !terminated {
        return Err(LexError::UnterminatedString { pos: start });
    }

    Ok(Token {
        kind: TokenKind::String,
        lexeme: out,
        pos: start,
    })
}

fn decode_escape(escaped: char, pos: usize) -> Result<char, LexError> {
    let decoded = match escaped {
        'n' => '\n',
        'r' => '\r',
        't' => '\t',
        '\\' => '\\',
        '\'' => '\'',
        '"' => '"',
        _ => return Err(LexError::InvalidEscape { pos }),
    };
    Ok(decoded)
}

fn tokenize_symbol(chars: &[char], index: &mut usize) -> Result<Token, LexError> {
    let ch = chars[*index];
    let pos = *index;
    if let Some(token) = two_char_op(chars, index, ch, pos)? {
        return Ok(token);
    }
    one_char_op(ch, pos).ok_or(LexError::UnexpectedCharacter { ch, pos })
}

fn simple(kind: TokenKind, ch: char, pos: usize) -> Token {
    Token {
        kind,
        lexeme: ch.to_string(),
        pos,
    }
}

fn token_pair(kind: TokenKind, lexeme: &str, pos: usize) -> Token {
    Token {
        kind,
        lexeme: lexeme.to_string(),
        pos,
    }
}

fn two_char_op(chars: &[char], index: &mut usize, ch: char, pos: usize) -> Result<Option<Token>, LexError> {
    let token = match ch {
        '!' if matches_next(chars, pos, '=') => Some((TokenKind::NotEq, "!=")),
        '=' if matches_next(chars, pos, '=') => Some((TokenKind::EqEq, "==")),
        '<' if matches_next(chars, pos, '=') => Some((TokenKind::Le, "<=")),
        '>' if matches_next(chars, pos, '=') => Some((TokenKind::Ge, ">=")),
        '&' if matches_next(chars, pos, '&') => Some((TokenKind::AndAnd, "&&")),
        '|' if matches_next(chars, pos, '|') => Some((TokenKind::OrOr, "||")),
        '=' | '&' | '|' => return Err(LexError::UnexpectedCharacter { ch, pos }),
        _ => None,
    };
    if let Some((kind, lexeme)) = token {
        *index += 1;
        return Ok(Some(token_pair(kind, lexeme, pos)));
    }
    Ok(None)
}

fn one_char_op(ch: char, pos: usize) -> Option<Token> {
    let kind = match ch {
        '+' => TokenKind::Plus,
        '-' => TokenKind::Minus,
        '*' => TokenKind::Star,
        '/' => TokenKind::Slash,
        '%' => TokenKind::Percent,
        '(' => TokenKind::LParen,
        ')' => TokenKind::RParen,
        '[' => TokenKind::LBracket,
        ']' => TokenKind::RBracket,
        ',' => TokenKind::Comma,
        '.' => TokenKind::Dot,
        '?' => TokenKind::Question,
        ':' => TokenKind::Colon,
        '!' => TokenKind::Bang,
        '<' => TokenKind::Lt,
        '>' => TokenKind::Gt,
        _ => return None,
    };
    Some(simple(kind, ch, pos))
}

fn matches_next(chars: &[char], index: usize, expected: char) -> bool {
    chars.get(index + 1).copied() == Some(expected)
}

#[cfg(test)]
#[path = "lexer_test.rs"]
mod tests;
