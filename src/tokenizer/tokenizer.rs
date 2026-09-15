use crate::diagnostics::Diagnostic;
use crate::source::SourceFile;
use crate::tokenizer::{Token, TokenKind};
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

            if next == '"' {
                let start = self.index;
                self.next();
                while let Some(next) = self.peek() && next != '"' {
                    match next {
                        '\n' | '\\' => {
                            self.diagnostics.push(Diagnostic::error(
                                "Unsupported character in string literal",
                                self.source.span(self.index, self.index + 1),
                                format!(
                                    "{} is not supported",
                                    next
                                )
                            ));
                        }

                        _ => {}
                    }
                    self.next();
                }

                match self.peek() {
                    None => {
                        self.diagnostics.push(Diagnostic::error(
                            "Unterminated string literal",
                            self.source.span(self.index, self.index),
                            ":("
                        ));
                    }
                    Some(_) => {
                        if let Err(diagnostic) = self.expect('"') {
                            self.diagnostics.push(diagnostic);
                        }
                    }
                }

                let end = self.index;

                self.tokens.push(Token {
                    span: self.source.span(start, end),
                    kind: TokenKind::StringLiteral
                });

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
            "pub" => TokenKind::Pub,
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
        // Number
        while let Some(next) = self.peek()
            && next.is_ascii_digit()
        {
            self.next();
        }

        // Optional suffix like `5i32`
        while let Some(next) = self.peek()
              && next.is_ascii_alphanumeric()
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
            '@' => TokenKind::At,

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
        let next = self.chars.next()?;
        self.index += 1;
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use ariadne::Source;
    use crate::source::SourceId;
    use super::*;

    fn source(text: &str) -> SourceFile {
        SourceFile {
            id: SourceId(0),
            name: "test.fl".to_owned(),
            source: Source::from(text.to_owned())
        }
    }

    fn assert_single_string(input: &str, expected_contents: &str) {
        let source = source(input);
        let tokens = Tokenizer::new(&source)
            .tokenize()
            .expect("tokenization should succeed");

        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, TokenKind::StringLiteral);
        assert_eq!(source.span_text(tokens[0].span), input);

        let complete_literal = source.span_text(tokens[0].span);
        let contents = &complete_literal[1..complete_literal.len() - 1];

        assert_eq!(contents, expected_contents);
    }

    fn tokenize_error(input: &str) -> (SourceFile, Vec<Diagnostic>) {
        let source = source(input);
        let diagnostics = Tokenizer::new(&source)
            .tokenize()
            .expect_err("tokenization should fail");

        (source, diagnostics)
    }

    #[test]
    fn tokenizes_ascii_string_literal() {
        assert_single_string(
            "\"getchar\"", "getchar"
        );
    }

    #[test]
    fn tokenizes_empty_string_literal() {
        assert_single_string(
            "\"\"", ""
        );
    }

    #[test]
    fn tokenizes_utf8_string_literal() {
        assert_single_string(
            "\"héllo 世界\"",
            "héllo 世界"
        );
    }

    #[test]
    fn tokenizes_adjacent_string_literals() {
        let source = source("\"a\" \"b\"");
        let tokens = Tokenizer::new(&source)
            .tokenize()
            .expect("tokenization should succeed");

        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0].kind, TokenKind::StringLiteral);
        assert_eq!(tokens[1].kind, TokenKind::StringLiteral);
        assert_eq!(source.span_text(tokens[0].span), "\"a\"");
        assert_eq!(source.span_text(tokens[1].span), "\"b\"");
    }

    #[test]
    fn rejects_backslash_in_string_literal() {
        let (source, diagnostics) = tokenize_error("\"é\\\"");

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].message,
            "Unsupported character in string literal"
        );
        assert_eq!(
            source.span_text(diagnostics[0].primary.span),
            "\\"
        );
    }

    #[test]
    fn rejects_newline_in_string_literal() {
        let (source, diagnostics) = tokenize_error("\"first\nsecond\"");

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            source.span_text(diagnostics[0].primary.span),
            "\n"
        );
    }

    #[test]
    fn rejects_unterminated_string_literal() {
        let (_source, diagnostics) = tokenize_error("\"unfinished");

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].message,
            "Unterminated string literal"
        );
    }
}