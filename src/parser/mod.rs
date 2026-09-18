use crate::diagnostics::Diagnostic;
use crate::parser::ast::Phase::Comptime;
use crate::parser::ast::{
    BinaryOperator, Binding, Block, ElseBranch, Expression, ExpressionData, FunctionExpression, If,
    Item, ItemData, Parameter, Phase, Program, Statement, StatementData, TypeExpression,
    TypeExpressionData, UnaryOperator, Visibility, While,
};
use crate::source::{SourceFile, Span};
use crate::tokenizer::{Token, TokenKind};
use std::fmt::Debug;

pub mod ast;

pub struct Parser<'src, 'tokens> {
    source: &'src SourceFile,
    tokens: &'tokens [Token],
    index: usize,
    diagnostics: Vec<Diagnostic>,
}

enum ParsedFunctionParameter {
    Named(Parameter),
    Unnamed(TypeExpression),
}

struct ParsedFunctionSignature {
    fn_span: Span,
    comptime_args: Vec<Parameter>,
    runtime_args: Vec<ParsedFunctionParameter>,
    return_type: TypeExpression,
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
        let mut program = Program { items: Vec::new() };

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
        let public_prefix = if self.peek_is(TokenKind::Pub) {
            Some(self.expect(TokenKind::Pub, "Expected `pub`")?)
        } else {
            None
        };

        let visibility = if public_prefix.is_some() {
            Visibility::Public
        } else {
            Visibility::Private
        };

        let comp = self.expect(TokenKind::Comp, "Expected `comp` at top-level Declaration")?;

        let start = public_prefix.map(|token| token.span).unwrap_or(comp.span);

        let identifier = self.expect(
            TokenKind::Identifier,
            "Expected identifier at top-level Binding",
        )?;

        let type_annotation = if self.peek_is(TokenKind::Colon) {
            self.expect(TokenKind::Colon, "Expected `:`")?;
            Some(self.parse_type_expression()?)
        } else {
            None
        };

        self.expect(TokenKind::Eq, "Expected `=` at top-level Binding")?;

        let expression = self.parse_expression()?;

        let semi = self.expect(TokenKind::Semi, "expected `;` after binding")?;

        Some(Item {
            span: self.source.fromto(start, semi.span),
            visibility,
            data: ItemData::Binding(Binding {
                name: identifier.span,
                expression,
                type_annotation,
                mutable: false,        // TODO
                phase: Comptime,
            }),
        })
    }

    fn parse_expression(&mut self) -> Option<Expression> {
        self.parse_binary_expression(0)
    }

    fn parse_prefix_expression(&mut self) -> Option<Expression> {
        if self.peek_is(TokenKind::Minus) {
            let minus = self.expect(TokenKind::Minus, "expected `-`")?;

            let operand = self.parse_prefix_expression()?;

            return Some(Expression {
                span: self.source.fromto(minus.span, operand.span),
                data: ExpressionData::Unary {
                    operator: UnaryOperator::Negate,
                    operand: Box::new(operand),
                },
            });
        }

        self.parse_postfix_expression()
    }

    fn parse_binary_expression(&mut self, minimum_precedence: u8) -> Option<Expression> {
        let mut lhs = self.parse_prefix_expression()?;

        loop {
            let Some((operator, precedence)) = self.peek_binary_operator() else {
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
                },
            };
        }

        Some(lhs)
    }

    fn parse_postfix_expression(&mut self) -> Option<Expression> {
        let mut expression = self.parse_primary()?;

        loop {
            let intrinsic = matches!(expression.data, ExpressionData::Intrinsic { .. });
            if self.peek_is(TokenKind::Dot) {
                expression = self.parse_member_expression(expression)?
            } else if self.peek_is(TokenKind::LParen) {
                expression = self.parse_call_expression(expression)?;
            } else if self.peek_is(TokenKind::LBrack) {
                expression = self.parse_index_expression(expression)?;
            } else if self.peek_is(TokenKind::LessThan)
                && (intrinsic || self.looks_like_specialization())
            {
                expression = self.parse_specialization_expression(expression)?;
            } else {
                break;
            }
        }

        Some(expression)
    }

    fn looks_like_specialization(&self) -> bool {
        if !self.peek_is(TokenKind::LessThan) {
            return false;
        }

        let mut index = self.index + 1;
        let mut nested_parens = 0usize;

        while let Some(token) = self.tokens.get(index) {
            match token.kind {
                TokenKind::LParen => nested_parens += 1,

                TokenKind::RParen if nested_parens > 0 => {
                    nested_parens -= 1;
                }

                TokenKind::GreaterThan if nested_parens == 0 => {
                    return self
                        .tokens
                        .get(index + 1)
                        .is_some_and(|next| next.kind == TokenKind::LParen);
                }

                TokenKind::Semi | TokenKind::LCurly | TokenKind::RCurly => return false,

                _ => {}
            }

            index += 1;
        }

        false
    }

    /// Temporary fix for something like make_adder<a > b> thinking it's actually make_adder<a> b >
    fn parse_specialization_expression(&mut self, callee: Expression) -> Option<Expression> {
        self.expect(TokenKind::LessThan, "Expected `<`")?;

        let mut arguments = Vec::new();

        while !self.peek_is(TokenKind::GreaterThan) {
            arguments.push(self.parse_prefix_expression()?);

            if self.peek_is(TokenKind::Comma) {
                self.consume();
            } else {
                break;
            }
        }

        let greater = self.expect(
            TokenKind::GreaterThan,
            "Expected `>` after compile-time arguments",
        )?;

        Some(Expression {
            span: self.source.fromto(callee.span, greater.span),
            data: ExpressionData::Specialize {
                callee: Box::new(callee),
                arguments,
            },
        })
    }

    fn parse_index_expression(&mut self, base: Expression) -> Option<Expression> {
        self.expect(TokenKind::LBrack, "Expected `[`")?;
        let index = self.parse_expression()?;
        let r_brack = self.expect(TokenKind::RBrack, "Expected `]`")?;

        Some(Expression {
            span: self.source.fromto(base.span, r_brack.span),
            data: ExpressionData::Index {
                base: Box::new(base),
                index: Box::new(index),
            },
        })
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

        let rparen = self.expect(TokenKind::RParen, "Expected `)` after call arguments")?;

        let span = self.source.fromto(callee.span, rparen.span);

        if let ExpressionData::Intrinsic { name } = &callee.data {
            if self.source.span_text(*name) == "import" {
                if arguments.len() != 1 {
                    self.diagnostics.push(Diagnostic::error(
                        "Invalid import arguments",
                        span,
                        "@import expects exactly one string literal",
                    ));

                    return None;
                }

                let argument = &arguments[0];

                if !matches!(&argument.data, ExpressionData::StringLiteral) {
                    self.diagnostics.push(Diagnostic::error(
                        "Import path must be a string literal",
                        argument.span,
                        "Computed import paths are not supported yet",
                    ));

                    return None;
                }

                return Some(Expression {
                    span,
                    data: ExpressionData::Import {
                        path: argument.span,
                    },
                });
            }
        }

        Some(Expression {
            span,
            data: ExpressionData::Call {
                callee: Box::new(callee),
                arguments,
            },
        })
    }

    fn peek_binary_operator(&self) -> Option<(BinaryOperator, u8)> {
        const PRECEDENCE_EQUALITY: u8 = 1;
        const PRECEDENCE_COMPARISON: u8 = 2;
        const PRECEDENCE_ADDITIVE: u8 = 3;
        const PRECEDENCE_MULTIPLICATIVE: u8 = 4;

        match self.peek() {
            Some(token) => Some(match (token.kind) {
                TokenKind::EqEq => (BinaryOperator::Equal, PRECEDENCE_EQUALITY),
                TokenKind::BangEq => (BinaryOperator::NotEqual, PRECEDENCE_EQUALITY),

                TokenKind::LessThanOrEqual => {
                    (BinaryOperator::LessThanOrEqual, PRECEDENCE_COMPARISON)
                }
                TokenKind::LessThan => (BinaryOperator::LessThan, PRECEDENCE_COMPARISON),
                TokenKind::GreaterThanOrEqual => {
                    (BinaryOperator::GreaterThanOrEqual, PRECEDENCE_COMPARISON)
                }
                TokenKind::GreaterThan => (BinaryOperator::GreaterThan, PRECEDENCE_COMPARISON),

                TokenKind::Plus => (BinaryOperator::Add, PRECEDENCE_ADDITIVE),
                TokenKind::Minus => (BinaryOperator::Subtract, PRECEDENCE_ADDITIVE),
                TokenKind::Star => (BinaryOperator::Multiply, PRECEDENCE_MULTIPLICATIVE),
                TokenKind::Slash => (BinaryOperator::Divide, PRECEDENCE_MULTIPLICATIVE),

                _ => return None,
            }),

            None => None,
        }
    }

    fn parse_primary(&mut self) -> Option<Expression> {
        if self.peek_is(TokenKind::At) {
            let at = self.expect(TokenKind::At, "Expected `@`")?;
            let name = self.expect(TokenKind::Identifier, "Expected identifier after `@`")?;

            return Some(Expression {
                span: self.source.fromto(at.span, name.span),
                data: ExpressionData::Intrinsic { name: name.span },
            });
        }

        if self.peek_is(TokenKind::StringLiteral) {
            let string_literal =
                self.expect(TokenKind::StringLiteral, "Expected string literal")?;
            return Some(Expression {
                span: string_literal.span,
                data: ExpressionData::StringLiteral,
            });
        }
        // [0; 50]
        if self.peek_is(TokenKind::LBrack) {
            let l_brack = self.expect(TokenKind::LBrack, "Expected `[`")?;
            let value = self.parse_expression()?;
            self.expect(TokenKind::Semi, "Expected `;`")?;
            let count = self.parse_expression()?;
            let r_brack = self.expect(TokenKind::RBrack, "Expected `]`")?;

            return Some(Expression {
                span: self.source.fromto(l_brack.span, r_brack.span),
                data: ExpressionData::ArrayRepeatInitialization {
                    value: Box::new(value),
                    size: Box::new(count),
                },
            });
        }

        if self.peek_is(TokenKind::True) {
            let true_ = self.expect(TokenKind::True, "Expected `true`")?;
            return Some(Expression {
                span: true_.span,
                data: ExpressionData::Boolean(true),
            });
        }

        if self.peek_is(TokenKind::False) {
            let false_ = self.expect(TokenKind::False, "Expected `false`")?;
            return Some(Expression {
                span: false_.span,
                data: ExpressionData::Boolean(false),
            });
        }

        if self.peek_is(TokenKind::NumberLiteral) {
            let number = self.expect(TokenKind::NumberLiteral, "Expected number literal")?;
            return Some(Expression {
                span: number.span,
                data: ExpressionData::IntegerLiteral,
            });
        }

        if self.peek_is(TokenKind::Fn) {
            return self.parse_function_expression();
        }

        if self.peek_is(TokenKind::Identifier) {
            let identifier = self.expect(TokenKind::Identifier, "Expected identifier")?;
            return Some(Expression {
                span: identifier.span,
                data: ExpressionData::Name,
            });
        }

        self.diagnostics.push(Diagnostic::error(
            "Unexpected start of expression",
            if let Some(token) = self.peek() {
                token.span
            } else {
                self.source.eof_span()
            },
            format!(
                "Expression may not start with `{}`",
                if let Some(token) = self.peek() {
                    token.kind.display()
                } else {
                    "EOF"
                }
            ),
        ));

        None
    }

    fn finish_function_type(&mut self, signature: ParsedFunctionSignature) -> Option<Expression> {
        if let Some(first) = signature.comptime_args.first() {
            self.diagnostics.push(Diagnostic::error(
                "Comptime parameters in function types are not supported yet",
                first.span,
                "remove the comptime parameter list",
            ));

            return None;
        }

        let mut parameters = Vec::new();

        for parameter in signature.runtime_args {
            match parameter {
                ParsedFunctionParameter::Unnamed(type_expression) => {
                    parameters.push(type_expression);
                }

                ParsedFunctionParameter::Named(parameter) => {
                    self.diagnostics.push(Diagnostic::error(
                        "Named function-type parameters are not supported yet",
                        parameter.span,
                        "write only the parameter type",
                    ));

                    return None;
                }
            }
        }

        let type_expression = TypeExpression {
            span: self
                .source
                .fromto(signature.fn_span, signature.return_type.span),
            data: TypeExpressionData::Function {
                parameters,
                return_type: Box::new(signature.return_type),
            },
        };

        Some(Expression {
            span: type_expression.span,
            data: ExpressionData::TypeValue(type_expression),
        })
    }

    fn finish_function_literal(
        &mut self,
        signature: ParsedFunctionSignature,
    ) -> Option<Expression> {
        let mut runtime_args = Vec::new();

        for parameter in signature.runtime_args {
            match parameter {
                ParsedFunctionParameter::Named(parameter) => {
                    runtime_args.push(parameter);
                }

                ParsedFunctionParameter::Unnamed(type_expression) => {
                    self.diagnostics.push(Diagnostic::error(
                        "Function implementation parameters require names",
                        type_expression.span,
                        "add an internal parameter name",
                    ));

                    return None;
                }
            }
        }

        let body = self.parse_block()?;
        let span = self.source.fromto(signature.fn_span, body.span);

        Some(Expression {
            span,
            data: ExpressionData::Function(FunctionExpression {
                comptime_args: signature.comptime_args,
                runtime_args,
                return_type: signature.return_type,
                body,
            }),
        })
    }

    fn parse_function_signature(&mut self) -> Option<ParsedFunctionSignature> {
        let fn_ = self.expect(TokenKind::Fn, "Expected `fn`")?;

        let comptime_args = if self.peek_is(TokenKind::LessThan) {
            self.expect(
                TokenKind::LessThan,
                "Expected `<` before comptime parameters",
            )?;

            let parameters = self.parse_parameters_until(TokenKind::GreaterThan)?;

            self.expect(
                TokenKind::GreaterThan,
                "Expected `>` after comptime parameters",
            )?;

            parameters
        } else {
            Vec::new()
        };

        self.expect(TokenKind::LParen, "Expected `(` before parameters")?;

        let runtime_args = self.parse_provisional_parameters()?;

        let rparen = self.expect(TokenKind::RParen, "Expected `)` after parameters")?;

        let return_type = if self.peek_is(TokenKind::RArrow) {
            self.consume();
            self.parse_type_expression()?
        } else {
            TypeExpression {
                span: rparen.span,
                data: TypeExpressionData::Unit,
            }
        };

        Some(ParsedFunctionSignature {
            fn_span: fn_.span,
            comptime_args,
            runtime_args,
            return_type,
        })
    }

    fn parse_named_parameter(&mut self) -> Option<Parameter> {
        let identifier = self.expect(TokenKind::Identifier, "Expected parameter name")?;

        self.expect(TokenKind::Colon, "Expected `:` after parameter name")?;

        let type_annotation = self.parse_type_expression()?;

        Some(Parameter {
            span: self.source.fromto(identifier.span, type_annotation.span),
            name: identifier.span,
            type_annotation,
        })
    }

    fn parse_parameters_until(&mut self, closing: TokenKind) -> Option<Vec<Parameter>> {
        let mut parameters = Vec::new();

        while !self.peek_is(closing) {
            parameters.push(self.parse_named_parameter()?);

            if self.peek_is(TokenKind::Comma) {
                self.consume();
            } else {
                break;
            }
        }

        Some(parameters)
    }

    fn parse_provisional_parameters(&mut self) -> Option<Vec<ParsedFunctionParameter>> {
        let mut parameters = Vec::new();

        while !self.peek_is(TokenKind::RParen) {
            let parameter = if self.peek_is(TokenKind::Identifier)
                && self.peek_offset_is(1, TokenKind::Colon)
            {
                ParsedFunctionParameter::Named(self.parse_named_parameter()?)
            } else {
                ParsedFunctionParameter::Unnamed(self.parse_type_expression()?)
            };

            parameters.push(parameter);

            if self.peek_is(TokenKind::Comma) {
                self.consume();
            } else {
                break;
            }
        }

        Some(parameters)
    }

    fn parse_function_expression(&mut self) -> Option<Expression> {
        let signature = self.parse_function_signature()?;

        if self.peek_is(TokenKind::LCurly) {
            self.finish_function_literal(signature)
        } else {
            self.finish_function_type(signature)
        }
    }

    fn parse_type_expression(&mut self) -> Option<TypeExpression> {
        if self.peek_is(TokenKind::Fn) {
            let fn_ = self.expect(TokenKind::Fn, "Expected `fn`")?;

            self.expect(TokenKind::LParen, "Expected `(`")?;
            let mut parameters = Vec::new();
            while !self.peek_is(TokenKind::RParen) {
                let arg_type = self.parse_type_expression()?;
                parameters.push(arg_type);

                if self.peek_is(TokenKind::Comma) {
                    self.expect(TokenKind::Comma, "Expected `,`")?;
                } else {
                    break;
                }
            }
            self.expect(TokenKind::RParen, "Expected `)`")?;
            self.expect(TokenKind::RArrow, "Expected `->`")?;

            let return_type = self.parse_type_expression()?;

            return Some(TypeExpression {
                span: self.source.fromto(fn_.span, return_type.span),
                data: TypeExpressionData::Function {
                    parameters,
                    return_type: Box::new(return_type),
                },
            });
        }

        if self.peek_is(TokenKind::LBrack) {
            let l_brack = self.expect(TokenKind::LBrack, "Expected `[`")?;
            let size = self.parse_expression()?;
            self.expect(TokenKind::RBrack, "Expected `]`")?;

            let base_type = self.parse_type_expression()?;

            return Some(TypeExpression {
                span: self.source.fromto(l_brack.span, base_type.span),
                data: TypeExpressionData::FixedArray {
                    size: Box::new(size),
                    base_type: Box::new(base_type),
                },
            });
        }

        let identifier = self.expect(TokenKind::Identifier, "Expected Type")?;
        Some(TypeExpression {
            span: identifier.span,
            data: TypeExpressionData::Identifier,
        })
    }

    fn parse_block(&mut self) -> Option<Block> {
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

    fn parse_let_statement(&mut self) -> Option<Statement> {
        let let_ = self.expect(TokenKind::Let, "Expected let")?;

        let mutable = if self.peek_is(TokenKind::Mut) {
            self.expect(TokenKind::Mut, "Expected `mut`")?;
            true
        } else {
            false
        };

        let name = self.expect(TokenKind::Identifier, "Expected binding name after `let`")?;

        let type_annotation = if self.peek_is(TokenKind::Colon) {
            let _colon = self.expect(TokenKind::Colon, "Expected `:`")?;
            let type_annotation = self.parse_type_expression()?;
            Some(type_annotation)
        } else {
            None
        };

        self.expect(TokenKind::Eq, "Expected `=` after binding name")?;

        let expression = self.parse_expression()?;

        let semi = self.expect(TokenKind::Semi, "Expected `;` after local binding")?;

        Some(Statement {
            span: self.source.fromto(let_.span, semi.span),
            data: StatementData::Binding(Binding {
                phase: Phase::Runtime,
                mutable,
                name: name.span,
                type_annotation, // todo
                expression,
            }),
        })
    }

    fn parse_return_statement(&mut self) -> Option<Statement> {
        let return_ = self.expect(TokenKind::Return, "Expected `return`")?;

        let (expression, span) = if self.peek_is(TokenKind::Semi) {
            let semi = self.expect(TokenKind::Semi, "Expected `;`")?;
            (None, self.source.fromto(return_.span, semi.span))
        } else {
            let expression = self.parse_expression()?;
            let semi = self.expect(TokenKind::Semi, "Expected `;`")?;
            (
                Some(expression),
                self.source.fromto(return_.span, semi.span),
            )
        };

        Some(Statement {
            span,
            data: StatementData::Return(expression),
        })
    }

    fn parse_statement(&mut self) -> Option<Statement> {
        match self.peek().map(|token| token.kind) {
            Some(TokenKind::Let) => self.parse_let_statement(),
            Some(TokenKind::Return) => self.parse_return_statement(),
            Some(TokenKind::While) => self.parse_while_statement(),
            Some(TokenKind::If) => self.parse_if_statement(),
            Some(TokenKind::Identifier) => {
                let target_or_expression = self.parse_expression()?;

                if self.peek_is(TokenKind::Eq) {
                    self.expect(TokenKind::Eq, "Expected `=` for assignment")?;

                    let value = self.parse_expression()?;
                    let semi = self.expect(TokenKind::Semi, "Expected `;` after assignment")?;

                    Some(Statement {
                        span: self.source.fromto(target_or_expression.span, semi.span),
                        data: StatementData::Assignment {
                            target: target_or_expression,
                            expression: value,
                        },
                    })
                } else {
                    let semi =
                        self.expect(TokenKind::Semi, "Expected `;` after statement expression")?;

                    Some(Statement {
                        span: self.source.fromto(target_or_expression.span, semi.span),
                        data: StatementData::Expression(target_or_expression),
                    })
                }
            }

            _ => {
                self.diagnostics.push(Diagnostic::error(
                    "Unexpected start of Statement",
                    match self.peek() {
                        Some(token) => token.span,
                        None => self.source.eof_span(),
                    },
                    format!(
                        "`{}` is not a valid statement starter",
                        match self.peek() {
                            Some(token) => token.kind.display(),
                            None => "EOF",
                        }
                    ),
                ));

                None
            }
        }
    }

    fn parse_while_statement(&mut self) -> Option<Statement> {
        let while_ = self.expect(TokenKind::While, "Expected `while`")?;

        let condition = self.parse_expression()?;
        let block = self.parse_block()?;

        Some(Statement {
            span: self.source.fromto(while_.span, block.span),
            data: StatementData::While(While {
                condition,
                while_block: block,
            }),
        })
    }

    fn parse_if_statement(&mut self) -> Option<Statement> {
        let if_ = self.expect(TokenKind::If, "Expected `if`")?;

        let condition = self.parse_expression()?;
        let body = self.parse_block()?;

        let (else_if, end_span): (Option<ElseBranch>, Span) = if self.peek_is(TokenKind::Else) {
            self.expect(TokenKind::Else, "Expected `else`")?;
            if self.peek_is(TokenKind::If) {
                let else_if = self.parse_if_statement()?;
                let span = else_if.span;
                (Some(ElseBranch::ElseIf(Box::new(else_if))), span)
            } else {
                let body = self.parse_block()?;
                let span = body.span;
                (Some(ElseBranch::Else(body)), span)
            }
        } else {
            (None, body.span)
        };

        Some(Statement {
            span: self.source.fromto(if_.span, end_span),
            data: StatementData::If(If {
                condition,
                then_block: body,
                else_: else_if,
            }),
        })
    }

    // Utilities
    fn peek(&self) -> Option<Token> {
        self.peek_offset(0)
    }

    fn peek_offset(&self, offset: usize) -> Option<Token> {
        let index = self.index + offset;
        if index < self.tokens.len() {
            Some(self.tokens[index])
        } else {
            None
        }
    }

    fn peek_is(&self, kind: TokenKind) -> bool {
        self.peek().is_some_and(|token| token.kind == kind)
    }

    fn peek_offset_is(&self, offset: usize, kind: TokenKind) -> bool {
        self.peek_offset(offset)
            .is_some_and(|token| token.kind == kind)
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
            let span = self
                .tokens
                .last()
                .map(|token| self.source.span(token.span.end, token.span.end))
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

    fn parse_member_expression(&mut self, base: Expression) -> Option<Expression> {
        self.expect(TokenKind::Dot, "Expected `.`")?;

        let name = self.expect(TokenKind::Identifier, "Expected member name after `.`")?;

        Some(Expression {
            span: self.source.fromto(base.span, name.span),
            data: ExpressionData::Member {
                base: Box::new(base),
                name: name.span,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceId;
    use crate::tokenizer::Tokenizer;
    use ariadne::Source;

    fn parse_source(text: &str) -> (SourceFile, Program) {
        let source = SourceFile {
            id: SourceId(0),
            name: "test.fl".to_owned(),
            source: Source::from(text.to_owned()),
        };

        let tokens = Tokenizer::new(&source)
            .tokenize()
            .expect("tokenization should succeed");

        let program = Parser::new(&source, &tokens)
            .parse()
            .expect("parsing should succeed");

        (source, program)
    }

    fn assert_string_binding(input: &str, expected_literal: &str) {
        let (source, program) = parse_source(input);

        assert_eq!(program.items.len(), 1);

        let ItemData::Binding(binding) = &program.items[0].data;

        assert!(matches!(
            binding.expression.data,
            ExpressionData::StringLiteral
        ));

        assert_eq!(source.span_text(binding.expression.span), expected_literal);
    }

    fn parse_error(text: &str) -> Vec<Diagnostic> {
        let source = SourceFile {
            id: SourceId(0),
            name: "test.fl".to_owned(),
            source: Source::from(text.to_owned()),
        };

        let tokens = Tokenizer::new(&source)
            .tokenize()
            .expect("tokenization should succeed");

        Parser::new(&source, &tokens)
            .parse()
            .expect_err("parsing should fail")
    }

    #[test]
    fn parses_string_literal_expression() {
        assert_string_binding("comp name = \"getchar\";", "\"getchar\"");
    }

    #[test]
    fn parses_empty_string_literal_expression() {
        assert_string_binding("comp name = \"\";", "\"\"");
    }

    #[test]
    fn parses_utf8_string_literal_expression() {
        assert_string_binding("comp name = \"héllo 世界\";", "\"héllo 世界\"");
    }

    #[test]
    fn parses_zero_parameter_function_type_value() {
        let (source, program) = parse_source("comp Signature = fn() -> i64;");

        let ItemData::Binding(binding) = &program.items[0].data;

        let ExpressionData::TypeValue(type_expression) = &binding.expression.data else {
            panic!("expected a function-type value");
        };

        let TypeExpressionData::Function {
            parameters,
            return_type,
        } = &type_expression.data
        else {
            panic!("expected a function type");
        };

        assert!(parameters.is_empty());
        assert_eq!(source.span_text(return_type.span), "i64");
        assert_eq!(source.span_text(binding.expression.span), "fn() -> i64");
    }

    #[test]
    fn parses_one_parameter_function_type_value() {
        let (source, program) = parse_source("comp Signature = fn(i32) -> i64;");

        let ItemData::Binding(binding) = &program.items[0].data;

        let ExpressionData::TypeValue(type_expression) = &binding.expression.data else {
            panic!("expected a function-type value");
        };

        let TypeExpressionData::Function {
            parameters,
            return_type,
        } = &type_expression.data
        else {
            panic!("expected a function type");
        };

        assert_eq!(parameters.len(), 1);
        assert_eq!(source.span_text(parameters[0].span), "i32");
        assert_eq!(source.span_text(return_type.span), "i64");
    }

    #[test]
    fn parses_multiple_parameter_function_type_value() {
        let (source, program) = parse_source("comp Signature = fn(i32, i64) -> i64;");

        let ItemData::Binding(binding) = &program.items[0].data;

        let ExpressionData::TypeValue(type_expression) = &binding.expression.data else {
            panic!("expected a function-type value");
        };

        let TypeExpressionData::Function {
            parameters,
            return_type,
        } = &type_expression.data
        else {
            panic!("expected a function type");
        };

        assert_eq!(parameters.len(), 2);
        assert_eq!(source.span_text(parameters[0].span), "i32");
        assert_eq!(source.span_text(parameters[1].span), "i64");
        assert_eq!(source.span_text(return_type.span), "i64");
    }

    #[test]
    fn parses_nested_function_type_value() {
        let (source, program) = parse_source("comp Signature = fn(fn(i64) -> i32) -> i64;");

        let ItemData::Binding(binding) = &program.items[0].data;

        let ExpressionData::TypeValue(type_expression) = &binding.expression.data else {
            panic!("expected a function-type value");
        };

        let TypeExpressionData::Function {
            parameters,
            return_type,
        } = &type_expression.data
        else {
            panic!("expected an outer function type");
        };

        assert_eq!(parameters.len(), 1);
        assert_eq!(source.span_text(parameters[0].span), "fn(i64) -> i32");
        assert_eq!(source.span_text(return_type.span), "i64");

        assert!(matches!(
            &parameters[0].data,
            TypeExpressionData::Function { .. }
        ));
    }

    #[test]
    fn still_parses_named_function_literal() {
        let (source, program) = parse_source(
            r#"
          comp identity = fn(value: i64) -> i64 {
              return value;
          };
          "#,
        );

        let ItemData::Binding(binding) = &program.items[0].data;

        let ExpressionData::Function(function) = &binding.expression.data else {
            panic!("expected a function literal");
        };

        assert_eq!(function.runtime_args.len(), 1);
        assert_eq!(source.span_text(function.runtime_args[0].name), "value");
    }

    #[test]
    fn rejects_unnamed_function_literal_parameter() {
        let diagnostics = parse_error(
            r#"
          comp invalid = fn(i64) -> i64 {
              return 1;
          };
          "#,
        );

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("Function implementation parameters require names")
        }));
    }

    #[test]
    fn rejects_named_function_type_parameter_for_now() {
        let diagnostics = parse_error("comp Signature = fn(value: i64) -> i64;");

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("Named function-type parameters are not supported yet")
        }));
    }

    #[test]
    fn parses_function_type_annotation() {
        let (source, program) = parse_source(
            r#"
          comp higher_order = fn<
              callback: fn(i64) -> i64
          >() -> i64 {
              return 0;
          };
          "#,
        );

        let ItemData::Binding(binding) = &program.items[0].data;

        let ExpressionData::Function(function) = &binding.expression.data else {
            panic!("expected a function template");
        };

        assert_eq!(function.comptime_args.len(), 1);

        assert!(matches!(
            &function.comptime_args[0].type_annotation.data,
            TypeExpressionData::Function { .. }
        ));

        assert_eq!(
            source.span_text(function.comptime_args[0].type_annotation.span),
            "fn(i64) -> i64"
        );
    }

    #[test]
    fn parses_bare_intrinsic() {
        let (source, program) = parse_source("comp x = @trap;");

        let ItemData::Binding(binding) = &program.items[0].data;

        let ExpressionData::Intrinsic { name } = &binding.expression.data else {
            panic!("expected an intrinsic");
        };

        assert_eq!(source.span_text(*name), "trap");
        assert_eq!(source.span_text(binding.expression.span), "@trap");
    }

    #[test]
    fn parses_intrinsic_runtime_call() {
        let (source, program) = parse_source("comp x = @trap();");

        let ItemData::Binding(binding) = &program.items[0].data;

        let ExpressionData::Call { callee, arguments } = &binding.expression.data else {
            panic!("expected a call");
        };

        assert!(arguments.is_empty());

        let ExpressionData::Intrinsic { name } = &callee.data else {
            panic!("expected the call target to be an intrinsic");
        };

        assert_eq!(source.span_text(*name), "trap");
        assert_eq!(source.span_text(binding.expression.span), "@trap()");
    }

    #[test]
    fn parses_specialized_extern_intrinsic() {
        let (source, program) = parse_source(r#"comp x = @extern<"C", "getchar", fn() -> i32>;"#);

        let ItemData::Binding(binding) = &program.items[0].data;

        let ExpressionData::Specialize { callee, arguments } = &binding.expression.data else {
            panic!("expected intrinsic specialization");
        };

        let ExpressionData::Intrinsic { name } = &callee.data else {
            panic!("expected an intrinsic specialization target");
        };

        assert_eq!(source.span_text(*name), "extern");
        assert_eq!(arguments.len(), 3);

        assert!(matches!(arguments[0].data, ExpressionData::StringLiteral));
        assert!(matches!(arguments[1].data, ExpressionData::StringLiteral));
        assert!(matches!(arguments[2].data, ExpressionData::TypeValue(_)));

        assert_eq!(source.span_text(arguments[0].span), "\"C\"");
        assert_eq!(source.span_text(arguments[1].span), "\"getchar\"");
        assert_eq!(source.span_text(arguments[2].span), "fn() -> i32");
    }

    #[test]
    fn parses_runtime_call_after_intrinsic_specialization() {
        let (source, program) = parse_source(r#"comp x = @foo<"compile-time">(123);"#);

        let ItemData::Binding(binding) = &program.items[0].data;

        let ExpressionData::Call {
            callee,
            arguments: runtime_arguments,
        } = &binding.expression.data
        else {
            panic!("expected a runtime call");
        };

        assert_eq!(runtime_arguments.len(), 1);
        assert_eq!(source.span_text(runtime_arguments[0].span), "123");

        let ExpressionData::Specialize {
            callee,
            arguments: comptime_arguments,
        } = &callee.data
        else {
            panic!("expected the call target to be specialized");
        };

        assert_eq!(comptime_arguments.len(), 1);
        assert_eq!(
            source.span_text(comptime_arguments[0].span),
            "\"compile-time\""
        );

        let ExpressionData::Intrinsic { name } = &callee.data else {
            panic!("expected an intrinsic specialization target");
        };

        assert_eq!(source.span_text(*name), "foo");
        assert_eq!(
            source.span_text(binding.expression.span),
            r#"@foo<"compile-time">(123)"#
        );
    }

    #[test]
    fn rejects_intrinsic_without_name() {
        let diagnostics = parse_error("comp x = @;");

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .primary
                .text
                .contains("Expected identifier after `@`")
        }));
    }

    #[test]
    fn rejects_intrinsic_specialization_without_name() {
        let diagnostics = parse_error(r#"comp x = @<"C">;"#);

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .primary
                .text
                .contains("Expected identifier after `@`")
        }));
    }

    #[test]
    fn parses_chained_member_call() {
        use crate::source::SourceFileManager;
        use crate::tokenizer::Tokenizer;

        let mut sources = SourceFileManager::new();
        let id = sources.add_file("test.fl".into(), "comp result = std.io.print(42);".into());
        let source = sources.get_file(id);

        let tokens = Tokenizer::new(source)
            .tokenize()
            .expect("tokenization should succeed");

        let program = Parser::new(source, &tokens)
            .parse()
            .expect("parsing should succeed");

        let ItemData::Binding(binding) = &program.items[0].data;

        let ExpressionData::Call { callee, arguments } = &binding.expression.data else {
            panic!("expected a call");
        };

        assert_eq!(arguments.len(), 1);
        assert_eq!(source.span_text(arguments[0].span), "42");

        let ExpressionData::Member { base, name } = &callee.data else {
            panic!("expected .print");
        };

        assert_eq!(source.span_text(*name), "print");
        assert_eq!(source.span_text(callee.span), "std.io.print");

        let ExpressionData::Member { base, name } = &base.data else {
            panic!("expected .io");
        };

        assert_eq!(source.span_text(*name), "io");
        assert!(matches!(&base.data, ExpressionData::Name));
        assert_eq!(source.span_text(base.span), "std");
    }

    #[test]
    fn parses_literal_import() {
        use crate::source::SourceFileManager;
        use crate::tokenizer::Tokenizer;

        let mut sources = SourceFileManager::new();
        let id = sources.add_file(
            "main.fl".into(),
            r#"comp library = @import("./library.fl");"#.into(),
        );

        let source = sources.get_file(id);
        let tokens = Tokenizer::new(source)
            .tokenize()
            .expect("tokenization should succeed");

        let program = Parser::new(source, &tokens)
            .parse()
            .expect("parsing should succeed");

        let ItemData::Binding(binding) = &program.items[0].data;

        let ExpressionData::Import { path } = &binding.expression.data else {
            panic!("expected an import expression");
        };

        assert_eq!(
          source.span_text(*path),
          r#""./library.fl""#,
      );

        assert_eq!(
          source.span_text(binding.expression.span),
          r#"@import("./library.fl")"#,
      );
    }

    #[test]
    fn rejects_invalid_import_arguments() {
        use crate::source::SourceFileManager;
        use crate::tokenizer::Tokenizer;

        for (text, expected_message) in [
            (
                "comp library = @import();",
                "Invalid import arguments",
            ),
            (
                r#"comp library = @import("./a.fl", "./b.fl");"#,
                "Invalid import arguments",
            ),
            (
                "comp library = @import(path);",
                "Import path must be a string literal",
            ),
            (
                "comp library = @import(42);",
                "Import path must be a string literal",
            ),
        ] {
            let mut sources = SourceFileManager::new();
            let id = sources.add_file("main.fl".into(), text.into());
            let source = sources.get_file(id);

            let tokens = Tokenizer::new(source)
                .tokenize()
                .expect("tokenization should succeed");

            let diagnostics = Parser::new(source, &tokens)
                .parse()
                .expect_err("invalid import should be rejected");

            assert!(
                diagnostics.iter().any(|diagnostic| {
                    diagnostic.message == expected_message
                }),
                "expected {expected_message:?} for {text:?}, got: {diagnostics:#?}",
            );
        }
    }
}
