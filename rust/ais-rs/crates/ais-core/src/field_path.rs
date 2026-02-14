use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum FieldPathSegment {
    Key(String),
    Index(usize),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FieldPath {
    segments: Vec<FieldPathSegment>,
}

impl FieldPath {
    pub fn root() -> Self {
        Self { segments: Vec::new() }
    }

    pub fn from_segments(segments: Vec<FieldPathSegment>) -> Self {
        Self { segments }
    }

    pub fn push_key(&mut self, key: impl Into<String>) {
        self.segments.push(FieldPathSegment::Key(key.into()));
    }

    pub fn push_index(&mut self, index: usize) {
        self.segments.push(FieldPathSegment::Index(index));
    }

    pub fn segments(&self) -> &[FieldPathSegment] {
        &self.segments
    }

    pub fn is_root(&self) -> bool {
        self.segments.is_empty()
    }
}

impl Default for FieldPath {
    fn default() -> Self {
        Self::root()
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FieldPathParseError {
    #[error("field path must start with '$' or an identifier")]
    InvalidStart,
    #[error("unexpected end of input")]
    UnexpectedEnd,
    #[error("invalid index segment")]
    InvalidIndex,
    #[error("expected '.' before key segment")]
    MissingDot,
    #[error("invalid key segment")]
    InvalidKey,
    #[error("unexpected character '{0}'")]
    UnexpectedChar(char),
}

impl std::str::FromStr for FieldPath {
    type Err = FieldPathParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        if input.is_empty() {
            return Err(FieldPathParseError::InvalidStart);
        }

        let bytes = input.as_bytes();
        let mut position = 0;
        if bytes[0] == b'$' {
            position += 1;
            if position == bytes.len() {
                return Ok(FieldPath::root());
            }
            if bytes[position] != b'.' && bytes[position] != b'[' {
                return Err(FieldPathParseError::UnexpectedChar(bytes[position] as char));
            }
        }

        let mut segments = Vec::new();
        let mut expect_key_or_index = position == 0;

        while position < bytes.len() {
            match bytes[position] {
                b'.' => {
                    position += 1;
                    if position >= bytes.len() {
                        return Err(FieldPathParseError::UnexpectedEnd);
                    }
                    let start = position;
                    while position < bytes.len() {
                        let c = bytes[position] as char;
                        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                            position += 1;
                            continue;
                        }
                        break;
                    }
                    if start == position {
                        return Err(FieldPathParseError::InvalidKey);
                    }
                    let key = &input[start..position];
                    segments.push(FieldPathSegment::Key(key.to_string()));
                    expect_key_or_index = false;
                }
                b'[' => {
                    position += 1;
                    let start = position;
                    while position < bytes.len() && bytes[position].is_ascii_digit() {
                        position += 1;
                    }
                    if start == position {
                        return Err(FieldPathParseError::InvalidIndex);
                    }
                    if position >= bytes.len() || bytes[position] != b']' {
                        return Err(FieldPathParseError::InvalidIndex);
                    }
                    let index = input[start..position]
                        .parse::<usize>()
                        .map_err(|_| FieldPathParseError::InvalidIndex)?;
                    position += 1;
                    segments.push(FieldPathSegment::Index(index));
                    expect_key_or_index = false;
                }
                _ => {
                    if expect_key_or_index {
                        let start = position;
                        while position < bytes.len() {
                            let c = bytes[position] as char;
                            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                                position += 1;
                                continue;
                            }
                            break;
                        }
                        if start == position {
                            return Err(FieldPathParseError::InvalidStart);
                        }
                        let key = &input[start..position];
                        segments.push(FieldPathSegment::Key(key.to_string()));
                        expect_key_or_index = false;
                    } else {
                        return Err(FieldPathParseError::MissingDot);
                    }
                }
            }
        }

        Ok(FieldPath::from_segments(segments))
    }
}

impl Display for FieldPath {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "$")?;
        for segment in &self.segments {
            match segment {
                FieldPathSegment::Key(key) => write!(f, ".{key}")?,
                FieldPathSegment::Index(index) => write!(f, "[{index}]")?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "field_path_test.rs"]
mod tests;
