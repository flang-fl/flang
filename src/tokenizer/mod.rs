use crate::diagnostics::Diagnostic;
use crate::source::{SourceFile, Span};

pub struct Tokenizer<'src> {
    source: &'src SourceFile,
    index: usize,
    tokens: Vec<Token>,
    diagnostics: Vec<Diagnostic>,
}

impl<'src> Tokenizer<'src> {
    pub fn new(source: &'src SourceFile) -> Self {
        Tokenizer {
            source,
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
        Ok(Token {
            span: self.source.span(start, self.index),
            kind: TokenKind::Identifier,
        })
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

            '-' => {
                // in the future single-token support
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

                todo!()
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
        self.source.text().chars().nth(self.index)
    }

    fn peek_offset(&self, offset: isize) -> Option<char> {
        self.source.text().chars().nth((self.index as isize + offset) as usize)
    }

    fn next(&mut self) -> Option<char> {
        self.index += 1;
        self.peek_offset(-1)
    }
}

#[derive(Debug)]
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
    RArrow,
    LCurly,
    RCurly,
    LParen,
    RParen,

    // Special
    Identifier,
    NumberLiteral,
}