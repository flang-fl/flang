use crate::diagnostics::Diagnostic;
use crate::parser::ast::{BinaryOperator, Phase};
use crate::semantic::SemanticProgram;
use crate::semantic::hir::{HirExpression, HirExpressionData, HirFunctionExpression, HirProgram};
use crate::semantic::symbols::{SymbolId, SymbolKind, SymbolTable};
use crate::semantic::types::Type;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum ComptimeValue {
    I64(i64),
    Function(FunctionId),
    Type(Type),
    Error,
}

#[derive(Debug)]
pub struct EvaluatedProgram {
    pub symbols: SymbolTable,
    pub values: ValueStore,
    pub functions: FunctionStore,
    pub hir: HirProgram
}

pub struct Evaluator {
    values: ValueStore,
    functions: FunctionStore,
    diagnostics: Vec<Diagnostic>
}

impl Evaluator {
    pub fn new() -> Self {
        Self {
            values: ValueStore { map: HashMap::new() },
            functions: FunctionStore { functions: Vec::new() },
            diagnostics: Vec::new(),
        }
    }

    pub fn evaluate(mut self, program: SemanticProgram)
        -> Result<EvaluatedProgram, Vec<Diagnostic>>
    {
        let symbols = program.symbols;
        for (id, value) in symbols.symbols.iter().enumerate() {
            let id = SymbolId(id as u32);

            match &value.kind {
                SymbolKind::BuiltinType(type_) => {
                    self.values.insert(
                        id,
                        ComptimeValue::Type(type_.clone())
                    );
                }

                _ => {}
            }
        }

        for binding in program.hir.bindings.iter() {
            if binding.phase == Phase::Runtime {
                continue;
            }

            let value = self.evaluate_expression(
                &binding.expression,
                &symbols,
            );

            self.values.insert(binding.symbol, value);
        }

        if self.diagnostics.is_empty() {
            Ok(EvaluatedProgram {
                hir: program.hir,
                values: self.values,
                symbols,
                functions: self.functions,
            })
        } else {
            Err(self.diagnostics)
        }
    }

    fn evaluate_expression(
        &mut self,
        expression: &HirExpression,
        symbols: &SymbolTable,
    ) -> ComptimeValue {
        match &expression.data {
            HirExpressionData::Function(function) => {
                let function_id = self.functions.insert(ComptimeFunction {
                    hir: function.clone()
                });

                ComptimeValue::Function(function_id)
            }

            HirExpressionData::Integer(value) =>
                ComptimeValue::I64(*value),

            HirExpressionData::Binary {
                lhs,
                operator,
                rhs
            } => {
                let lhs = self.evaluate_expression(lhs, symbols);
                let rhs = self.evaluate_expression(rhs, symbols);

                match (lhs, rhs) {
                    (ComptimeValue::I64(lhs), ComptimeValue::I64(rhs)) => {
                        match *operator {
                            BinaryOperator::Add => {
                                let result = lhs.checked_add(rhs);
                                match result {
                                    Some(result) => ComptimeValue::I64(result),
                                    None => {
                                        self.diagnostics.push(Diagnostic::error(
                                            "Integer Overflow",
                                            expression.span,
                                            "this overflows :("
                                        ));
                                        ComptimeValue::Error
                                    },
                                }
                            },
                            BinaryOperator::Subtract => {
                                let result = lhs.checked_sub(rhs);
                                match result {
                                    Some(result) => ComptimeValue::I64(result),
                                    None => {
                                        self.diagnostics.push(Diagnostic::error(
                                            "Integer Overflow",
                                            expression.span,
                                            "this overflows :("
                                        ));
                                        ComptimeValue::Error
                                    }
                                }
                            },
                            BinaryOperator::Multiply => {
                                let result = lhs.checked_mul(rhs);
                                match result {
                                    Some(result) => ComptimeValue::I64(result),
                                    None => {
                                        self.diagnostics.push(Diagnostic::error(
                                            "Integer Overflow",
                                            expression.span,
                                            "this overflows :("
                                        ));
                                        ComptimeValue::Error
                                    }
                                }
                            },
                            BinaryOperator::Divide => {
                                if rhs == 0 {
                                    self.diagnostics.push(Diagnostic::error(
                                        "Division by zero",
                                        expression.span,
                                        "really?"
                                    ));
                                    return ComptimeValue::Error;
                                }
                                let result = lhs.checked_div(rhs);
                                match result {
                                    Some(result) => ComptimeValue::I64(result),
                                    None => {
                                        self.diagnostics.push(Diagnostic::error(
                                            "Integer Overflow",
                                            expression.span,
                                            "this overflows :("
                                        ));
                                        ComptimeValue::Error
                                    }
                                }
                            },
                        }
                    }

                    (ComptimeValue::Error, _) | (_, ComptimeValue::Error) => ComptimeValue::Error,

                    _ => {
                        // Type invariant was violated
                        ComptimeValue::Error
                    }
                }
            }

            HirExpressionData::Symbol(symbol) => {
                if let Some(value) = self.values.get(*symbol) {
                    return value.clone();
                }

                let symbol_info = symbols.get(*symbol);

                let message = match &symbol_info.kind {
                    SymbolKind::BuiltinType(_) => {
                        format!(
                            "internal error: built-in `{}` has no compile-time value",
                            symbol_info.name,
                        )
                    }

                    SymbolKind::Binding {
                        phase: Phase::Runtime,
                        ..
                    } => {
                        format!(
                            "runtime binding `{}` is unavailable at compile time",
                            symbol_info.name,
                        )
                    }

                    SymbolKind::Binding {
                        phase: Phase::Comptime,
                        ..
                    } => {
                        format!(
                            "compile-time binding `{}` has not been evaluated yet",
                            symbol_info.name,
                        )
                    }

                    SymbolKind::Parameter => {
                        format!(
                            "parameter `{}` is unavailable outside a compile-time function call",
                            symbol_info.name,
                        )
                    }

                    SymbolKind::Local => {
                        format!(
                            "local binding `{}` is unavailable outside a compile-time function call",
                            symbol_info.name,
                        )
                    }
                };

                self.diagnostics.push(Diagnostic::error(
                    "Value unavailable at compile time",
                    expression.span,
                    message,
                ));

                ComptimeValue::Error
            }

            HirExpressionData::Error => ComptimeValue::Error,
        }
    }
}

#[derive(Debug)]
pub struct ValueStore {
    map: HashMap<SymbolId, ComptimeValue>,
}

impl ValueStore {
    fn insert(&mut self, symbol: SymbolId, value: ComptimeValue) {
        self.map.insert(symbol, value);
    }

    pub fn get(&self, symbol: SymbolId) -> Option<&ComptimeValue> {
        self.map.get(&symbol)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FunctionId(u32);

#[derive(Debug)]
pub struct FunctionStore {
    functions: Vec<ComptimeFunction>,
}

impl FunctionStore {
    pub fn insert(&mut self, function: ComptimeFunction) -> FunctionId {
        let id = FunctionId(self.functions.len() as u32);
        self.functions.push(function);
        id
    }

    pub fn get(&self, id: FunctionId) -> Option<&ComptimeFunction> {
        self.functions.get(id.0 as usize)
    }
}

#[derive(Debug)]
pub struct ComptimeFunction {
    pub hir: HirFunctionExpression
}