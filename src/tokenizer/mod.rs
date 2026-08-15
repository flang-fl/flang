use std::str::Chars;
use crate::diagnostics::Diagnostic;
use crate::source::{SourceFile, Span};

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
                "Evil :(".to_owned()
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
        while let Some(next) = self.peek() && (next.is_ascii_alphanumeric() || next == '_') {
            self.next();
        }

        let span = self.source.span(start, self.index);
        
        let kind = if let Some(k) = 
            Self::keyword(self.source.span_text(span)) { k } else {
            TokenKind::Identifier
        };

        Ok(Token {
            span,
            kind,
        })
    }

    fn keyword(identifier: &str) -> Option<TokenKind> {
        match identifier {
            "fn" => Some(TokenKind::Fn),
            "comp" => Some(TokenKind::Comp),
            "return" => Some(TokenKind::Return),

            _ => None,
        }
    }

    fn tokenize_number_literal(&mut self) -> Result<Token, Diagnostic> {
        // TODO: Decimal Numbers
        let start = self.index;
        while let Some(next) = self.peek() && next.is_ascii_digit() {
            self.next();
        }
        Ok(Token {
            span: self.source.span(start, self.index),
            kind: TokenKind::NumberLiteral
        })
    }

    fn process_symbol_tokens(&mut self) -> Option<Token> {
        let Some(next) = self.peek() else {
            return None;
        };

        let single_char_token_kind = match next {
            '=' => TokenKind::Eq,
            '(' => TokenKind::LParen,
            ')' => TokenKind::RParen,
            '{' => TokenKind::LCurly,
            '}' => TokenKind::RCurly,
            ';' => TokenKind::Semi,
            ',' => TokenKind::Comma,
            ':' => TokenKind::Colon,

            '+' => TokenKind::Plus,
            '-' => {
                if self.peek_offset(1) == Some('>') {
                    let start = self.index;
                    self.next();
                    self.next();
                    let end = self.index;

                    return Some(Token {
                        span: self.source.span(start, end),
                        kind: TokenKind::RArrow,
                    });
                }

                TokenKind::Minus
            }
            '*' => TokenKind::Star,
            '/' => TokenKind::Slash,

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
    Comp,
    Return,

    // Symbols
    Eq,
    Semi,
    Plus,
    Star,
    Minus,
    Slash,
    Comma,
    Colon,
    RArrow,
    LCurly,
    RCurly,
    LParen,
    RParen,

    // Special
    Identifier,
    NumberLiteral,
}