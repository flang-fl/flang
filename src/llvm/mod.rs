use crate::comptime::{ComptimeFunction, ComptimeValue, EvaluatedProgram, FunctionId};
use crate::diagnostics::Diagnostic;
use crate::parser::ast::{BinaryOperator, Phase};
use crate::semantic::hir::{HirExpression, HirExpressionData, HirStatementData};
use crate::semantic::symbols::{SymbolId, SymbolKind};
use crate::semantic::types::Type;
use crate::source::{SourceId, Span};
use std::collections::HashMap;

struct FunctionEmitter<'program> {
    program: &'program EvaluatedProgram,
    instructions: String,
    next_temporary: usize,
    parameter_operands: HashMap<SymbolId, String>,
}

impl<'program> FunctionEmitter<'program> {
    fn new(
        program: &'program EvaluatedProgram,
        function: &ComptimeFunction,
    ) -> Self {
        let parameter_operands = function
            .hir
            .parameters
            .iter()
            .enumerate()
            .map(|(index, parameter)| {
                (
                    parameter.symbol,
                    format!("%arg{index}")
                )
            })
            .collect();

        Self {
            program,
            instructions: String::new(),
            next_temporary: 0,
            parameter_operands,
        }
    }

    fn fresh_temporary(&mut self) -> String {
        let temporary = format!("%tmp{}", self.next_temporary);
        self.next_temporary += 1;
        temporary
    }

    fn emit_i64_expression(
        &mut self,
        expression: &HirExpression,
    ) -> Result<String, Diagnostic> {
        match &expression.data {
            HirExpressionData::Integer(value) => {
                Ok(value.to_string())
            }

            HirExpressionData::Symbol(symbol_id) => {
                if let Some(operand) =
                    self.parameter_operands.get(symbol_id)
                {
                    return Ok(operand.clone());
                }

                match self.program.values.get(*symbol_id) {
                    Some(ComptimeValue::I64(value)) => {
                        Ok(value.to_string())
                    }

                    Some(_) => {
                        todo!("Diagnostic: symbol isn't an i64 constant");
                    }

                    None => {
                        todo!("Eventually this may be a runtime variable or parameter");
                    }
                }
            }

            HirExpressionData::Binary {
                lhs,
                operator,
                rhs,
            } => {
                let lhs = self.emit_i64_expression(lhs)?;
                let rhs = self.emit_i64_expression(rhs)?;

                let opcode = match operator {
                    BinaryOperator::Add => "add",
                    BinaryOperator::Subtract => "sub",
                    BinaryOperator::Multiply => "mul",
                    BinaryOperator::Divide => "sdiv",
                };

                let result = self.fresh_temporary();

                self.instructions.push_str(&format!(
                    "  {result} = {opcode} i64 {lhs}, {rhs}\n"
                ));

                Ok(result)
            }

            HirExpressionData::Error => {
                panic!("This should have been caught before LLVM generation");
            }

            HirExpressionData::Call {
                callee,
                arguments,
            } => {
                let function_id = match &callee.data {
                    HirExpressionData::Symbol(symbol_id) => {
                        match self.program.values.get(*symbol_id) {
                            Some(ComptimeValue::Function(function_id)) => {
                                *function_id
                            }

                            _ => {
                                return Err(Diagnostic::error(
                                    "Function is unavailable at runtime",
                                    callee.span,
                                    "callee must currently be a compile-time-known function",
                                ));
                            }
                        }
                    }

                    _ => {
                        return Err(Diagnostic::error(
                            "Unsupported runtime callee",
                            callee.span,
                            "only calls to compile-time-known function bindings are currently supported",
                        ));
                    }
                };

                let llvm_arguments = arguments
                    .iter()
                    .map(|argument| {
                        self.emit_i64_expression(argument)
                            .map(|operand| format!("i64 {operand}"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;

                let llvm_arguments = llvm_arguments.join(", ");

                let function_name = llvm_function_name(function_id);
                let result = self.fresh_temporary();

                self.instructions.push_str(&format!(
                    "  {result} = call i64 @{function_name}({llvm_arguments})\n"
                ));

                Ok(result)
            }

            HirExpressionData::Function(_) => {
                todo!("Diagnostic: functions arent i64s smh");
            }
        }
    }

    fn emit_i64_body(
        mut self,
        function: &ComptimeFunction,
    ) -> Result<String, Diagnostic> {
        let [statement] = function.hir.body.statements.as_slice() else {
            todo!("For now: Requires exactly one statement in main");
        };

        match &statement.data {
            HirStatementData::Return(Some(expression)) => {
                let operand = self.emit_i64_expression(expression)?;

                self.instructions.push_str(
                    &format!("  ret i64 {operand}\n")
                );
            }

            HirStatementData::Return(None) => {
                todo!("No empty returns in main")
            }
        }

        Ok(self.instructions)
    }
}

pub fn emit(program: &EvaluatedProgram) -> Result<String, Vec<Diagnostic>> {
    let symbols = &program.symbols;
    let functions = &program.functions;
    let values = &program.values;

    let mut diagnostics = Vec::new();

    let mut llvm = String::new();

    let Some((main_id, main_symbol)) = symbols.find_by_name("main") else {
        return Err(vec![Diagnostic::error(
            "No main method found",
            Span {
                source: SourceId(0),
                start: 0,
                end: 0,
            },
            "=(",
        )]);
    };

    let SymbolKind::Binding { phase, mutable } = &main_symbol.kind else {
        return Err(vec![Diagnostic::error(
            "`main` needs to be an immutable comptime binding for a runtime function",
            main_symbol.declaration_span.unwrap(),
            "Evil !",
        )]);
    };

    if *phase != Phase::Comptime || *mutable == true {
        todo!("main cant be runtime binding or mutable");
    }

    let Some(main_value) = values.get(main_id) else {
        return Err(vec![Diagnostic::error(
            "`main` has no compile-time value",
            main_symbol.declaration_span.unwrap(),
            "`main` must be available at compile time",
        )]);
    };

    let ComptimeValue::Function(function_id) = main_value else {
        return Err(vec![Diagnostic::error(
            "`main` is not a function",
            main_symbol.declaration_span.unwrap(),
            "expected a function value",
        )]);
    };

    let Some(main_fn) = functions.get(*function_id) else {
        todo!("`main` is not a function");
    };

    if main_fn.hir.return_type != Type::I64 {
        todo!("`main` must return an i64 rn");
    }

    if !main_fn.hir.parameters.is_empty() {
        todo!("`main` must have no parameters");
    }

    for (function_id, function) in functions.iter() {
        let llvm_parameters = function
            .hir
            .parameters
            .iter()
            .enumerate()
            .map(|(index, parameter)| {
                if parameter.type_ != Type::I64 {
                    todo!("non-i64 parameters are evil");
                }
                format!("i64 %arg{index}")
            })
            .collect::<Vec<_>>()
            .join(", ");

        let emitter = FunctionEmitter::new(program, function);

        let body = emitter.emit_i64_body(function);
        match body {
            Err(diagnostic) => {
                diagnostics.push(diagnostic);
                continue;
            }
            Ok(body) => {
                let name = llvm_function_name(function_id);

                let definition = format!(
                    "define i64 @{name}({llvm_parameters}) {{\n\
                    entry:\n\
                    {body}\
                    }}\n\n"
                );

                llvm.push_str(&definition);
            }
        }
    }

    let main_name = llvm_function_name(*function_id);

    llvm.push_str(&format!(
        "define i32 @main() {{\n\
        entry:\n\
          %result = call i64 @{main_name}()\n\
          %status = trunc i64 %result to i32\n\
          ret i32 %status\n\
        }}\n"
    ));

    if diagnostics.is_empty() {
        Ok(llvm)
    } else {
        Err(diagnostics)
    }
}

fn llvm_function_name(function_id: FunctionId) -> String {
    format!("flang_fn_{}", function_id.index())
}