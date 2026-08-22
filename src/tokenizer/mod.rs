use crate::diagnostics::Diagnostic;
use crate::source::{SourceFile, Span};
use std::str::Chars;

pub struct Tokenizer<'src> {
    source: &'src SourceFile,
    chars: Chars<'src>,
    index: usize,
    tokens: Vec<Token>,
    diagnostics: Vec<Diagnostic>,
}

impl<'src> Tokenizer<'src> {
    pub fn new(source: &'src SourceFile) -> Self {
        Tokenizer {
            source,
            chars: source.text().chars(),
            index: 0,
            diagnostics: Vec::new(),
            tokens: Vec::new(),
        }
    }

    pub fn tokenize(mut self) -> Result<Vec<Token>, Vec<Diagnostic>> {
        while let Some(next) = self.peek() {
            if let Some(token) = self.process_symbol_tokens() {
                self.tokens.push(token);
                continue;
            }

            if next.is_ascii_digit() {
                match self.tokenize_number_literal() {
                    Ok(token) => self.tokens.push(token),
                    Err(diagnostic) => self.diagnostics.push(diagnostic),
                }
                continue;
            }

            if next.is_ascii_alphabetic() {
                match self.tokenize_identifier() {
                    Ok(token) => self.tokens.push(token),
                    Err(diagnostic) => self.diagnostics.push(diagnostic),
                }
                continue;
            }

            if next.is_ascii_whitespace() {
                self.next();
                continue;
            }

            self.diagnostics.push(Diagnostic::error(
                format!("Unexpected character '{next}'"),
                self.source.span(self.index, self.index + 1),
                "Evil :(".to_owned(),
            ));
            self.next();
        }

        if self.diagnostics.is_empty() {
            Ok(self.tokens)
        } else {
            Err(self.diagnostics)
        }
    }

    fn tokenize_identifier(&mut self) -> Result<Token, Diagnostic> {
        let start = self.index;
        while let Some(next) = self.peek()
            && (next.is_ascii_alphanumeric() || next == '_')
        {
            self.next();
        }

        let span = self.source.span(start, self.index);

        let kind = if let Some(k) = Self::keyword(self.source.span_text(span)) {
            k
        } else {
            TokenKind::Identifier
        };

        Ok(Token { span, kind })
    }

    fn keyword(identifier: &str) -> Option<TokenKind> {
        Some(match identifier {
            "fn" => TokenKind::Fn,
            "if" => TokenKind::If,
            "let" => TokenKind::Let,
            "mut" => TokenKind::Mut,
            "else" => TokenKind::Else,
            "comp" => TokenKind::Comp,
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            "while" => TokenKind::While,
            "return" => TokenKind::Return,

            _ => return None,
        })
    }

    fn tokenize_number_literal(&mut self) -> Result<Token, Diagnostic> {
        // TODO: Decimal Numbers
        let start = self.index;
        while let Some(next) = self.peek()
            && next.is_ascii_digit()
        {
            self.next();
        }
        Ok(Token {
            span: self.source.span(start, self.index),
            kind: TokenKind::NumberLiteral,
        })
    }

    fn single_or_double_tokens(
        &mut self,
        char1: char,
        second_chars: &[char],
        token1: TokenKind,
        second_tokens: &[TokenKind],
    ) -> Option<Token> {
        let start = self.index;
        if let Err(diagnostic) = self.expect(char1) {
            self.diagnostics.push(diagnostic);
            None
        } else {
            for (char, token) in second_chars.iter().zip(second_tokens) {
                if self.peek() == Some(*char) {
                    self.next();
                    return Some(Token {
                        span: self.source.span(start, self.index),
                        kind: *token,
                    });
                }
            }
            Some(Token {
                span: self.source.span(start, self.index),
                kind: token1,
            })
        }
    }

    fn process_symbol_tokens(&mut self) -> Option<Token> {
        let Some(next) = self.peek() else {
            return None;
        };

        let single_char_token_kind = match next {
            '(' => TokenKind::LParen,
            ')' => TokenKind::RParen,
            '{' => TokenKind::LCurly,
            '}' => TokenKind::RCurly,
            '[' => TokenKind::LBrack,
            ']' => TokenKind::RBrack,
            ';' => TokenKind::Semi,
            ',' => TokenKind::Comma,
            ':' => TokenKind::Colon,

            '+' => {
                return self.single_or_double_tokens(
                    '+', &['='],
                    TokenKind::Plus, &[TokenKind::PlusEq]
                )
            }
            '-' => {
                return self.single_or_double_tokens(
                    '-', &['>', '='],
                    TokenKind::Minus, &[TokenKind::RArrow, TokenKind::MinusEq]);
            }
            '*' => {
                return self.single_or_double_tokens(
                    '*', &['='],
                    TokenKind::Star, &[TokenKind::StarEq]
                )
            },
            '/' => {
                return self.single_or_double_tokens(
                    '/', &['='],
                    TokenKind::Slash, &[TokenKind::SlashEq]
                )
            },

            '=' => {
                return self.single_or_double_tokens(
                    '=', &['='],
                    TokenKind::Eq, &[TokenKind::EqEq]
                );
            }

            '!' => {
                return self.single_or_double_tokens(
                    '!', &['='],
                    TokenKind::Bang, &[TokenKind::BangEq]
                );
            }

            '<' => {
                return self.single_or_double_tokens(
                    '<', &['='],
                    TokenKind::LessThan, &[TokenKind::LessThanOrEqual]
                )
            }

            '>' => {
                return self.single_or_double_tokens(
                    '>', &['='],
                    TokenKind::GreaterThan, &[TokenKind::GreaterThanOrEqual]
                )
            }

            _ => return None,
        };

        Some(self.consume_and_build_single_char_token(single_char_token_kind))
    }

    fn consume_and_build_single_char_token(&mut self, kind: TokenKind) -> Token {
        let start = self.index;
        self.next();
        Token {
            kind,
            span: self.source.span(start, self.index),
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.clone().next()
    }

    fn peek_offset(&self, offset: isize) -> Option<char> {
        self.chars.clone().nth(offset as usize)
    }

    fn expect(&mut self, char: char) -> Result<(), Diagnostic> {
        if self.peek() == Some(char) {
            self.next();
            Ok(())
        } else {
            Err(Diagnostic::error(
                format!("Expected `{}` but got `{:?}`", char, self.peek()),
                self.source.span(self.index, self.index + 1),
                "here",
            ))
        }
    }

    fn next(&mut self) -> Option<char> {
        let next = self.chars.next();

        if next.is_some() {
            self.index += 1;
        }

        next
    }
}

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
}
