use crate::comptime::{ComptimeFunction, ComptimeValue};
use crate::diagnostics::Diagnostic;
use crate::elaboration::Elaborator;
use crate::parser::ast::{BinaryOperator, Phase, UnaryOperator};
use crate::semantic::hir::{
    HirBlock, HirElseBranch, HirExpression, HirExpressionData, HirFunctionExpression, HirPlaceData,
    HirStatement, HirStatementData,
};
use crate::semantic::symbols::{SymbolId, SymbolKind};
use crate::semantic::types::Type;
use std::collections::HashMap;

pub(super) enum EvaluationFlow {
    Continue,
    Return(ComptimeValue),
    Error,
}

impl Elaborator<'_> {
    pub(super) fn lookup_value(&self, symbol: SymbolId) -> Option<&ComptimeValue> {
        self.frames
            .iter()
            .rev()
            .find_map(|frame| frame.get(&symbol))
            .or_else(|| self.values.get(symbol))
    }

    pub(super) fn evaluate_statement(&mut self, statement: &HirStatement) -> EvaluationFlow {
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
                            panic!(
                                "HIR invariant was violated: the local should have been bound earlier"
                            );
                        };

                        *slot = value;
                    }

                    HirPlaceData::Index { .. } => {
                        self.diagnostics.push(Diagnostic::error(
                            "Arrays are not supported at compile time right now",
                            statement.span,
                            ":(",
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

    pub(super) fn evaluate_expression(&mut self, expression: &HirExpression) -> ComptimeValue {
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
                    captures: HashMap::new(),
                });

                ComptimeValue::Function(function_id)
            }

            HirExpressionData::FunctionTemplate(template_id) => {
                ComptimeValue::FunctionTemplate(*template_id)
            }

            HirExpressionData::KnownFunction(function_id) => {
                ComptimeValue::Function(*function_id)
            }

            HirExpressionData::Integer(value) => {
                let Some(integer_type) = expression.type_.as_integer() else {
                    return ComptimeValue::Error;
                };

                ComptimeValue::Integer {
                    value: *value,
                    type_: integer_type,
                }
            }

            HirExpressionData::Unary {
                operator,
                operand
            } => {
                let operand = self.evaluate_expression(operand);

                match (operator, operand) {
                    (
                        UnaryOperator::Negate,
                        ComptimeValue::Integer {
                            value,
                            type_
                        },
                    ) => {
                        if !type_.is_signed() {
                            self.diagnostics.push(Diagnostic::error(
                                "Cannot negate unsigned integer",
                                expression.span,
                                format!(
                                    "`{}` is unsigned",
                                    type_.name()
                                )
                            ));

                            return ComptimeValue::Error;
                        }

                        let Some(value) = value.checked_neg() else {
                            self.diagnostics.push(Diagnostic::error(
                                "Integer overflow",
                                expression.span,
                                "negation overflowed"
                            ));

                            return ComptimeValue::Error;
                        };

                        if !type_.contains(
                            value,
                            &self.target
                        ) {
                            self.diagnostics.push(Diagnostic::error(
                                "Integer overflow",
                                expression.span,
                                format!(
                                    "result `{value}` does not fit in `{}`",
                                    type_.name()
                                )
                            ));

                            return ComptimeValue::Error;
                        }

                        ComptimeValue::Integer {
                            value,
                            type_
                        }
                    }

                    (_, ComptimeValue::Error) => ComptimeValue::Error,

                    (UnaryOperator::Negate, _) => {
                        self.diagnostics.push(Diagnostic::error(
                            "Evil bad",
                            expression.span,
                            "Fix my diagnostic later"
                        ));

                        ComptimeValue::Error
                    }
                }
            }

            HirExpressionData::Binary { lhs, operator, rhs } => {
                let lhs = self.evaluate_expression(lhs);
                let rhs = self.evaluate_expression(rhs);

                match (lhs, rhs) {
                    (ComptimeValue::Error, _) | (_, ComptimeValue::Error) => ComptimeValue::Error,

                    (
                        ComptimeValue::Integer {
                            value: lhs,
                            type_: lhs_type,
                        },
                        ComptimeValue::Integer {
                            value: rhs,
                            type_: rhs_type,
                        },
                    ) => {
                        if lhs_type != rhs_type {
                            self.diagnostics.push(Diagnostic::error(
                                "Mismatched integer types",
                                expression.span,
                                format!(
                                    "cannot evaluate `{}` and `{}` together",
                                    lhs_type.name(),
                                    rhs_type.name()
                                ),
                            ));

                            return ComptimeValue::Error;
                        }

                        match operator {
                            BinaryOperator::Equal => ComptimeValue::Bool(lhs == rhs),

                            BinaryOperator::NotEqual => ComptimeValue::Bool(lhs != rhs),

                            BinaryOperator::LessThan => ComptimeValue::Bool(lhs < rhs),

                            BinaryOperator::LessThanOrEqual => ComptimeValue::Bool(lhs <= rhs),

                            BinaryOperator::GreaterThan => ComptimeValue::Bool(lhs > rhs),

                            BinaryOperator::GreaterThanOrEqual => ComptimeValue::Bool(lhs >= rhs),

                            BinaryOperator::Add
                            | BinaryOperator::Subtract
                            | BinaryOperator::Multiply
                            | BinaryOperator::Divide => {
                                let result = match operator {
                                    BinaryOperator::Add => lhs.checked_add(rhs),
                                    BinaryOperator::Subtract => lhs.checked_sub(rhs),
                                    BinaryOperator::Multiply => lhs.checked_mul(rhs),

                                    BinaryOperator::Divide => {
                                        if rhs == 0 {
                                            self.diagnostics.push(Diagnostic::error(
                                                "Division by zero",
                                                expression.span,
                                                "the divisor evaluates to zero",
                                            ));

                                            return ComptimeValue::Error;
                                        }

                                        lhs.checked_div(rhs)
                                    }

                                    _ => unreachable!(),
                                };

                                let Some(result) = result else {
                                    self.diagnostics.push(Diagnostic::error(
                                        "Integer overflow",
                                        expression.span,
                                        format!("this operation overflows `{}`", lhs_type.name()),
                                    ));

                                    return ComptimeValue::Error;
                                };

                                if !lhs_type.contains(
                                    result,
                                    &self.target
                                ) {
                                    self.diagnostics.push(Diagnostic::error(
                                        "Integer overflow",
                                        expression.span,
                                        format!(
                                            "result `{result}` does not fit in `{}`",
                                            lhs_type.name()
                                        ),
                                    ));

                                    return ComptimeValue::Error;
                                }

                                ComptimeValue::Integer {
                                    value: result,
                                    type_: lhs_type,
                                }
                            }
                        }
                    }
                    (ComptimeValue::Bool(lhs), ComptimeValue::Bool(rhs)) => match operator {
                        BinaryOperator::Equal => ComptimeValue::Bool(lhs == rhs),
                        BinaryOperator::NotEqual => ComptimeValue::Bool(lhs != rhs),

                        _ => {
                            self.diagnostics.push(Diagnostic::error(
                                "Invalid boolean operation",
                                expression.span,
                                format!("operator `{operator:?}` cannot be applied to booleans"),
                            ));

                            ComptimeValue::Error
                        }
                    },

                    (lhs, rhs) => {
                        self.diagnostics.push(Diagnostic::error(
                            "Invalid compile-time binary operation",
                            expression.span,
                            format!(
                                "operator `{operator:?}` cannot be applied to \
                                `{lhs:?}` and `{rhs:?}`"
                            )
                        ));

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

                let stored_function = match self.functions.get(function_id) {
                    Some(function) => function.clone(),
                    None => {
                        return ComptimeValue::Error;
                    }
                };

                let mut frame = stored_function.captures.clone();

                for (parameter, argument) in stored_function
                    .hir
                    .parameters
                    .iter()
                    .zip(argument_values)
                {
                    frame.insert(parameter.symbol, argument);
                }

                self.frames.push(frame);

                let result = self.evaluate_function_body(&stored_function.hir);

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

                    SymbolKind::ComptimeParameter => {
                        format!(
                            "comptime parameter `{}` is unavailable outside its specialization",
                            symbol_info.name
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
