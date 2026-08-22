use crate::comptime::{ComptimeFunction, ComptimeValue, EvaluatedProgram, FunctionId};
use crate::parser::ast::BinaryOperator;
use crate::semantic::hir::{
    HirBlock, HirElseBranch, HirExpression, HirExpressionData, HirPlace, HirPlaceData,
    HirStatement, HirStatementData,
};
use crate::semantic::symbols::{SymbolId, SymbolKind};
use crate::semantic::types::Type;
use inkwell::IntPredicate;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::types::{ArrayType, BasicMetadataTypeEnum, BasicType, IntType};
use inkwell::values::{BasicMetadataValueEnum, FunctionValue, IntValue, PointerValue};
use std::collections::HashMap;

pub fn emit(program: &EvaluatedProgram) -> Result<String, String> {
    let context = Context::create();
    let mut generator = CodeGenerator::new(&context, program);

    generator.declare_external_functions()?;
    generator.declare_functions()?;
    generator.emit_function_bodies()?;
    generator.emit_main_wrapper()?;

    generator
        .module
        .verify()
        .map_err(|message| message.to_string())?;

    Ok(generator.module.print_to_string().to_string())
}

type Operands<'ctx> = HashMap<SymbolId, LocalOperand<'ctx>>;

#[derive(Clone, Copy)]
enum LocalOperand<'ctx> {
    Value(IntValue<'ctx>),
    Mutable(PointerValue<'ctx>),

    Array {
        pointer: PointerValue<'ctx>,
        llvm_type: ArrayType<'ctx>,
    },
}

pub struct CodeGenerator<'ctx, 'program> {
    context: &'ctx Context,
    program: &'program EvaluatedProgram,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    functions: HashMap<FunctionId, FunctionValue<'ctx>>,
    external_functions: HashMap<SymbolId, FunctionValue<'ctx>>,
}

impl<'ctx, 'program> CodeGenerator<'ctx, 'program> {
    pub fn new(context: &'ctx Context, program: &'program EvaluatedProgram) -> Self {
        Self {
            context,
            program,
            module: context.create_module("flang"),
            builder: context.create_builder(),
            external_functions: HashMap::new(),
            functions: HashMap::new(),
        }
    }

    fn declare_external_functions(&mut self) -> Result<(), String> {
        for (index, symbol) in self.program.symbols.symbols.iter().enumerate() {
            let SymbolKind::ExternFunction { link_name } = &symbol.kind else {
                continue;
            };

            let Type::Function {
                parameters,
                return_type,
            } = &symbol.type_
            else {
                return Err("external function symbol does not have a function type".to_owned());
            };

            let parameter_types = parameters
                .iter()
                .map(|parameter| self.llvm_int_type(parameter).map(Into::into))
                .collect::<Result<Vec<BasicMetadataTypeEnum>, String>>()?;

            let function_type = match return_type.as_ref() {
                Type::I64 | Type::Bool => self
                    .llvm_int_type(return_type)?
                    .fn_type(&parameter_types, false),

                Type::Unit => self.context.void_type().fn_type(&parameter_types, false),

                unsupported => {
                    return Err(format!("unsupported external return type {unsupported:?}"));
                }
            };

            let llvm_function = self.module.add_function(link_name, function_type, None);

            self.external_functions
                .insert(SymbolId(index as u32), llvm_function);
        }

        Ok(())
    }

    fn emit_main_wrapper(&self) -> Result<(), String> {
        let (main_symbol_id, _) = self
            .program
            .symbols
            .find_by_name("main")
            .ok_or_else(|| "program does not define `main`".to_owned())?;

        let function_id = match self.program.values.get(main_symbol_id) {
            Some(ComptimeValue::Function(function_id)) => *function_id,

            _ => {
                return Err("`main` must be a compile-time-known function".to_owned());
            }
        };

        let function = self
            .program
            .functions
            .get(function_id)
            .ok_or_else(|| "internal error: main FunctionId is missing".to_owned())?;

        if !function.hir.parameters.is_empty() {
            return Err("`main` must not take parameters".to_owned());
        }

        if function.hir.return_type != Type::I64 {
            return Err("`main` must return i64".to_owned());
        }

        let language_main = self
            .functions
            .get(&function_id)
            .copied()
            .ok_or_else(|| "internal error: main was not declared in LLVM".to_owned())?;

        let i32_type = self.context.i32_type();
        let wrapper_type = i32_type.fn_type(&[], false);

        let wrapper = self.module.add_function("main", wrapper_type, None);

        let entry = self.context.append_basic_block(wrapper, "entry");

        self.builder.position_at_end(entry);

        let call = self
            .builder
            .build_call(language_main, &[], "main_result")
            .map_err(|error| error.to_string())?;

        let result = call
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "language main did not produce a value".to_owned())?
            .into_int_value();

        let status = self
            .builder
            .build_int_truncate(result, i32_type, "status")
            .map_err(|error| error.to_string())?;

        self.builder
            .build_return(Some(&status))
            .map_err(|error| error.to_string())?;

        Ok(())
    }

    fn declare_functions(&mut self) -> Result<(), String> {
        for (function_id, function) in self.program.functions.iter() {
            let parameter_types = function
                .hir
                .parameters
                .iter()
                .map(|parameter| self.llvm_int_type(&parameter.type_).map(Into::into))
                .collect::<Result<Vec<BasicMetadataTypeEnum>, String>>()?;

            let function_type = match &function.hir.return_type {
                Type::I64 | Type::Bool => self
                    .llvm_int_type(&function.hir.return_type)?
                    .fn_type(&parameter_types, false),

                Type::Unit => self.context.void_type().fn_type(&parameter_types, false),

                unsupported => {
                    return Err(format!(
                        "unsupported LLVM function return type: `{:?}`",
                        unsupported
                    ));
                }
            };

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

        let mut operands = Operands::<'ctx>::new();

        for (index, (parameter, llvm_parameter)) in function
            .hir
            .parameters
            .iter()
            .zip(llvm_function.get_param_iter())
            .enumerate()
        {
            let llvm_parameter = llvm_parameter.into_int_value();
            llvm_parameter.set_name(&format!("arg{index}"));

            operands.insert(parameter.symbol, LocalOperand::Value(llvm_parameter));
        }

        let flow = self.emit_block(&function.hir.body, &mut operands)?;

        match flow {
            EmitFlow::Continues => {
                if function.hir.return_type == Type::Unit {
                    self.builder
                        .build_return(None)
                        .map_err(|error| error.to_string())?;

                    Ok(())
                } else {
                    Err("Function body without a return".to_owned())
                }
            }
            EmitFlow::Terminates => Ok(()),
        }
    }

    fn emit_block(
        &mut self,
        block: &HirBlock,
        operands: &mut Operands<'ctx>,
    ) -> Result<EmitFlow, String> {
        for statement in block.statements.iter() {
            let flow = self.emit_statement(statement, operands)?;
            match flow {
                EmitFlow::Continues => {}
                flow => return Ok(flow),
            }
        }

        Ok(EmitFlow::Continues)
    }

    fn emit_place(
        &self,
        place: &HirPlace,
        operands: &Operands<'ctx>,
    ) -> Result<PointerValue<'ctx>, String> {
        match &place.data {
            HirPlaceData::Symbol(symbol) => match operands.get(symbol) {
                Some(LocalOperand::Mutable(pointer)) => Ok(*pointer),

                _ => Err("symbol place does not have mutable LLVM storage".to_owned()),
            },

            HirPlaceData::Index {
                array,
                index,
                array_size,
            } => {
                self.emit_array_element_pointer(
                    *array,
                    index,
                    *array_size,
                    operands,
                )
            }
        }
    }

    fn emit_array_element_pointer(
        &self,
        array: SymbolId,
        index: &HirExpression,
        array_size: usize,
        operands: &Operands<'ctx>,
    ) -> Result<PointerValue<'ctx>, String> {
        let Some(LocalOperand::Array { pointer, llvm_type }) = operands.get(&array) else {
            return Err("array has no local LLVM storage".to_owned());
        };

        let index = self.emit_expression(index, operands)?;

        self.emit_array_bounds_check(index, array_size)?;

        let zero = self.context.i64_type().const_zero();

        unsafe {
            self.builder.build_gep(
                *llvm_type,
                *pointer,
                &[zero, index],
                "array.element.ptr"
            )
        }.map_err(|error| error.to_string())
    }

    fn emit_statement(
        &mut self,
        statement: &HirStatement,
        operands: &mut Operands<'ctx>,
    ) -> Result<EmitFlow, String> {
        match &statement.data {
            HirStatementData::Error => Err("Evil".to_owned()),

            HirStatementData::Expression(expression) => {
                let HirExpressionData::Call { callee, arguments } = &expression.data else {
                    return Err("unsupported unit expression statement".to_owned());
                };

                let value = self.emit_call(operands, callee, arguments)?;

                if value.is_some() {
                    return Err("value-producing expression used as a unit statement".to_owned());
                }

                Ok(EmitFlow::Continues)
            }

            HirStatementData::Assignment { target, expression } => {
                let pointer = self.emit_place(target, operands)?;
                let value = self.emit_expression(expression, operands)?;

                self.builder
                    .build_store(pointer, value)
                    .map_err(|error| error.to_string())?;

                Ok(EmitFlow::Continues)
            }

            HirStatementData::If {
                condition,
                then_block,
                else_branch,
            } => {
                let condition = self.emit_expression(condition, operands)?;

                let llvm_function = self
                    .builder
                    .get_insert_block()
                    .and_then(|block| block.get_parent())
                    .ok_or_else(|| "if emitted outside an LLVM function".to_owned())?;

                let then_bb = self.context.append_basic_block(llvm_function, "if.then");

                let else_bb = self.context.append_basic_block(llvm_function, "if.else");

                let merge_bb = self.context.append_basic_block(llvm_function, "if.end");

                self.builder
                    .build_conditional_branch(condition, then_bb, else_bb)
                    .map_err(|error| error.to_string())?;

                // Then Branch
                self.builder.position_at_end(then_bb);

                let mut then_operands = operands.clone();
                let then_flow = self.emit_block(then_block, &mut then_operands)?;

                let then_continues = matches!(then_flow, EmitFlow::Continues);

                if then_continues {
                    self.builder
                        .build_unconditional_branch(merge_bb)
                        .map_err(|error| error.to_string())?;
                }

                // Else Branch
                self.builder.position_at_end(else_bb);

                let mut else_operands = operands.clone();

                let else_flow = match else_branch {
                    Some(HirElseBranch::Else(block)) => {
                        self.emit_block(block, &mut else_operands)?
                    }
                    Some(HirElseBranch::ElseIf(statement)) => {
                        self.emit_statement(statement, &mut else_operands)?
                    }

                    None => EmitFlow::Continues,
                };

                let else_continues = matches!(else_flow, EmitFlow::Continues);

                if else_continues {
                    self.builder
                        .build_unconditional_branch(merge_bb)
                        .map_err(|error| error.to_string())?;
                }

                // Merge Block
                if then_continues || else_continues {
                    self.builder.position_at_end(merge_bb);
                    Ok(EmitFlow::Continues)
                } else {
                    // The block exists but neither branch
                    // can reach it
                    self.builder.position_at_end(merge_bb);

                    self.builder
                        .build_unreachable()
                        .map_err(|error| error.to_string())?;

                    Ok(EmitFlow::Terminates)
                }
            }

            HirStatementData::While {
                condition,
                while_block,
            } => {
                let llvm_function = self
                    .builder
                    .get_insert_block()
                    .and_then(|block| block.get_parent())
                    .ok_or_else(|| "while emitted outside an LLVM function".to_owned())?;

                let condition_bb = self
                    .context
                    .append_basic_block(llvm_function, "while.condition");

                let body_bb = self.context.append_basic_block(llvm_function, "while.body");

                let end_bb = self.context.append_basic_block(llvm_function, "while.end");

                self.builder
                    .build_unconditional_branch(condition_bb)
                    .map_err(|error| error.to_string())?;

                self.builder.position_at_end(condition_bb);

                let condition_value = self.emit_expression(condition, operands)?;

                self.builder
                    .build_conditional_branch(condition_value, body_bb, end_bb)
                    .map_err(|error| error.to_string())?;

                self.builder.position_at_end(body_bb);

                let mut body_operands = operands.clone();
                let body_flow = self.emit_block(while_block, &mut body_operands)?;

                if matches!(body_flow, EmitFlow::Continues) {
                    self.builder
                        .build_unconditional_branch(condition_bb)
                        .map_err(|error| error.to_string())?;
                }

                self.builder.position_at_end(end_bb);

                Ok(EmitFlow::Continues)
            }

            HirStatementData::Binding { symbol, expression } => {
                if matches!(
                    expression.data,
                    HirExpressionData::ArrayRepeatInitialization { .. }
                ) {
                    self.emit_zero_array_binding(*symbol, expression, operands)?;
                    return Ok(EmitFlow::Continues);
                }

                let value = self.emit_expression(expression, &operands)?;
                let symbol_info = self.program.symbols.get(*symbol);

                match &symbol_info.kind {
                    SymbolKind::Local { mutable: true } => {
                        let pointer = self
                            .builder
                            .build_alloca(value.get_type(), &symbol_info.name)
                            .map_err(|error| error.to_string())?;

                        self.builder
                            .build_store(pointer, value)
                            .map_err(|error| error.to_string())?;

                        operands.insert(*symbol, LocalOperand::Mutable(pointer));
                    }

                    _ => {
                        operands.insert(*symbol, LocalOperand::Value(value));
                    }
                }

                Ok(EmitFlow::Continues)
            }

            HirStatementData::Return(Some(expression)) => {
                let value = self.emit_expression(expression, &operands)?;

                self.builder
                    .build_return(Some(&value))
                    .map_err(|error| error.to_string())?;

                Ok(EmitFlow::Terminates)
            }

            HirStatementData::Return(None) => {
                self.builder
                    .build_return(None)
                    .map_err(|error| error.to_string())?;

                Ok(EmitFlow::Terminates)
            }
        }
    }

    fn emit_expression(
        &self,
        expression: &HirExpression,
        operands: &Operands<'ctx>,
    ) -> Result<IntValue<'ctx>, String> {
        match &expression.data {
            HirExpressionData::Index { base, index } => {
                let HirExpressionData::Symbol(symbol) = &base.data else {
                    return Err("only local arrays can currently be indexed".to_owned());
                };

                let Type::FixedArray { size, .. } = &base.type_ else {
                    return Err("index base lost its array type".to_owned());
                };

                let element_pointer = self.emit_array_element_pointer(
                    *symbol,
                    index,
                    *size,
                    operands
                )?;

                let element_type = self.llvm_int_type(&expression.type_)?;

                let element = self
                    .builder
                    .build_load(element_type, element_pointer, "array.element")
                    .map_err(|error| error.to_string())?
                    .into_int_value();

                Ok(element)
            }

            HirExpressionData::ArrayRepeatInitialization { .. } => {
                unreachable!()
            }

            HirExpressionData::Integer(value) => {
                Ok(self.context.i64_type().const_int(*value as u64, true))
            }

            HirExpressionData::Symbol(symbol_id) => {
                match operands.get(symbol_id) {
                    Some(LocalOperand::Array { llvm_type, pointer }) => {
                        todo!("IDK what to do here #001");
                    }

                    Some(LocalOperand::Value(value)) => {
                        return Ok(*value);
                    }

                    Some(LocalOperand::Mutable(pointer)) => {
                        let symbol = self.program.symbols.get(*symbol_id);
                        let llvm_type = self.llvm_int_type(&symbol.type_)?;

                        let loaded = self
                            .builder
                            .build_load(llvm_type, *pointer, "loadtmp")
                            .map_err(|error| error.to_string())?;

                        return Ok(loaded.into_int_value());
                    }

                    None => {}
                }

                match self.program.values.get(*symbol_id) {
                    Some(ComptimeValue::I64(value)) => {
                        Ok(self.context.i64_type().const_int(*value as u64, true))
                    }

                    Some(ComptimeValue::Bool(value)) => {
                        Ok(self.context.bool_type().const_int(u64::from(*value), false))
                    }

                    _ => Err("symbol has no available LLVM value".to_owned()),
                }
            }

            HirExpressionData::Binary { lhs, operator, rhs } => {
                let lhs = self.emit_expression(lhs, operands)?;
                let rhs = self.emit_expression(rhs, operands)?;

                let result = match operator {
                    BinaryOperator::LessThanOrEqual => {
                        self.builder
                            .build_int_compare(IntPredicate::SLE, lhs, rhs, "sletmp")
                    }

                    BinaryOperator::LessThan => {
                        self.builder
                            .build_int_compare(IntPredicate::SLT, lhs, rhs, "slttmp")
                    }

                    BinaryOperator::GreaterThanOrEqual => {
                        self.builder
                            .build_int_compare(IntPredicate::SGE, lhs, rhs, "sgetmp")
                    }

                    BinaryOperator::GreaterThan => {
                        self.builder
                            .build_int_compare(IntPredicate::SGT, lhs, rhs, "sgttmp")
                    }

                    BinaryOperator::NotEqual => {
                        self.builder
                            .build_int_compare(IntPredicate::NE, lhs, rhs, "neqtmp")
                    }

                    BinaryOperator::Equal => {
                        self.builder
                            .build_int_compare(IntPredicate::EQ, lhs, rhs, "eqltmp")
                    }

                    BinaryOperator::Add => self.builder.build_int_add(lhs, rhs, "addtmp"),

                    BinaryOperator::Subtract => self.builder.build_int_sub(lhs, rhs, "subtmp"),

                    BinaryOperator::Multiply => self.builder.build_int_mul(lhs, rhs, "multmp"),

                    BinaryOperator::Divide => self.builder.build_int_signed_div(lhs, rhs, "divtmp"),
                }
                .map_err(|error| error.to_string())?;

                Ok(result)
            }

            HirExpressionData::Call { callee, arguments } => self
                .emit_call(operands, callee, arguments)?
                .ok_or_else(|| "unit-returning call used as a value".to_owned()),

            HirExpressionData::Bool(bool) => {
                Ok(self.context.bool_type().const_int(u64::from(*bool), false))
            }

            HirExpressionData::Function(_) | HirExpressionData::Error => {
                Err("expression is not supported by Inkwell backend yet".to_owned())
            }
        }
    }

    fn emit_array_bounds_check(
        &self,
        index: IntValue<'ctx>,
        array_size: usize,
    ) -> Result<(), String> {
        let current_block = self
            .builder
            .get_insert_block()
            .ok_or_else(|| "bounds check emitted outside a block".to_owned())?;

        let function = current_block
            .get_parent()
            .ok_or_else(|| "bounds check emitted outside a function".to_owned())?;

        let valid_block = self.context.append_basic_block(function, "index.valid");

        let invalid_block = self.context.append_basic_block(function, "index.invalid");

        let i64_type = self.context.i64_type();
        let zero = i64_type.const_zero();
        let length = i64_type.const_int(array_size as u64, false);

        let nonnegative = self
            .builder
            .build_int_compare(IntPredicate::SGE, index, zero, "index.nonnegative")
            .map_err(|error| error.to_string())?;

        let below_length = self
            .builder
            .build_int_compare(IntPredicate::SLT, index, length, "index.below_length")
            .map_err(|error| error.to_string())?;

        let valid = self
            .builder
            .build_and(nonnegative, below_length, "index.in_bounds")
            .map_err(|error| error.to_string())?;

        self.builder
            .build_conditional_branch(valid, valid_block, invalid_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(invalid_block);

        let trap = self.module.get_function("llvm.trap").unwrap_or_else(|| {
            let trap_type = self.context.void_type().fn_type(&[], false);
            self.module.add_function("llvm.trap", trap_type, None)
        });

        self.builder
            .build_call(trap, &[], "")
            .map_err(|error| error.to_string())?;

        self.builder
            .build_unreachable()
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(valid_block);

        Ok(())
    }

    fn emit_call(
        &self,
        operands: &Operands<'ctx>,
        callee: &HirExpression,
        arguments: &[HirExpression],
    ) -> Result<Option<IntValue<'ctx>>, String> {
        let llvm_function = self.resolve_function(callee)?;

        let llvm_arguments = arguments
            .iter()
            .map(|argument| {
                self.emit_expression(argument, operands)
                    .map(BasicMetadataValueEnum::from)
            })
            .collect::<Result<Vec<_>, String>>()?;

        let call = self
            .builder
            .build_call(llvm_function, &llvm_arguments, "calltmp")
            .map_err(|error| error.to_string())?;

        let value = call
            .try_as_basic_value()
            .basic()
            .map(|value| value.into_int_value());

        Ok(value)
    }

    fn resolve_function(&self, callee: &HirExpression) -> Result<FunctionValue<'ctx>, String> {
        let HirExpressionData::Symbol(symbol_id) = &callee.data else {
            return Err("Inkwell backend only supports statically known callees rn".to_owned());
        };

        if let Some(function) = self.external_functions.get(symbol_id) {
            return Ok(*function);
        }

        let Some(ComptimeValue::Function(function_id)) = self.program.values.get(*symbol_id) else {
            return Err("callee does not have a compile-time function value".to_owned());
        };

        self.functions.get(function_id).copied().ok_or_else(|| {
            format!(
                "internal error: function {:?} was not declared",
                function_id
            )
        })
    }

    fn emit_zero_array_binding(
        &mut self,
        symbol: SymbolId,
        expression: &HirExpression,
        operands: &mut Operands<'ctx>,
    ) -> Result<(), String> {
        let HirExpressionData::ArrayRepeatInitialization { value, .. } = &expression.data else {
            return Err("expected array repeat initializer".to_owned());
        };

        let HirExpressionData::Integer(0) = value.data else {
            return Err("only `[0; N]` array initialization is supported for now".to_owned());
        };

        let symbol_info = self.program.symbols.get(symbol);
        let array_type = self.llvm_array_type(&expression.type_)?;

        let pointer = self
            .builder
            .build_alloca(array_type, &symbol_info.name)
            .map_err(|error| error.to_string())?;

        self.builder
            .build_store(pointer, array_type.const_zero())
            .map_err(|error| error.to_string())?;

        operands.insert(
            symbol,
            LocalOperand::Array {
                pointer,
                llvm_type: array_type,
            },
        );

        Ok(())
    }

    fn llvm_int_type(&self, type_: &Type) -> Result<IntType<'ctx>, String> {
        match type_ {
            Type::I64 => Ok(self.context.i64_type()),
            Type::Bool => Ok(self.context.bool_type()),
            _ => Err(format!("unsupported LLVM value type: {type_:?}")),
        }
    }

    fn llvm_array_type(&self, type_: &Type) -> Result<ArrayType<'ctx>, String> {
        let Type::FixedArray { size, base_type } = type_ else {
            return Err(format!("expected fixed array type, found {type_:?}"));
        };

        let size = u32::try_from(*size)
            .map_err(|_| "array length does not fit in LLVM's array length".to_owned())?;

        let element_type = self.llvm_int_type(base_type)?;

        Ok(element_type.array_type(size))
    }
}

fn llvm_function_name(function_id: FunctionId) -> String {
    format!("flang_fn_{}", function_id.index())
}

enum EmitFlow {
    Continues,
    Terminates,
}
