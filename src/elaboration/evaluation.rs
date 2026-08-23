use crate::comptime::{ComptimeFunction, ComptimeValue};
use crate::diagnostics::Diagnostic;
use crate::elaboration::Elaborator;
use crate::parser::ast::{BinaryOperator, Phase};
use crate::semantic::hir::{HirBlock, HirElseBranch, HirExpression, HirExpressionData, HirFunctionExpression, HirPlaceData, HirStatement, HirStatementData};
use crate::semantic::symbols::{SymbolId, SymbolKind};
use crate::semantic::types::Type;
use std::collections::HashMap;

pub(super) enum EvaluationFlow {
    Continue,
    Return(ComptimeValue),
    Error,
}

impl Elaborator<'_> {
    pub(super) fn lookup_value(
        &self,
        symbol: SymbolId
    ) -> Option<&ComptimeValue> {
        self.frames
            .iter()
            .rev()
            .find_map(|frame| frame.get(&symbol))
            .or_else(|| self.values.get(symbol))
    }

    pub(super) fn evaluate_statement(
        &mut self,
        statement: &HirStatement,
    ) -> EvaluationFlow {
        match &statement.data {
            HirStatementData::Error => EvaluationFlow::Error,

            HirStatementData::Expression(expression) => {
                match self.evaluate_expression(expression) {
                    ComptimeValue::Error => EvaluationFlow::Error,
                    _ => EvaluationFlow::Continue,
                }
            }

            HirStatementData::Assignment { target, expression } => {
                let value = self.evaluate_expression(expression);

                if value == ComptimeValue::Error {
                    return EvaluationFlow::Error;
                }

                let frame = self
                    .frames
                    .last_mut()
                    .expect("assignment requires a function call frame");

                match &target.data {
                    HirPlaceData::Symbol(symbol) => {
                        let Some(slot) = frame.get_mut(symbol) else {
                            panic!("HIR invariant was violated: the local should have been bound earlier");
                        };

                        *slot = value;
                    }

                    HirPlaceData::Index { .. } => {
                        self.diagnostics.push(Diagnostic::error(
                            "Arrays are not supported at compile time right now",
                            statement.span,
                            ":("
                        ));

                        return EvaluationFlow::Error;
                    }
                }

                EvaluationFlow::Continue
            }

            HirStatementData::Binding { symbol, expression } => {
                let value = self.evaluate_expression(expression);
                if value == ComptimeValue::Error {
                    return EvaluationFlow::Error;
                }

                let frame = self
                    .frames
                    .last_mut()
                    .expect("function evaluation requires a stack frame");

                frame.insert(*symbol, value);
                EvaluationFlow::Continue
            }

            HirStatementData::Return(Some(expression)) => {
                EvaluationFlow::Return(self.evaluate_expression(expression))
            }

            HirStatementData::Return(None) => EvaluationFlow::Return(ComptimeValue::Unit),

            HirStatementData::While {
                condition,
                while_block,
            } => loop {
                match self.evaluate_expression(condition) {
                    ComptimeValue::Bool(false) => {
                        return EvaluationFlow::Continue;
                    }

                    ComptimeValue::Bool(true) => {}

                    ComptimeValue::Error => {
                        return EvaluationFlow::Error;
                    }

                    _ => {
                        self.diagnostics.push(Diagnostic::error(
                            "Internal Compiler Error",
                            condition.span,
                            "not a boolean expression",
                        ));
                        return EvaluationFlow::Error;
                    }
                }

                match self.evaluate_block(while_block) {
                    EvaluationFlow::Continue => {}

                    EvaluationFlow::Return(value) => {
                        return EvaluationFlow::Return(value);
                    }

                    EvaluationFlow::Error => {
                        return EvaluationFlow::Error;
                    }
                }
            },

            HirStatementData::If {
                condition,
                then_block,
                else_branch,
            } => {
                let condition = self.evaluate_expression(condition);

                match condition {
                    ComptimeValue::Bool(true) => self.evaluate_block(then_block),
                    ComptimeValue::Bool(false) => match else_branch {
                        Some(HirElseBranch::Else(block)) => self.evaluate_block(block),
                        Some(HirElseBranch::ElseIf(statement)) => {
                            self.evaluate_statement(statement)
                        }

                        None => EvaluationFlow::Continue,
                    },

                    ComptimeValue::Error => EvaluationFlow::Error,

                    _ => {
                        // Semantic invariant was violated
                        EvaluationFlow::Error
                    }
                }
            }
        }
    }

    pub(super) fn evaluate_block(&mut self, block: &HirBlock) -> EvaluationFlow {
        for statement in block.statements.iter() {
            match self.evaluate_statement(statement) {
                EvaluationFlow::Continue => {}
                flow => return flow,
            }
        }

        EvaluationFlow::Continue
    }

    pub(super) fn evaluate_expression(
        &mut self,
        expression: &HirExpression,
    ) -> ComptimeValue {
        match &expression.data {
            HirExpressionData::Index { .. }
            | HirExpressionData::ArrayRepeatInitialization { .. } => {
                self.diagnostics.push(Diagnostic::error(
                    "Arrays are currently unsupported in comptime",
                    expression.span,
                    ":(",
                ));

                ComptimeValue::Error
            }

            HirExpressionData::Bool(bool) => ComptimeValue::Bool(*bool),

            HirExpressionData::Function(function) => {
                let function_id = self.functions.insert(ComptimeFunction {
                    hir: function.clone(),
                });

                ComptimeValue::Function(function_id)
            }

            HirExpressionData::Integer(value) => ComptimeValue::I64(*value),

            HirExpressionData::Binary { lhs, operator, rhs } => {
                let lhs = self.evaluate_expression(lhs);
                let rhs = self.evaluate_expression(rhs);

                match (operator, lhs, rhs) {
                    (
                        BinaryOperator::GreaterThanOrEqual,
                        ComptimeValue::I64(lhs),
                        ComptimeValue::I64(rhs),
                    ) => ComptimeValue::Bool(lhs >= rhs),

                    (
                        BinaryOperator::GreaterThan,
                        ComptimeValue::I64(lhs),
                        ComptimeValue::I64(rhs),
                    ) => ComptimeValue::Bool(lhs > rhs),

                    (
                        BinaryOperator::LessThanOrEqual,
                        ComptimeValue::I64(lhs),
                        ComptimeValue::I64(rhs),
                    ) => ComptimeValue::Bool(lhs <= rhs),

                    (
                        BinaryOperator::LessThan,
                        ComptimeValue::I64(lhs),
                        ComptimeValue::I64(rhs),
                    ) => ComptimeValue::Bool(lhs < rhs),

                    (
                        BinaryOperator::NotEqual,
                        ComptimeValue::I64(lhs),
                        ComptimeValue::I64(rhs),
                    ) => ComptimeValue::Bool(lhs != rhs),

                    (
                        BinaryOperator::NotEqual,
                        ComptimeValue::Bool(lhs),
                        ComptimeValue::Bool(rhs),
                    ) => ComptimeValue::Bool(lhs != rhs),

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
                let callee_value = self.evaluate_expression(callee);

                let ComptimeValue::Function(function_id) = callee_value else {
                    // Semantic analysis should prevent this from occuring
                    return ComptimeValue::Error;
                };

                let argument_values = arguments
                    .iter()
                    .map(|argument| self.evaluate_expression(argument))
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

                let result = self.evaluate_function_body(&function);

                self.frames.pop();

                result
            }

            HirExpressionData::Symbol(symbol) => {
                if let Some(value) = self.lookup_value(*symbol).cloned() {
                    return value;
                }

                let is_pending_comptime_binding = matches!(
                    self.symbols.get(*symbol).kind,
                    SymbolKind::Binding {
                        phase: Phase::Comptime,
                        ..
                    }
                ) && self.pending_bindings.contains_key(symbol);
                
                if is_pending_comptime_binding {
                    return self
                        .ensure_binding_evaluated(*symbol)
                        .unwrap_or(ComptimeValue::Error);
                }
                
                let symbol_info = self.symbols.get(*symbol);

                let message = match &symbol_info.kind {
                    SymbolKind::ExternFunction { .. } => {
                        format!(
                            "runtime external function `{}` is unavailable at compile time",
                            symbol_info.name
                        )
                    }

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

                    SymbolKind::Local { .. } => {
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

    pub(super) fn evaluate_function_body(
        &mut self,
        function: &HirFunctionExpression,
    ) -> ComptimeValue {
        for statement in function.body.statements.iter() {
            let flow = self.evaluate_statement(statement);

            match flow {
                EvaluationFlow::Continue => continue,
                EvaluationFlow::Return(value) => return value,
                EvaluationFlow::Error => return ComptimeValue::Error,
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