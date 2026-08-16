use crate::comptime::{ComptimeFunction, ComptimeValue, EvaluatedProgram, FunctionId};
use crate::semantic::hir::{HirExpression, HirExpressionData, HirStatementData};
use crate::semantic::symbols::SymbolId;
use crate::semantic::types::Type;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::values::{BasicMetadataValueEnum, FunctionValue, IntValue};
use std::collections::HashMap;
use crate::parser::ast::BinaryOperator;

pub fn emit(program: &EvaluatedProgram) -> Result<String, String> {
    let context = Context::create();
    let mut generator = CodeGenerator::new(&context, program);

    generator.declare_functions()?;
    generator.emit_function_bodies()?;
    generator.emit_main_wrapper()?;

    generator
        .module
        .verify()
        .map_err(|message| message.to_string())?;

    Ok(generator.module.print_to_string().to_string())
}

pub struct CodeGenerator<'ctx, 'program> {
    context: &'ctx Context,
    program: &'program EvaluatedProgram,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    functions: HashMap<FunctionId, FunctionValue<'ctx>>,
}

impl<'ctx, 'program> CodeGenerator<'ctx, 'program> {
    pub fn new(context: &'ctx Context, program: &'program EvaluatedProgram) -> Self {
        Self {
            context,
            program,
            module: context.create_module("flang"),
            builder: context.create_builder(),
            functions: HashMap::new(),
        }
    }

    fn emit_main_wrapper(&self) -> Result<(), String> {
        let (main_symbol_id, _) = self
            .program
            .symbols
            .find_by_name("main")
            .ok_or_else(|| {
                "program does not define `main`".to_owned()
            })?;

        let function_id = match self.program.values.get(main_symbol_id) {
            Some(ComptimeValue::Function(function_id)) => {
                *function_id
            }

            _ => {
                return Err(
                    "`main` must be a compile-time-known function"
                        .to_owned(),
                );
            }
        };

        let function = self
            .program
            .functions
            .get(function_id)
            .ok_or_else(|| {
                "internal error: main FunctionId is missing"
                    .to_owned()
            })?;

        if !function.hir.parameters.is_empty() {
            return Err(
                "`main` must not take parameters".to_owned(),
            );
        }

        if function.hir.return_type != Type::I64 {
            return Err(
                "`main` must return i64".to_owned(),
            );
        }

        let language_main = self
            .functions
            .get(&function_id)
            .copied()
            .ok_or_else(|| {
                "internal error: main was not declared in LLVM"
                    .to_owned()
            })?;

        let i32_type = self.context.i32_type();
        let wrapper_type = i32_type.fn_type(&[], false);

        let wrapper = self.module.add_function(
            "main",
            wrapper_type,
            None,
        );

        let entry = self.context.append_basic_block(
            wrapper,
            "entry",
        );

        self.builder.position_at_end(entry);

        let call = self.builder
            .build_call(
                language_main,
                &[],
                "main_result",
            )
            .map_err(|error| error.to_string())?;

        let result = call
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| {
                "language main did not produce a value".to_owned()
            })?
            .into_int_value();

        let status = self.builder
            .build_int_truncate(
                result,
                i32_type,
                "status",
            )
            .map_err(|error| error.to_string())?;

        self.builder
            .build_return(Some(&status))
            .map_err(|error| error.to_string())?;

        Ok(())
    }

    fn declare_functions(&mut self) -> Result<(), String> {
        let i64_type = self.context.i64_type();

        for (function_id, function) in self.program.functions.iter() {
            if function.hir.return_type != Type::I64 {
                return Err("Inkwell backend currently requires i64 return types".to_owned());
            }

            let parameter_types = function
                .hir
                .parameters
                .iter()
                .map(|parameter| {
                    if parameter.type_ != Type::I64 {
                        return Err(
                            "Inkwell backend currently requires i64 return types".to_owned()
                        );
                    }

                    Ok(i64_type.into())
                })
                .collect::<Result<Vec<_>, String>>()?;

            let function_type = i64_type.fn_type(&parameter_types, false);

            let name = llvm_function_name(function_id);

            let llvm_function = self.module.add_function(&name, function_type, None);

            self.functions.insert(function_id, llvm_function);
        }

        Ok(())
    }

    fn emit_function_bodies(&mut self) -> Result<(), String> {
        for (function_id, function) in self.program.functions.iter() {
            let llvm_function = self.functions.get(&function_id).copied().ok_or_else(|| {
                format!(
                    "internal error: function {:?} was not declared",
                    function_id
                )
            })?;

            self.emit_function_body(llvm_function, function)?;
        }

        Ok(())
    }

    fn emit_function_body(
        &mut self,
        llvm_function: FunctionValue<'ctx>,
        function: &ComptimeFunction,
    ) -> Result<(), String> {
        let entry = self.context.append_basic_block(llvm_function, "entry");

        self.builder.position_at_end(entry);

        let mut operands = HashMap::<SymbolId, IntValue<'ctx>>::new();

        for (index, (parameter, llvm_parameter)) in function
            .hir
            .parameters
            .iter()
            .zip(llvm_function.get_param_iter())
            .enumerate()
        {
            let llvm_parameter = llvm_parameter.into_int_value();
            llvm_parameter.set_name(&format!("arg{index}"));

            operands.insert(parameter.symbol, llvm_parameter);
        }

        for statement in function.hir.body.statements.iter() {
            match &statement.data {
                HirStatementData::Binding {
                    symbol,
                    expression
                } => {
                    let value = self.emit_i64_expression(expression, &operands)?;
                    operands.insert(*symbol, value);
                },

                HirStatementData::Return(Some(expression)) => {
                    let value = self.emit_i64_expression(expression, &operands)?;

                    self.builder
                        .build_return(Some(&value))
                        .map_err(|error| error.to_string())?;
                    
                    return Ok(());
                }

                HirStatementData::Return(None) => {
                    return Err(
                        "i64 function cannot return without a value"
                            .to_owned()
                    )
                }
            }
        }

        Err("Function body without a return".to_owned())
    }

    fn emit_i64_expression(
        &self,
        expression: &HirExpression,
        operands: &HashMap<SymbolId, IntValue<'ctx>>,
    ) -> Result<IntValue<'ctx>, String> {
        match &expression.data {
            HirExpressionData::Integer(value) => {
                Ok(self.context.i64_type().const_int(*value as u64, true))
            }

            HirExpressionData::Symbol(symbol_id) => {
                if let Some(value) = operands.get(symbol_id) {
                    return Ok(*value);
                }

                match self.program.values.get(*symbol_id) {
                    Some(ComptimeValue::I64(value)) => {
                        Ok(self.context.i64_type().const_int(*value as u64, true))
                    }

                    _ => Err("symbol has no available i64 LLVM value".to_owned()),
                }
            }

            HirExpressionData::Binary {
                lhs,
                operator,
                rhs
            } => {
                let lhs = self.emit_i64_expression(lhs, operands)?;
                let rhs = self.emit_i64_expression(rhs, operands)?;

                let result = match operator {
                    BinaryOperator::Add => {
                        self.builder.build_int_add(
                            lhs,
                            rhs,
                            "addtmp",
                        )
                    }

                    BinaryOperator::Subtract => {
                        self.builder.build_int_sub(
                            lhs,
                            rhs,
                            "subtmp",
                        )
                    }

                    BinaryOperator::Multiply => {
                        self.builder.build_int_mul(
                            lhs,
                            rhs,
                            "multmp",
                        )
                    }

                    BinaryOperator::Divide => {
                        self.builder.build_int_signed_div(
                            lhs,
                            rhs,
                            "divtmp",
                        )
                    }
                }.map_err(|error| error.to_string())?;

                Ok(result)
            }

            HirExpressionData::Call {
                callee,
                arguments,
            } => {
                let llvm_function = self.resolve_function(callee)?;

                let llvm_arguments = arguments
                    .iter()
                    .map(|argument| {
                        self.emit_i64_expression(argument, operands)
                            .map(BasicMetadataValueEnum::from)
                    })
                    .collect::<Result<Vec<_>, String>>()?;

                let call = self.builder
                    .build_call(
                        llvm_function,
                        &llvm_arguments,
                        "calltmp",
                    )
                    .map_err(|error| error.to_string())?;

                let value = call
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| {
                        "expected runtime call to produce an i64 value".to_owned()
                    })?;

                Ok(value.into_int_value())
            }

            _ => Err("expression is not supported by Inkwell backend yet".to_owned()),
        }
    }

    fn resolve_function(
        &self,
        callee: &HirExpression,
    ) -> Result<FunctionValue<'ctx>, String> {
        let HirExpressionData::Symbol(symbol_id) = &callee.data else {
            return Err(
                "Inkwell backend only supports statically known callees rn".to_owned()
            )
        };

        let Some(ComptimeValue::Function(function_id)) =
            self.program.values.get(*symbol_id)
        else {
            return Err(
                "callee does not have a compile-time function value".to_owned()
            );
        };

        self.functions
            .get(function_id)
            .copied()
            .ok_or_else(|| {
                format!(
                    "internal error: function {:?} was not declared",
                    function_id
                )
            })
    }
}

pub fn emit_empty_module() -> Result<String, String> {
    let context = Context::create();
    let module = context.create_module("flang");

    module.verify().map_err(|e| e.to_string())?;

    Ok(module.print_to_string().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inkwell_creates_valid_empty_module() {
        let llvm = emit_empty_module().expect("Failed to emit llvm module");

        assert!(llvm.contains("ModuleID"))
    }
}

fn llvm_function_name(function_id: FunctionId) -> String {
    format!("flang_fn_{}", function_id.index())
}
