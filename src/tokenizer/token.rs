use crate::source::Span;

#[derive(Debug, Clone, Copy)]
pub struct Token {
    pub span: Span,
    pub kind: TokenKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    // Keywords
    Fn,
    If,
    Let,
    Mut,
    Else,
    Comp,
    True,
    False,
    While,
    Return,

    // Symbols
    At,                 // @
    Eq,                 // =
    EqEq,               // ==
    Semi,               // ;
    Plus,               // +
    Star,               // *
    Bang,               // !
    Minus,              // -
    Slash,              // /
    Comma,              // ,
    Colon,              // :
    RArrow,             // ->
    LCurly,             // {
    RCurly,             // }
    LParen,             // (
    RParen,             // )
    LBrack,             // [
    RBrack,             // ]
    BangEq,             // !=
    PlusEq,             // +=
    StarEq,             // *=
    MinusEq,            // -=
    SlashEq,            // /=
    LessThan,           // <
    LessThanOrEqual,    // <=
    GreaterThan,        // >
    GreaterThanOrEqual, // >=

    // Special
    Identifier,
    NumberLiteral,
    StringLiteral,
}
