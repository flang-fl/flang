use crate::diagnostics::Diagnostic;
use crate::parser::ast::{Binding, Block, Expression, ExpressionData, Item, ItemData, Program, Statement, StatementData, TypeExpression, TypeExpressionData};
use crate::semantic::hir::{HirBinding, HirBlock, HirExpression, HirExpressionData, HirFunctionExpression, HirProgram, HirStatement, HirStatementData};
use crate::semantic::symbols::{Environment, Symbol, SymbolId, SymbolKind, SymbolTable};
use crate::semantic::types::Type;
use crate::source::{SourceFile, Span};

mod symbols;
mod types;
mod hir;

pub struct Analyzer<'src> {
    source: &'src SourceFile,
    symbols: SymbolTable,
    environment: Environment,
    diagnostics: Vec<Diagnostic>,
}

impl<'src> Analyzer<'src> {
    pub fn new(source: &'src SourceFile) -> Self {
        let mut symbols = SymbolTable::new();
        let mut environment = Environment::new();

        let i64_id = symbols.insert(Symbol {
            name: "i64".to_owned(),
            kind: SymbolKind::BuiltinType(Type::I64),
            declaration_span: None,
            type_: Type::Type
        });

        environment.define("i64".to_owned(), i64_id);

        let unit_id = symbols.insert(Symbol {
            name: "Unit".to_owned(),
            kind: SymbolKind::BuiltinType(Type::Unit),
            declaration_span: None,
            type_: Type::Type
        });

        environment.define("unit".to_owned(), unit_id);


        Self {
            source,
            symbols,
            environment,
            diagnostics: Vec::new(),
        }
    }

    pub fn analyze(
        &mut self,
        program: Program,
    ) -> Result<HirProgram, Vec<Diagnostic>> {
        let mut collected_bindings = Vec::new();
        for item in program.items.iter() {
            let Item { span: item_span, data } = item;
            match data {
                ItemData::Binding(binding) => {
                    let name = self.source.span_text(binding.name);

                    if let Some(_) = self.environment.lookup(name) {
                        self.diagnostics.push(Diagnostic::error(
                            "Duplicate binding found".to_owned(),
                            *item_span,
                            "Evil :(".to_owned(),
                        ));

                        continue;
                    }

                    let symbol = Symbol {
                        name: name.to_owned(),
                        declaration_span: Some(binding.name),
                        kind: SymbolKind::Binding {
                            phase: binding.phase,
                            mutable: binding.mutable,
                        },
                        type_: Type::Unknown
                    };

                    let symbol_id = self.symbols.insert(symbol);
                    self.environment.define(name.to_owned(), symbol_id);
                    collected_bindings.push((
                        binding,
                        *item_span,
                        symbol_id,
                    ));
                }
            }
        }

        let mut hir_bindings = Vec::new();

        for (binding, item_span, symbol_id) in collected_bindings {
            if let Some(hir_binding) = self.analyze_binding(
                binding,
                item_span,
                symbol_id
            ) {
                hir_bindings.push(hir_binding);
            }
        }

        if self.diagnostics.is_empty() {
            Ok(HirProgram {
                bindings: hir_bindings
            })
        } else {
            Err(self.diagnostics.clone())
        }
    }

    fn analyze_binding(&mut self, binding: &Binding, span: Span, symbol_id: SymbolId) -> Option<HirBinding> {
        let type_annotation = binding.type_annotation
            .as_ref()
            .map(|type_expression|
                self.resolve_type_expression(type_expression)
            );

        let hir_expression = self.analyze_expression(
            &binding.expression,
            type_annotation.as_ref()
        );

        self.symbols.get_mut(symbol_id).type_ =
            hir_expression.type_.clone();

        let hir_binding = HirBinding {
            symbol: symbol_id,
            span,
            phase: binding.phase,
            mutable: binding.mutable,
            expression: hir_expression,
        };

        Some(hir_binding)
    }

    fn resolve_type_expression(
        &mut self,
        expression: &TypeExpression
    ) -> Type {
        match expression.data {
            TypeExpressionData::Unit => Type::Unit,
            TypeExpressionData::Identifier => {
                let name = self.source.span_text(expression.span);

                let Some(symbol_id) = self.environment.lookup(name) else {
                    self.diagnostics.push(Diagnostic::error(
                        "Unknown Type".to_owned(),
                        expression.span,
                        ":(".to_owned()
                    ));
                    return Type::Error;
                };

                match &self.symbols.get(symbol_id).kind {
                    SymbolKind::BuiltinType(type_) => type_.clone(),
                    _ => {
                        self.diagnostics.push(Diagnostic::error(
                            "Expected a Type found a Value".to_owned(),
                            expression.span,
                            ":(".to_owned()
                        ));
                        Type::Error
                    }
                }
            }
        }
    }

    fn analyze_expression(
        &mut self,
        expression: &Expression,
        expected: Option<&Type>
    ) -> HirExpression {
        match &expression.data {
            ExpressionData::Function(function) => {
                let return_type = self.resolve_type_expression(&function.return_type);

                let body = self.analyze_block(
                    &function.body,
                    &return_type
                );

                let function_type = Type::Function {
                    return_type: Box::new(return_type.clone()),
                };

                let hir_function = HirFunctionExpression {
                    parameters: Vec::new(), // TODO function parameters
                    return_type,
                    body,
                };

                HirExpression {
                    type_: function_type,
                    span: expression.span,
                    data: HirExpressionData::Function(hir_function)
                }
            },
            ExpressionData::IntegerLiteral => {
                if let Some(expected) = expected {
                    if *expected != Type::I64 {
                        self.diagnostics.push(Diagnostic::error(
                            "Type mismatch".to_owned(),
                            expression.span,
                            format!(
                                "Expected an expression of type `{:?}` but got a number",
                                expected
                            )
                        ));
                        return HirExpression {
                            type_: Type::Error,
                            span: expression.span,
                            data: HirExpressionData::Error
                        };
                    }
                }


                let parsed = self.source.span_text(expression.span)
                    .parse::<i64>();

                let Ok(parsed) = parsed else {
                    self.diagnostics.push(Diagnostic::error(
                        "Number overflow".to_owned(),
                        expression.span,
                        ":(".to_owned()
                    ));
                    return HirExpression {
                        type_: Type::Error,
                        span: expression.span,
                        data: HirExpressionData::Error
                    };
                };

                HirExpression {
                    type_: Type::I64,
                    span: expression.span,
                    data: HirExpressionData::Integer(parsed)
                }
            }
            ExpressionData::Name => todo!(),
        }
    }

    fn analyze_block(
        &mut self,
        block: &Block,
        return_type: &Type,
    ) -> HirBlock {
        self.environment.push_scope();

        let statements = block
            .statements
            .iter()
            .map(|statement| {
                self.analyze_statement(statement, return_type)
            })
            .collect();

        self.environment.pop_scope();

        HirBlock {
            statements,
            span: block.span,
        }
    }

    fn analyze_statement(&mut self, statement: &Statement, return_type: &Type) -> HirStatement {
        match &statement.data {
            StatementData::Return(expression) => {
                let Some(expression) = expression else {
                    return if *return_type == Type::Unit {
                        HirStatement {
                            span: statement.span,
                            data: HirStatementData::Return(None),
                        }
                    } else {
                        self.diagnostics.push(Diagnostic::error(
                            "Return without value in function with non-unit return type".to_owned(),
                            statement.span,
                            format!("Expected a `{:?}`", return_type)
                        ));

                        HirStatement {
                            span: statement.span,
                            data: HirStatementData::Return(None),
                        }
                    }
                };

                let hir_expression = self.analyze_expression(
                    &expression, Some(return_type)
                );

                HirStatement {
                    span: statement.span,
                    data: HirStatementData::Return(Some(hir_expression)),
                }
            },
        }
    }
}