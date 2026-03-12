#[derive(Debug, Clone, PartialEq)]
pub enum AstNode {
    Null,
    Bool(bool),
    Integer(String),
    Decimal(String),
    String(String),
    Identifier(String),
    List(Vec<AstNode>),
    Unary {
        op: UnaryOp,
        expr: Box<AstNode>,
    },
    Binary {
        left: Box<AstNode>,
        op: BinaryOp,
        right: Box<AstNode>,
    },
    Ternary {
        condition: Box<AstNode>,
        then_expr: Box<AstNode>,
        else_expr: Box<AstNode>,
    },
    Member {
        object: Box<AstNode>,
        property: String,
    },
    Index {
        object: Box<AstNode>,
        index: Box<AstNode>,
    },
    Call {
        callee: Box<AstNode>,
        args: Vec<AstNode>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Not,
    Neg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    In,
}
