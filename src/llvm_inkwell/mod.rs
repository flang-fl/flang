use crate::TargetInfo;
use crate::comptime::{ComptimeFunction, ComptimeValue, FunctionId};
use crate::elaboration::ElaboratedProgram;
use crate::parser::ast::{BinaryOperator, UnaryOperator};
use crate::semantic::hir::{
    HirBlock, HirElseBranch, HirExpression, HirExpressionData, HirPlace, HirPlaceData,
    HirStatement, HirStatementData,
};
use crate::semantic::symbols::{SymbolId, SymbolKind};
use crate::semantic::types::{IntegerType, Type};
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::targets::{
    CodeModel, InitializationConfig, RelocMode, Target, TargetData, TargetMachine,
};
use inkwell::types::{ArrayType, BasicMetadataTypeEnum, BasicType, IntType};
use inkwell::values::{BasicMetadataValueEnum, FunctionValue, IntValue, PointerValue};
use inkwell::{IntPredicate, OptimizationLevel};
use std::collections::HashMap;

pub fn emit(program: &ElaboratedProgram, target_info: TargetInfo) -> Result<String, String> {
    let context = Context::create();
    let mut generator = CodeGenerator::new(&context, program, target_info)?;

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
    program: &'program ElaboratedProgram,
    target: TargetInfo,
    module: Module<'ctx>,
    target_data: TargetData,
    builder: Builder<'ctx>,
    functions: HashMap<FunctionId, FunctionValue<'ctx>>,
    external_functions: HashMap<SymbolId, FunctionValue<'ctx>>,
}

impl<'ctx, 'program> CodeGenerator<'ctx, 'program> {
    pub fn new(
        context: &'ctx Context,
        program: &'program ElaboratedProgram,
        target_info: TargetInfo,
    ) -> Result<Self, String> {
        Target::initialize_native(&InitializationConfig::default())
            .map_err(|error| format!("failed to initialize native LLVM target: {error}"))?;

        let triple = TargetMachine::get_default_triple();

        let llvm_target = Target::from_triple(&triple).map_err(|err| err.to_string())?;

        let target_machine = llvm_target
            .create_target_machine(
                &triple,
                "generic",
                "",
                OptimizationLevel::None,
                RelocMode::Default,
                CodeModel::Default,
            )
            .ok_or_else(|| "failed to create LLVM target machine".to_owned())?;

        let target_data = target_machine.get_target_data();

        let llvm_pointer_width = target_data.get_pointer_byte_size(None) * 8;

        if llvm_pointer_width != target_info.pointer_bit_width {
            return Err(format!(
                "target pointer width mismatch: semantic \
                analysis uses {} bits, but LLVM uses \
                {llvm_pointer_width} bits",
                target_info.pointer_bit_width
            ));
        }

        let module = context.create_module("flang");

        module.set_triple(&triple);

        let data_layout = target_data.get_data_layout();

        module.set_data_layout(&data_layout);

        Ok(Self {
            context,
            program,
            target: target_info,
            module,
            target_data,
            builder: context.create_builder(),
            external_functions: HashMap::new(),
            functions: HashMap::new(),
        })
    }

    fn declare_external_functions(&mut self) -> Result<(), String> {
        for (index, symbol) in self.program.symbols.symbols.iter().enumerate() {
            let SymbolKind::ExternFunction { link_name, abi: _ } = &symbol.kind else {
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
                Type::Integer(_) | Type::Bool => self
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
        let main_symbol_id = self
            .program
            .entry_symbol
            .ok_or_else(|| "entry module does not define `main`".to_owned())?;

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

        if function.hir.return_type != Type::Integer(IntegerType::I64) {
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
                Type::Integer(_) | Type::Bool => self
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

        for (symbol, value) in &function.captures {
            let llvm_value = match value {
                ComptimeValue::Integer { value, type_ } => {
                    let semantic_type = Type::Integer(*type_);

                    let llvm_type = self.llvm_int_type(&semantic_type)?;

                    let bits = integer_bit_pattern(*value, *type_, &self.target);

                    llvm_type.const_int(bits, type_.is_signed())
                }

                ComptimeValue::Bool(value) => {
                    self.context.bool_type().const_int(u64::from(*value), false)
                }

                unsupported => {
                    return Err(format!(
                        "unsupported compile-time capture \
                   in LLVM function: {unsupported:?}"
                    ));
                }
            };

            operands.insert(*symbol, LocalOperand::Value(llvm_value));
        }

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
            } => self.emit_array_element_pointer(*array, index, *array_size, operands),
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
            self.builder
                .build_gep(*llvm_type, *pointer, &[zero, index], "array.element.ptr")
        }
        .map_err(|error| error.to_string())
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
            HirExpressionData::StringLiteral(_) => {
                Err("`str` has no runtime representation".to_owned())
            }

            HirExpressionData::Index { base, index } => {
                let HirExpressionData::Symbol(symbol) = &base.data else {
                    return Err("only local arrays can currently be indexed".to_owned());
                };

                let Type::FixedArray { size, .. } = &base.type_ else {
                    return Err("index base lost its array type".to_owned());
                };

                let element_pointer =
                    self.emit_array_element_pointer(*symbol, index, *size, operands)?;

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
                let integer_type = expression
                    .type_
                    .as_integer()
                    .ok_or_else(|| "integer expression lost its integer type".to_owned())?;

                let llvm_type = self.llvm_int_type(&expression.type_)?;

                let bits = integer_bit_pattern(*value, integer_type, &self.target);

                Ok(llvm_type.const_int(bits, integer_type.is_signed()))
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
                    Some(ComptimeValue::Integer { value, type_ }) => {
                        let semantic_type = Type::Integer(*type_);
                        let llvm_type = self.llvm_int_type(&semantic_type)?;

                        let bits = integer_bit_pattern(*value, *type_, &self.target);

                        Ok(llvm_type.const_int(bits, type_.is_signed()))
                    }

                    Some(ComptimeValue::Bool(value)) => {
                        Ok(self.context.bool_type().const_int(u64::from(*value), false))
                    }

                    _ => Err("symbol has no available LLVM value".to_owned()),
                }
            }

            HirExpressionData::Unary { operator, operand } => {
                let operand = self.emit_expression(operand, operands)?;

                match operator {
                    UnaryOperator::Negate => self
                        .builder
                        .build_int_neg(operand, "negtmp")
                        .map_err(|error| error.to_string()),
                }
            }

            HirExpressionData::Binary { lhs, operator, rhs } => {
                let integer_type = lhs.type_.as_integer();
                let signed = integer_type.is_some_and(IntegerType::is_signed);

                let lhs = self.emit_expression(lhs, operands)?;
                let rhs = self.emit_expression(rhs, operands)?;

                let result = match operator {
                    BinaryOperator::LessThanOrEqual => {
                        let predicate = if signed {
                            IntPredicate::SLE
                        } else {
                            IntPredicate::ULE
                        };

                        self.builder.build_int_compare(predicate, lhs, rhs, "letmp")
                    }

                    BinaryOperator::LessThan => {
                        let predicate = if signed {
                            IntPredicate::SLT
                        } else {
                            IntPredicate::ULT
                        };

                        self.builder.build_int_compare(predicate, lhs, rhs, "lttmp")
                    }

                    BinaryOperator::GreaterThanOrEqual => {
                        let predicate = if signed {
                            IntPredicate::SGE
                        } else {
                            IntPredicate::UGE
                        };

                        self.builder.build_int_compare(predicate, lhs, rhs, "getmp")
                    }

                    BinaryOperator::GreaterThan => {
                        let predicate = if signed {
                            IntPredicate::SGT
                        } else {
                            IntPredicate::UGT
                        };

                        self.builder.build_int_compare(predicate, lhs, rhs, "gttmp")
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

                    BinaryOperator::Divide if signed => {
                        self.builder.build_int_signed_div(lhs, rhs, "divtmp")
                    }

                    BinaryOperator::Divide => {
                        self.builder.build_int_unsigned_div(lhs, rhs, "divtmp")
                    }
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

            HirExpressionData::FunctionTemplate(_) => {
                Err("an unspecialized function template reached LLVM value emission".to_owned())
            }

            HirExpressionData::Function(_)
            | HirExpressionData::Error
            | HirExpressionData::KnownFunction(_) => {
                Err("expression is not supported by Inkwell backend yet".to_owned())
            }

            HirExpressionData::TypeValue(_) => {
                Err("type values have no runtime representation".to_owned())
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

        let index_type = index.get_type();

        let length = index_type.const_int(array_size as u64, false);

        let valid = self
            .builder
            .build_int_compare(IntPredicate::ULT, index, length, "index.in_bounds")
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
        match &callee.data {
            HirExpressionData::KnownFunction(function_id) => {
                self.functions.get(function_id).copied().ok_or_else(|| {
                    format!(
                        "specialized function {:?} \
                           was not declared",
                        function_id,
                    )
                })
            }

            HirExpressionData::Symbol(symbol_id) => {
                if let Some(function) = self.external_functions.get(symbol_id) {
                    return Ok(*function);
                }

                match self.program.values.get(*symbol_id) {
                    Some(ComptimeValue::Function(function_id)) => self
                        .functions
                        .get(function_id)
                        .copied()
                        .ok_or_else(|| format!("function {:?} was not declared", function_id)),

                    Some(ComptimeValue::ExternFunction(external_symbol)) => self
                        .external_functions
                        .get(external_symbol)
                        .copied()
                        .ok_or_else(|| {
                            format!(
                                "external function symbol {:?} was not declared",
                                external_symbol
                            )
                        }),

                    _ => Err("callee does not have a compile-time function value".to_owned()),
                }
            }

            other => Err(format!(
                "Inkwell backend only supports \
               statically known callees, found {other:?}"
            )),
        }
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
        use IntegerType::*;
        use Type::Integer;
        match type_ {
            Integer(U8 | I8) => Ok(self.context.i8_type()),
            Integer(U16 | I16) => Ok(self.context.i16_type()),
            Integer(U32 | I32) => Ok(self.context.i32_type()),
            Integer(U64 | I64) => Ok(self.context.i64_type()),

            Integer(Usize | Isize) => {
                let llvm_type = self.context.ptr_sized_int_type(&self.target_data, None);

                if llvm_type.get_bit_width() != self.target.pointer_bit_width {
                    return Err(format!(
                        "LLVM pointer-sized integer has width {}, \
                        but semantic analysis expects {}",
                        llvm_type.get_bit_width(),
                        self.target.pointer_bit_width
                    ));
                }

                Ok(llvm_type)
            }

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

fn integer_bit_pattern(value: i128, integer_type: IntegerType, target: &TargetInfo) -> u64 {
    let width = integer_type.bit_width(target);

    if width == 64 {
        value as u64
    } else {
        let mask = (1u64 << width) - 1;
        (value as u64) & mask
    }
}

enum EmitFlow {
    Continues,
    Terminates,
}
