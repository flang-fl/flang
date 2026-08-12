use crate::diagnostics::Diagnostic;
use crate::parser::ast::Phase::Comptime;
use crate::parser::ast::{Binding, Block, Expression, ExpressionData, FunctionExpression, Item, ItemData, Program, Statement, StatementData, TypeExpression, TypeExpressionData};
use crate::source::SourceFile;
use crate::tokenizer::{Token, TokenKind};

pub mod ast;

pub struct Parser<'src, 'tokens> {
    source: &'src SourceFile,
    tokens: &'tokens [Token],
    index: usize,
    diagnostics: Vec<Diagnostic>,
}

impl<'src, 'tokens> Parser<'src, 'tokens> {
    pub fn new(source: &'src SourceFile, tokens: &'tokens [Token]) -> Self {
        Self {
            tokens,
            source,
            index: 0,
            diagnostics: Vec::new(),
        }
    }

    pub fn parse(mut self) -> Result<Program, Vec<Diagnostic>> {
        let mut program = Program {
            items: Vec::new(),
        };

        while self.peek().is_some() {
            let item = self.parse_item();
            if let Some(item) = item {
                program.items.push(item);
            } else {
                break;
            }
        }

        if self.diagnostics.is_empty() {
            Ok(program)
        } else {
            Err(self.diagnostics)
        }
    }

    fn parse_item(&mut self) -> Option<Item> {
        let comp = self.expect(TokenKind::Comp, "Expected `comp` at top-level Declaration")?;

        let identifier = self.expect(TokenKind::Identifier, "Expected identifier at top-level Binding")?;

        self.expect(TokenKind::Eq, "Expected `=` at top-level Binding")?;

        let expression = self.parse_expression()?;

        Some(Item {
            span: self.source.fromto(comp.span, expression.span),
            data: ItemData::Binding(Binding {
                name: identifier.span,
                expression: expression,
                type_annotation: None, // TODO
                mutable: false, // TODO
                phase: Comptime
            })
        })
    }

    fn parse_expression(&mut self) -> Option<Expression> {
        if self.peek_is(TokenKind::NumberLiteral) {
            let number = self.expect(TokenKind::NumberLiteral, "Expected number literal")?;
            return Some(Expression {
                span: number.span,
                data: ExpressionData::IntegerLiteral
            });
        }

        if self.peek_is(TokenKind::Fn) {
            let fn_ = self.expect(TokenKind::Fn, "Expected `fn`")?;

            self.expect(TokenKind::LParen, "Expected `)`")?;
            // TODO
            let rparen = self.expect(TokenKind::RParen, "Expected `)`")?;

            let return_type = if self.peek_is(TokenKind::RArrow) {
                self.expect(TokenKind::RArrow, "Expected `->`")?;
                self.parse_type_expression()?
            } else {
                TypeExpression {
                    span: rparen.span,
                    data: TypeExpressionData::Unit
                }
            };

            let body = self.parse_body()?;

            let end_span = body.span;

            let data = ExpressionData::Function(FunctionExpression {
                parameters: Vec::new(), // TODO,
                return_type,
                body,
            });

            return Some(Expression {
                span: self.source.fromto(fn_.span, end_span),
                data,
            })
        }

        None
    }

    fn parse_type_expression(&mut self) -> Option<TypeExpression> {
        let identifier = self.expect(TokenKind::Identifier, "Expected Type")?;
        Some(TypeExpression {
            span: identifier.span,
            data: TypeExpressionData::Identifier
        })
    }

    fn parse_body(&mut self) -> Option<Block> {
        let lcurly = self.expect(TokenKind::LCurly, "Expected `{` for block")?;

        let mut statements = Vec::new();

        while !self.peek_is(TokenKind::RCurly) {
            statements.push(self.parse_statement()?);
        }

        let rcurly = self.expect(TokenKind::RCurly, "Expected `}` for block")?;

        Some(Block {
            span: self.source.fromto(lcurly.span, rcurly.span),
            statements,
        })
    }

    fn parse_statement(&mut self) -> Option<Statement> {
        if self.peek_is(TokenKind::Return) {
            let return_ = self.expect(TokenKind::Return, "Expected `return`")?;

            let (expression, span) = if self.peek_is(TokenKind::Semi) {
                let semi = self.expect(TokenKind::Semi, "Expected `;`")?;
                (None, self.source.fromto(return_.span, semi.span))
            } else {
                let expression = self.parse_expression()?;
                let semi = self.expect(TokenKind::Semi, "Expected `;`")?;
                (Some(expression), self.source.fromto(return_.span, semi.span))
            };

            return Some(Statement {
                span,
                data: StatementData::Return(expression)
            })
        }

        None
    }

    // Utilities
    fn peek(&self) -> Option<Token> {
        if self.index < self.tokens.len() {
            Some(self.tokens[self.index])
        } else {
            None
        }
    }

    fn peek_is(&self, kind: TokenKind) -> bool {
        self.peek().is_some_and(|token| token.kind == kind)
    }

    fn consume(&mut self) -> Option<Token> {
        if let Some(token) = self.peek() {
            self.index += 1;
            Some(token)
        } else {
            None
        }
    }

    fn expect(&mut self, kind: TokenKind, description: &str) -> Option<Token> {
        let Some(token) = self.peek() else {
            let span = self.tokens.last()
                .map(|token| self.source.span(
                    token.span.end,
                    token.span.end,
                ))
                .unwrap_or(self.source.span(0, 0));

            self.diagnostics.push(Diagnostic::error(
                "Expected Token found EOF".to_owned(),
                span,
                description.to_owned(),
            ));

            return None;
        };

        if token.kind != kind {
            self.diagnostics.push(Diagnostic::error(
                "Unexpected Token".to_owned(),
                token.span,
                description.to_owned(),
            ));

            return None;
        }

        self.index += 1;
        Some(token)
    }
}