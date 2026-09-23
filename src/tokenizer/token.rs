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
    Pub,
    Else,
    Comp,
    True,
    False,
    While,
    Return,

    // Symbols
    At,                 // @
    Eq,                 // =
    Dot,                // .
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

impl TokenKind {
    pub(crate) fn display(self) -> &'static str {
        use TokenKind::*;
        match self {
            Fn => "fn",
            If => "if",
            Let => "let",
            Mut => "mut",
            Pub => "pub",
            Else => "else",
            Comp => "comp",
            True => "true",
            False => "false",
            While => "while",
            Return => "return",

            At => "@",
            Eq => "=",
            Dot => ".",
            EqEq => "==",
            Semi => ";",
            Plus => "+",
            Star => "*",
            Bang => "!",
            Minus => "-",
            Slash => "/",
            Comma => ",",
            Colon => ":",
            RArrow => "->",
            LCurly => "{",
            RCurly => "}",
            LParen => "(",
            RParen => ")",
            LBrack => "[",
            RBrack => "]",
            BangEq => "!=",
            PlusEq => "+=",
            StarEq => "*=",
            MinusEq => "-=",
            SlashEq => "/=",
            LessThan => "<",
            LessThanOrEqual => "<=",
            GreaterThan => ">",
            GreaterThanOrEqual => ">=",

            Identifier => "<identifier>",
            NumberLiteral => "<number-literal>",
            StringLiteral => "<string-literal>",
        }
    }
}