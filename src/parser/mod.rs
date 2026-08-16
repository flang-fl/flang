use std::cmp::min;
use crate::diagnostics::Diagnostic;
use crate::parser::ast::Phase::Comptime;
use crate::parser::ast::{BinaryOperator, Binding, Block, Expression, ExpressionData, FunctionExpression, Item, ItemData, Parameter, Phase, Program, Statement, StatementData, TypeExpression, TypeExpressionData};
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

        let semi = self.expect(
            TokenKind::Semi,
            "expected `;` after binding",
        )?;

        Some(Item {
            span: self.source.fromto(comp.span, semi.span),
            data: ItemData::Binding(Binding {
                name: identifier.span,
                expression,
                type_annotation: None, // TODO
                mutable: false, // TODO
                phase: Comptime
            })
        })
    }

    fn parse_expression(&mut self) -> Option<Expression> {
        self.parse_binary_expression(0)
    }

    fn parse_binary_expression(
        &mut self,
        minimum_precedence: u8,
    ) -> Option<Expression> {
        let mut lhs = self.parse_postfix_expression()?;

        loop {
            let Some((operator, precedence)) =
              self.peek_binary_operator()
            else {
                break;
            };

            if precedence < minimum_precedence {
                break;
            }

            self.consume();

            let rhs = self.parse_binary_expression(precedence + 1)?;
            let span = self.source.fromto(lhs.span, rhs.span);

            lhs = Expression {
                span,
                data: ExpressionData::Binary {
                    lhs: Box::new(lhs),
                    operator,
                    rhs: Box::new(rhs),
                }
            };
        }

        Some(lhs)
    }

    fn parse_postfix_expression(&mut self) -> Option<Expression> {
        let mut expression = self.parse_primary()?;

        while self.peek_is(TokenKind::LParen) {
            expression = self.parse_call_expression(expression)?;
        }

        Some(expression)
    }

    fn parse_call_expression(&mut self, callee: Expression) -> Option<Expression> {
        self.expect(TokenKind::LParen, "Expected `(`")?;

        let mut arguments = Vec::new();

        while !self.peek_is(TokenKind::RParen) {
            arguments.push(self.parse_expression()?);

            if self.peek_is(TokenKind::Comma) {
                self.consume();
            } else {
                break;
            }
        }

        let rparen = self.expect(
            TokenKind::RParen,
            "Expected `)` after call arguments"
        )?;

        let span = self.source.fromto(callee.span, rparen.span);

        Some(Expression {
            span,
            data: ExpressionData::Call {
                callee: Box::new(callee),
                arguments,
            }
        })
    }

    fn peek_binary_operator(&self) -> Option<(BinaryOperator, u8)> {
        match self.peek() {
            Some(token) => Some(match(token.kind) {
                TokenKind::Plus => (BinaryOperator::Add, 1),
                TokenKind::Minus => (BinaryOperator::Subtract, 1),
                TokenKind::Star => (BinaryOperator::Multiply, 2),
                TokenKind::Slash => (BinaryOperator::Divide, 2),

                _ => return None,
            }),

            None => None
        }
    }

    fn parse_primary(&mut self) -> Option<Expression> {
        if self.peek_is(TokenKind::True) {
            let true_ = self.expect(TokenKind::True, "Expected `true`")?;
            return Some(Expression {
                span: true_.span,
                data: ExpressionData::Boolean(true)
            });
        }
        
        if self.peek_is(TokenKind::False) {
            let false_ = self.expect(TokenKind::False, "Expected `false`")?;
            return Some(Expression {
                span: false_.span,
                data: ExpressionData::Boolean(false)
            })
        }
        
        if self.peek_is(TokenKind::NumberLiteral) {
            let number = self.expect(TokenKind::NumberLiteral, "Expected number literal")?;
            return Some(Expression {
                span: number.span,
                data: ExpressionData::IntegerLiteral
            });
        }

        if self.peek_is(TokenKind::Fn) {
            return self.parse_function_literal();
        }

        if self.peek_is(TokenKind::Identifier) {
            let identifier = self.expect(TokenKind::Identifier, "Expected identifier")?;
            return Some(Expression {
                span: identifier.span,
                data: ExpressionData::Name,
            });
        }

        None
    }

    fn parse_function_literal(&mut self) -> Option<Expression> {
        let fn_ = self.expect(TokenKind::Fn, "Expected `fn`")?;

        self.expect(TokenKind::LParen, "Expected `)`")?;

        let mut parameters = Vec::new();
        while !self.peek_is(TokenKind::RParen) {
            let identifier = self.expect(TokenKind::Identifier, "Expected identifier of function parameters")?;
            self.expect(TokenKind::Colon, "Expected `:`")?;
            let type_ = self.parse_type_expression()?;

            parameters.push(Parameter {
                span: self.source.fromto(identifier.span, type_.span),
                name: identifier.span,
                type_annotation: type_,
            });

            if self.peek_is(TokenKind::Comma) {
                self.expect(TokenKind::Comma, "Expected `,`")?;
            } else {
                break;
            }
        }

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
            parameters,
            return_type,
            body,
        });

        Some(Expression {
            span: self.source.fromto(fn_.span, end_span),
            data,
        })
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
        if self.peek_is(TokenKind::Let) {
            let let_ = self.expect(TokenKind::Let, "Expected let")?;
            let name = self.expect(
                TokenKind::Identifier,
                "Expected binding name after `let`"
            )?;

            self.expect(TokenKind::Eq, "Expected `=` after binding name")?;

            let expression = self.parse_expression()?;

            let semi = self.expect(
                TokenKind::Semi,
                "Expected `;` after local binding"
            )?;

            Some(Statement {
                span: self.source.fromto(let_.span, semi.span),
                data: StatementData::Binding(Binding {
                    phase: Phase::Runtime,
                    mutable: false, // todo
                    name: name.span,
                    type_annotation: None, // todo
                    expression,
                }),
            })
        } else if self.peek_is(TokenKind::Return) {
            let return_ = self.expect(TokenKind::Return, "Expected `return`")?;

            let (expression, span) = if self.peek_is(TokenKind::Semi) {
                let semi = self.expect(TokenKind::Semi, "Expected `;`")?;
                (None, self.source.fromto(return_.span, semi.span))
            } else {
                let expression = self.parse_expression()?;
                let semi = self.expect(TokenKind::Semi, "Expected `;`")?;
                (Some(expression), self.source.fromto(return_.span, semi.span))
            };

            Some(Statement {
                span,
                data: StatementData::Return(expression)
            })
        } else {
            None
        }
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