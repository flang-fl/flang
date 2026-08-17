use crate::comptime::{ComptimeFunction, ComptimeValue, EvaluatedProgram, FunctionId};
use crate::parser::ast::BinaryOperator;
use crate::semantic::hir::{
    HirBlock, HirElseBranch, HirExpression, HirExpressionData, HirStatement, HirStatementData,
};
use crate::semantic::symbols::{SymbolId, SymbolKind};
use crate::semantic::types::Type;
use inkwell::IntPredicate;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::types::IntType;
use inkwell::values::{BasicMetadataValueEnum, FunctionValue, IntValue, PointerValue};
use std::collections::HashMap;

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

type Operands<'ctx> = HashMap<SymbolId, LocalOperand<'ctx>>;

#[derive(Clone, Copy)]
enum LocalOperand<'ctx> {
    Value(IntValue<'ctx>),
    Mutable(PointerValue<'ctx>),
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
            let return_type = self.llvm_int_type(&function.hir.return_type)?;

            let parameter_types = function
                .hir
                .parameters
                .iter()
                .map(|parameter| self.llvm_int_type(&parameter.type_).map(Into::into))
                .collect::<Result<Vec<_>, String>>()?;

            let function_type = return_type.fn_type(&parameter_types, false);

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
            EmitFlow::Continues => Err("Function body without a return".to_owned()),
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

    fn emit_statement(
        &mut self,
        statement: &HirStatement,
        operands: &mut Operands<'ctx>,
    ) -> Result<EmitFlow, String> {
        match &statement.data {
            HirStatementData::Error => Err("Evil".to_owned()),

            HirStatementData::Assignment { symbol, expression } => {
                let value = self.emit_expression(expression, operands)?;

                let Some(LocalOperand::Mutable(pointer)) = operands.get(symbol) else {
                    return Err("assignment target is not a mutable LLVM local".to_owned());
                };

                self.builder
                    .build_store(*pointer, value)
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

            HirStatementData::Binding { symbol, expression } => {
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
                Err("i64 function cannot return without a value".to_owned())
            }
        }
    }

    fn emit_expression(
        &self,
        expression: &HirExpression,
        operands: &Operands<'ctx>,
    ) -> Result<IntValue<'ctx>, String> {
        match &expression.data {
            HirExpressionData::Integer(value) => {
                Ok(self.context.i64_type().const_int(*value as u64, true))
            }

            HirExpressionData::Symbol(symbol_id) => {
                match operands.get(symbol_id) {
                    Some(LocalOperand::Value(value)) => {
                        return Ok(*value);
                    }

                    Some(LocalOperand::Mutable(pointer)) => {
                        let symbol = self.program.symbols.get(*symbol_id);
                        let llvm_type = self.llvm_int_type(&symbol.type_)?;

                        let loaded = self.builder
                            .build_load(
                                llvm_type,
                                *pointer,
                                "loadtmp"
                            )
                            .map_err(|error| error.to_string())?;

                        return Ok(loaded.into_int_value())
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

            HirExpressionData::Call { callee, arguments } => {
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

                let value = call.try_as_basic_value().basic().ok_or_else(|| {
                    "expected runtime call to produce an integer-like value".to_owned()
                })?;

                Ok(value.into_int_value())
            }

            HirExpressionData::Bool(bool) => {
                Ok(self.context.bool_type().const_int(u64::from(*bool), false))
            }

            HirExpressionData::Function(_) | HirExpressionData::Error => {
                Err("expression is not supported by Inkwell backend yet".to_owned())
            }
        }
    }

    fn resolve_function(&self, callee: &HirExpression) -> Result<FunctionValue<'ctx>, String> {
        let HirExpressionData::Symbol(symbol_id) = &callee.data else {
            return Err("Inkwell backend only supports statically known callees rn".to_owned());
        };

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

    fn llvm_int_type(&self, type_: &Type) -> Result<IntType<'ctx>, String> {
        match type_ {
            Type::I64 => Ok(self.context.i64_type()),
            Type::Bool => Ok(self.context.bool_type()),
            _ => Err(format!("unsupported LLVM value type: {type_:?}")),
        }
    }
}

fn llvm_function_name(function_id: FunctionId) -> String {
    format!("flang_fn_{}", function_id.index())
}

enum EmitFlow {
    Continues,
    Terminates,
}
