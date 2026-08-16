use crate::diagnostics::Diagnostic;
use crate::parser::ast::{BinaryOperator, Phase};
use crate::semantic::SemanticProgram;
use crate::semantic::hir::{
    HirExpression, HirExpressionData, HirFunctionExpression, HirProgram, HirStatementData,
};
use crate::semantic::symbols::{SymbolId, SymbolKind, SymbolTable};
use crate::semantic::types::Type;
use std::cmp::PartialEq;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum ComptimeValue {
    I64(i64),
    Function(FunctionId),
    Type(Type),
    Bool(bool),
    Unit,
    Error,
}

#[derive(Debug)]
pub struct EvaluatedProgram {
    pub symbols: SymbolTable,
    pub values: ValueStore,
    pub functions: FunctionStore,
    pub hir: HirProgram,
}

pub struct Evaluator {
    values: ValueStore,
    functions: FunctionStore,
    frames: Vec<HashMap<SymbolId, ComptimeValue>>,
    diagnostics: Vec<Diagnostic>,
}

impl Evaluator {
    pub fn new() -> Self {
        Self {
            values: ValueStore {
                map: HashMap::new(),
            },
            functions: FunctionStore {
                functions: Vec::new(),
            },
            frames: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn lookup_value(&self, symbol: SymbolId) -> Option<&ComptimeValue> {
        self.frames
            .iter()
            .rev()
            .find_map(|frame| frame.get(&symbol))
            .or_else(|| self.values.get(symbol))
    }

    pub fn evaluate(
        mut self,
        program: SemanticProgram,
    ) -> Result<EvaluatedProgram, Vec<Diagnostic>> {
        let symbols = program.symbols;
        for (id, value) in symbols.symbols.iter().enumerate() {
            let id = SymbolId(id as u32);

            match &value.kind {
                SymbolKind::BuiltinType(type_) => {
                    self.values.insert(id, ComptimeValue::Type(type_.clone()));
                }

                _ => {}
            }
        }

        for binding in program.hir.bindings.iter() {
            if binding.phase == Phase::Runtime {
                continue;
            }

            let value = self.evaluate_expression(&binding.expression, &symbols);

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
            HirExpressionData::Bool(bool) => ComptimeValue::Bool(*bool),

            HirExpressionData::Function(function) => {
                let function_id = self.functions.insert(ComptimeFunction {
                    hir: function.clone(),
                });

                ComptimeValue::Function(function_id)
            }

            HirExpressionData::Integer(value) => ComptimeValue::I64(*value),

            HirExpressionData::Binary { lhs, operator, rhs } => {
                let lhs = self.evaluate_expression(lhs, symbols);
                let rhs = self.evaluate_expression(rhs, symbols);

                match (operator, lhs, rhs) {
                    (BinaryOperator::NotEqual, ComptimeValue::I64(lhs), ComptimeValue::I64(rhs)) => {
                        ComptimeValue::Bool(lhs != rhs)
                    }

                    (BinaryOperator::NotEqual, ComptimeValue::Bool(lhs), ComptimeValue::Bool(rhs)) => {
                        ComptimeValue::Bool(lhs != rhs)
                    }

                    (BinaryOperator::Equal, ComptimeValue::I64(lhs), ComptimeValue::I64(rhs)) => {
                        ComptimeValue::Bool(lhs == rhs)
                    }

                    (BinaryOperator::Equal, ComptimeValue::Bool(lhs), ComptimeValue::Bool(rhs)) => {
                        ComptimeValue::Bool(lhs == rhs)
                    }

                    (BinaryOperator::Add, ComptimeValue::I64(lhs), ComptimeValue::I64(rhs)) => {
                        let result = lhs.checked_add(rhs);
                        match result {
                            Some(result) => ComptimeValue::I64(result),
                            None => {
                                self.diagnostics.push(Diagnostic::error(
                                    "Integer Overflow",
                                    expression.span,
                                    "this overflows :(",
                                ));
                                ComptimeValue::Error
                            }
                        }
                    }
                    (
                        BinaryOperator::Subtract,
                        ComptimeValue::I64(lhs),
                        ComptimeValue::I64(rhs),
                    ) => {
                        let result = lhs.checked_sub(rhs);
                        match result {
                            Some(result) => ComptimeValue::I64(result),
                            None => {
                                self.diagnostics.push(Diagnostic::error(
                                    "Integer Overflow",
                                    expression.span,
                                    "this overflows :(",
                                ));
                                ComptimeValue::Error
                            }
                        }
                    }
                    (
                        BinaryOperator::Multiply,
                        ComptimeValue::I64(lhs),
                        ComptimeValue::I64(rhs),
                    ) => {
                        let result = lhs.checked_mul(rhs);
                        match result {
                            Some(result) => ComptimeValue::I64(result),
                            None => {
                                self.diagnostics.push(Diagnostic::error(
                                    "Integer Overflow",
                                    expression.span,
                                    "this overflows :(",
                                ));
                                ComptimeValue::Error
                            }
                        }
                    }
                    (BinaryOperator::Divide, ComptimeValue::I64(lhs), ComptimeValue::I64(rhs)) => {
                        if rhs == 0 {
                            self.diagnostics.push(Diagnostic::error(
                                "Division by zero",
                                expression.span,
                                "really?",
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
                                    "this overflows :(",
                                ));
                                ComptimeValue::Error
                            }
                        }
                    }

                    (_, ComptimeValue::Error, _) | (_, _, ComptimeValue::Error) => {
                        ComptimeValue::Error
                    }

                    _ => {
                        // Type invariant was violated
                        ComptimeValue::Error
                    }
                }
            }

            HirExpressionData::Call { callee, arguments } => {
                let callee_value = self.evaluate_expression(callee, symbols);

                let ComptimeValue::Function(function_id) = callee_value else {
                    // Semantic analysis should prevent this from occuring
                    return ComptimeValue::Error;
                };

                let argument_values = arguments
                    .iter()
                    .map(|argument| self.evaluate_expression(argument, symbols))
                    .collect::<Vec<_>>();

                if argument_values
                    .iter()
                    .any(|argument| *argument == ComptimeValue::Error)
                {
                    return ComptimeValue::Error;
                }

                let function = match self.functions.get(function_id) {
                    Some(function) => function.hir.clone(),
                    None => {
                        panic!("Test Explode");
                        return ComptimeValue::Error;
                    }
                };

                let frame = function
                    .parameters
                    .iter()
                    .zip(argument_values)
                    .map(|(parameter, argument)| (parameter.symbol, argument))
                    .collect::<HashMap<_, _>>();

                self.frames.push(frame);

                let result = self.evaluate_function_body(&function, symbols);

                self.frames.pop();

                result
            }

            HirExpressionData::Symbol(symbol) => {
                if let Some(value) = self.lookup_value(*symbol).cloned() {
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

    fn evaluate_function_body(
        &mut self,
        function: &HirFunctionExpression,
        symbols: &SymbolTable,
    ) -> ComptimeValue {
        let Some(statement) = function.body.statements.first() else {
            self.diagnostics.push(Diagnostic::error(
                "Function is missing a return statement",
                function.body.span,
                ":(",
            ));

            return ComptimeValue::Error;
        };

        for statement in function.body.statements.iter() {
            match &statement.data {
                HirStatementData::Binding { symbol, expression } => {
                    let value = self.evaluate_expression(expression, symbols);
                    if value == ComptimeValue::Error {
                        return ComptimeValue::Error;
                    }

                    let frame = self
                        .frames
                        .last_mut()
                        .expect("function evaluation requires a stack frame");

                    frame.insert(*symbol, value);
                }

                HirStatementData::Return(Some(expression)) => {
                    return self.evaluate_expression(expression, symbols);
                }

                HirStatementData::Return(None) => {
                    return ComptimeValue::Unit;
                }
            }
        }

        if function.return_type == Type::Unit {
            ComptimeValue::Unit
        } else {
            self.diagnostics.push(Diagnostic::error(
                "Function is missing a return statement",
                function.body.span,
                "no return :(",
            ));

            ComptimeValue::Error
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

impl FunctionId {
    pub fn index(self) -> u32 {
        self.0
    }
}

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

    pub fn iter(&self) -> impl Iterator<Item = (FunctionId, &ComptimeFunction)> {
        self.functions
            .iter()
            .enumerate()
            .map(|(index, function)| (FunctionId(index as u32), function))
    }
}

#[derive(Debug)]
pub struct ComptimeFunction {
    pub hir: HirFunctionExpression,
}
