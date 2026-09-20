use super::Elaborator;
use crate::comptime::{
    ComptimeFunction, ComptimeValue, FunctionId, FunctionTemplate, FunctionTemplateId,
};
use crate::diagnostics::{Diagnostic, Diagnostics, Label};
use crate::parser::ast::{
    BinaryOperator, Binding, Block, ElseBranch, Expression, ExpressionData, FunctionExpression, If,
    Phase, Statement, StatementData, TypeExpression, TypeExpressionData, UnaryOperator, Visibility,
    While,
};
use crate::semantic::hir::{
    HirBinding, HirBlock, HirElseBranch, HirExpression, HirExpressionData, HirFunctionExpression,
    HirParameter, HirPlace, HirPlaceData, HirStatement, HirStatementData,
};
use crate::semantic::symbols::{ExternAbi, Symbol, SymbolId, SymbolKind};
use crate::semantic::types::{ComptimeKey, IntegerType, SpecializationKey, Type};
use crate::source::Span;
use std::collections::{HashMap, HashSet};

impl Elaborator<'_> {
    pub(super) fn analyze_expression(
        &mut self,
        expression: &Expression,
        expected: Option<&Type>,
    ) -> HirExpression {
        match &expression.data {
            ExpressionData::Import { path } => {
                let Some(module) = self.import_targets.get(path).copied() else {
                    self.diagnostics.error(
                        "Import was not resolved",
                        *path,
                        "The import must be loaded before elaboration",
                    );

                    return hir_error(expression.span);
                };

                let actual_type = Type::Module(module);

                if let Some(expected_type) = expected {
                    if *expected_type != Type::Error && *expected_type != actual_type {
                        self.diagnostics.type_mismatch(expression.span, expected_type, actual_type);
                        return hir_error(expression.span);
                    }
                }

                HirExpression {
                    span: expression.span,
                    type_: actual_type,
                    data: HirExpressionData::Module(module),
                }
            }

            ExpressionData::Member { base, name } => {
                let base_hir = self.analyze_expression(base, None);

                if base_hir.type_ == Type::Error {
                    return hir_error(expression.span);
                }

                if !matches!(&base_hir.type_, Type::Module(_)) {
                    self.diagnostics.error(
                        "Member access requires a module",
                        base.span,
                        format!("Expected a module, found `{:?}`", base_hir.type_),
                    );

                    return hir_error(expression.span);
                }

                let module = match self.evaluate_expression(&base_hir) {
                    ComptimeValue::Module(module) => module,

                    ComptimeValue::Error => {
                        return hir_error(expression.span);
                    }

                    _ => {
                        self.diagnostics.error(
                            "Expected a comptime module value",
                            base.span,
                            "The expression did not evaluate to a module",
                        );

                        return hir_error(expression.span);
                    }
                };

                let scope = self.modules.get(module).scope;
                let member_name = self.sources.span_text(*name);

                let Some(symbol) = self.environment.lookup_in(scope, member_name) else {
                    self.diagnostics.error(
                        "Unknown module member",
                        *name,
                        format!("This module does not declare `{member_name}`"),
                    );

                    return hir_error(expression.span);
                };

                if self.symbols.get(symbol).visibility != Visibility::Public {
                    self.diagnostics.error(
                        "Module member is private",
                        *name,
                        format!("`{member_name}` is not declared with `pub`"),
                    );

                    return hir_error(expression.span);
                }

                self.analyze_symbol_reference(symbol, expression.span, expected)
            }

            ExpressionData::Intrinsic { name } => {
                self.diagnostics.error(
                    "Intrinsics are currently unsupported",
                    expression.span,
                    ":(",
                );

                hir_error(expression.span)
            }

            ExpressionData::TypeValue(type_expression) => {
                let value = self.resolve_type_expression(type_expression);

                if value == Type::Error {
                    return hir_error(expression.span);
                }

                if let Some(expected) = expected
                    && *expected != Type::Type
                    && *expected != Type::Error
                {
                    self.diagnostics.type_mismatch(expression.span, expected, "type");
                    return hir_error(expression.span);
                }

                HirExpression {
                    span: expression.span,
                    type_: Type::Type,
                    data: HirExpressionData::TypeValue(value),
                }
            }

            ExpressionData::StringLiteral => {
                let start = expression.span.start + 1; // cut off left "
                let end = expression.span.end - 1; // cut off right "
                let content = self
                    .sources
                    .span_text(
                        self.sources
                            .get_file(expression.span.source)
                            .span(start, end),
                    )
                    .to_string();

                if let Some(type_) = expected
                    && *type_ != Type::Str
                {
                    self.diagnostics.error(
                        "Type mismatch",
                        expression.span,
                        format!("expected `{:?}` but got `{:?}`", type_, Type::Str),
                    );

                    return hir_error(expression.span);
                }

                return HirExpression {
                    span: expression.span,
                    data: HirExpressionData::StringLiteral(content),
                    type_: Type::Str,
                };
            }

            ExpressionData::Index { base, index } => {
                let base = self.analyze_expression(base, None);
                let index =
                    self.analyze_expression(index, Some(&Type::Integer(IntegerType::Usize)));

                if base.type_ == Type::Error || index.type_ == Type::Error {
                    return hir_error(expression.span);
                }

                let (element_type, array_size) = match &base.type_ {
                    Type::FixedArray { base_type, size } => (base_type.as_ref().clone(), *size),

                    other => {
                        self.diagnostics.error(
                            "Value is not indexable",
                            base.span,
                            format!("expected an array, found `{other:?}`"),
                        );

                        return hir_error(expression.span);
                    }
                };

                if let Some(expected) = expected {
                    if *expected != Type::Error && *expected != element_type {
                        self.diagnostics.type_mismatch(expression.span, expected, element_type);
                        return hir_error(expression.span);
                    }
                }

                if let HirExpressionData::Integer(index_value) = &index.data {
                    let valid_index =
                        usize::try_from(*index_value).is_ok_and(|index| index < array_size);

                    if !valid_index {
                        self.diagnostics.error(
                            "Array index out of bounds",
                            index.span,
                            format!("array length is {array_size}, but the index is {index_value}"),
                        );

                        return hir_error(expression.span);
                    }
                }

                HirExpression {
                    span: expression.span,
                    type_: element_type,
                    data: HirExpressionData::Index {
                        base: Box::new(base),
                        index: Box::new(index),
                    },
                }
            }

            ExpressionData::ArrayRepeatInitialization { value, size } => {
                let amount: usize = match self.resolve_static_array_length(size) {
                    Some(amount) => amount,
                    None => return hir_error(expression.span),
                };

                let base_type = if let Some(expected) = expected {
                    match expected {
                        Type::FixedArray { base_type, size } => {
                            if *size != amount {
                                self.diagnostics.error(
                                    "Array Length Mismatch",
                                    expression.span,
                                    format!(
                                        "Expected Array of length {size} but got length {amount}"
                                    ),
                                );
                                return hir_error(expression.span);
                            }
                            Some(base_type.as_ref())
                        }

                        _ => {
                            self.diagnostics.type_mismatch(expression.span, expected, "array");
                            return hir_error(expression.span);
                        }
                    }
                } else {
                    None
                };

                let value = self.analyze_expression(value.as_ref(), base_type);

                HirExpression {
                    span: expression.span,
                    type_: Type::FixedArray {
                        size: amount,
                        base_type: Box::new(value.type_.clone()),
                    },
                    data: HirExpressionData::ArrayRepeatInitialization {
                        amount,
                        value: Box::new(value),
                    },
                }
            }

            ExpressionData::Boolean(bool) => {
                if let Some(expected) = expected {
                    if *expected != Type::Bool {
                        self.diagnostics.type_mismatch(expression.span, expected, Type::Bool);
                        return hir_error(expression.span);
                    }
                }

                HirExpression {
                    span: expression.span,
                    type_: Type::Bool,
                    data: HirExpressionData::Bool(*bool),
                }
            }

            ExpressionData::Binary { lhs, operator, rhs } => {
                let expected_integer_type = expected.filter(|expected| expected.is_integer());

                let lhs = match operator {
                    BinaryOperator::Add
                    | BinaryOperator::Subtract
                    | BinaryOperator::Multiply
                    | BinaryOperator::Divide => self.analyze_expression(lhs, expected_integer_type),

                    _ => self.analyze_expression(lhs, None),
                };

                if lhs.type_ == Type::Error {
                    return hir_error(expression.span);
                }

                let supports_equality = lhs.type_.is_integer() || lhs.type_ == Type::Bool;
                if matches!(operator, BinaryOperator::Equal | BinaryOperator::NotEqual)
                    && !supports_equality
                {
                    self.diagnostics.error(
                        "Type does not support equality",
                        expression.span,
                        format!("found operands of type `{:?}`", lhs.type_),
                    )
                }

                let rhs = self.analyze_expression(rhs, Some(&lhs.type_));

                if rhs.type_ == Type::Error {
                    return hir_error(expression.span);
                }

                if lhs.type_ != rhs.type_ {
                    self.diagnostics.error(
                        "Binary operand type mismatch",
                        expression.span,
                        format!(
                            "left operand has type `{:?}`, but right operand has type `{:?}`",
                            lhs.type_, rhs.type_
                        ),
                    );

                    return hir_error(expression.span);
                }

                let requires_integer = matches!(
                    operator,
                    BinaryOperator::Add
                        | BinaryOperator::Subtract
                        | BinaryOperator::Multiply
                        | BinaryOperator::Divide
                        | BinaryOperator::LessThan
                        | BinaryOperator::GreaterThan
                        | BinaryOperator::LessThanOrEqual
                        | BinaryOperator::GreaterThanOrEqual
                );

                if requires_integer && !lhs.type_.is_integer() {
                    self.diagnostics.error(
                        "Operator requires integer operands",
                        expression.span,
                        format!("found operands of type `{:?}`", lhs.type_),
                    );

                    return hir_error(expression.span);
                }

                let resulting_type = match operator {
                    BinaryOperator::Equal
                    | BinaryOperator::NotEqual
                    | BinaryOperator::LessThan
                    | BinaryOperator::GreaterThan
                    | BinaryOperator::LessThanOrEqual
                    | BinaryOperator::GreaterThanOrEqual => Type::Bool,

                    BinaryOperator::Add
                    | BinaryOperator::Subtract
                    | BinaryOperator::Multiply
                    | BinaryOperator::Divide => lhs.type_.clone(),
                };

                if let Some(expected) = expected {
                    if *expected != resulting_type {
                        self.diagnostics.error(
                            "Type mismatch",
                            expression.span,
                            format!(
                                "Expected `{:?}`, but this expression produces `{:?}`",
                                expected, resulting_type
                            ),
                        );

                        return hir_error(expression.span);
                    }
                }

                HirExpression {
                    span: expression.span,
                    type_: resulting_type,
                    data: HirExpressionData::Binary {
                        lhs: Box::new(lhs),
                        operator: *operator,
                        rhs: Box::new(rhs),
                    },
                }
            }

            ExpressionData::Unary { operator, operand } => {
                if *operator == UnaryOperator::Negate
                    && matches!(&operand.data, ExpressionData::IntegerLiteral)
                {
                    return self.analyze_integer_literal(operand, expression.span, expected, true);
                }

                let operand = self.analyze_expression(operand, expected);

                if operand.type_ == Type::Error {
                    return hir_error(expression.span);
                }

                let Some(integer_type) = operand.type_.as_integer() else {
                    self.diagnostics.error(
                        "Unary negation requires an integer",
                        expression.span,
                        format!("found operand of type `{:?}`", operand.type_,),
                    );

                    return hir_error(expression.span);
                };

                if !integer_type.is_signed() {
                    self.diagnostics.error(
                        "Cannot negate an unsigned integer",
                        expression.span,
                        format!("`{}` is unsigned", integer_type.name(),),
                    );

                    return hir_error(expression.span);
                }

                HirExpression {
                    type_: operand.type_.clone(),
                    span: expression.span,

                    data: HirExpressionData::Unary {
                        operator: *operator,
                        operand: Box::new(operand),
                    },
                }
            }

            ExpressionData::Specialize { callee, arguments } => {
                if let ExpressionData::Intrinsic { name } = &callee.data {
                    return match self.sources.span_text(*name) {
                        "extern" => self.analyze_extern_intrinsic(arguments, expression.span),

                        other => {
                            self.diagnostics.error(
                                "Unknown intrinsic",
                                callee.span,
                                format!("Unknown intrinsic `{}`", other),
                            );

                            hir_error(expression.span)
                        }
                    };
                }

                let callee = self.analyze_expression(callee, None);

                let Type::FunctionTemplate(template_id) = callee.type_ else {
                    if callee.type_ != Type::Error {
                        self.diagnostics.error(
                            "Expression cannot be specialized",
                            callee.span,
                            format!("expected a function template found `{:?}`", callee.type_),
                        );
                    }

                    return hir_error(expression.span);
                };

                let Some(function_id) =
                    self.specialize_function(template_id, arguments, expression.span)
                else {
                    return hir_error(expression.span);
                };

                let function = self
                    .functions
                    .get(function_id)
                    .expect("completed serialization is missing");

                let type_ = Type::Function {
                    parameters: function
                        .hir
                        .parameters
                        .iter()
                        .map(|parameter| parameter.type_.clone())
                        .collect(),
                    return_type: Box::new(function.hir.return_type.clone()),
                };

                HirExpression {
                    type_,
                    span: expression.span,
                    data: HirExpressionData::KnownFunction(function_id),
                }
            }

            ExpressionData::Call { callee, arguments } => {
                let callee = self.analyze_expression(callee, None);
                if callee.type_ == Type::Error {
                    return hir_error(expression.span);
                }

                let (parameter_types, return_type) = match &callee.type_ {
                    Type::Function {
                        parameters,
                        return_type,
                    } => (parameters.clone(), return_type.as_ref().clone()),

                    actual_type => {
                        self.diagnostics.error(
                            "Expression is not callable",
                            callee.span,
                            format!("expected a function, found `{:?}`", actual_type),
                        );

                        return hir_error(expression.span);
                    }
                };

                if arguments.len() != parameter_types.len() {
                    self.diagnostics.error(
                        "Incorrect number of arguments",
                        expression.span,
                        format!(
                            "expected {} arguments, found {}",
                            parameter_types.len(),
                            arguments.len()
                        ),
                    );

                    return hir_error(expression.span);
                }

                let hir_arguments = arguments
                    .iter()
                    .zip(parameter_types.iter())
                    .map(|(argument, parameter_type)| {
                        self.analyze_expression(argument, Some(parameter_type))
                    })
                    .collect::<Vec<_>>();

                if hir_arguments
                    .iter()
                    .any(|argument| argument.type_ == Type::Error)
                {
                    return hir_error(expression.span);
                }

                if let Some(expected_type) = expected {
                    if *expected_type != Type::Error && *expected_type != return_type {
                        self.diagnostics.error(
                            "Call result has the wrong type",
                            expression.span,
                            format!("Expected `{:?}`, found `{:?}`", expected_type, return_type),
                        );

                        return hir_error(expression.span);
                    }
                }

                HirExpression {
                    span: expression.span,
                    type_: return_type,
                    data: HirExpressionData::Call {
                        callee: Box::new(callee),
                        arguments: hir_arguments,
                    },
                }
            }

            ExpressionData::Function(function) => {
                if !function.comptime_args.is_empty() {
                    let template_id = self.function_templates.insert(FunctionTemplate {
                        ast: function.clone(),
                        defining_scope: self.environment.current_scope(),
                    });

                    return HirExpression {
                        type_: Type::FunctionTemplate(template_id),
                        span: expression.span,
                        data: HirExpressionData::FunctionTemplate(template_id),
                    };
                }

                let mut return_type = self.resolve_type_expression(&function.return_type);

                if !self.validate_runtime_type(
                    &return_type,
                    function.return_type.span,
                    "functions cannot return unsized types at runtime",
                ) {
                    return_type = Type::Error;
                }

                let parameter_types = function
                    .runtime_args
                    .iter()
                    .map(|parameter| {
                        let mut type_ = self.resolve_type_expression(&parameter.type_annotation);

                        if !self.validate_runtime_type(
                            &type_,
                            parameter.type_annotation.span,
                            "runtime parameters cannot store unsized values",
                        ) {
                            type_ = Type::Error;
                        }

                        type_
                    })
                    .collect::<Vec<_>>();

                self.environment.push_scope();

                let mut hir_parameters = Vec::new();
                let mut names = HashMap::new();

                for (parameter, parameter_type) in
                    function.runtime_args.iter().zip(parameter_types.iter())
                {
                    let name = self.sources.span_text(parameter.name).to_owned();

                    if let Some(old) = names.insert(name.clone(), parameter.name) {
                        self.diagnostics.error_with_extra_labels(
                            "Duplicate parameter name",
                            parameter.name,
                            "duplicate",
                            vec![Label {
                                span: old,
                                text: "already defined here".to_owned(),
                            }],
                        );
                    }

                    let symbol_id = self.symbols.insert(Symbol {
                        name: name.clone(),
                        visibility: Visibility::Private,
                        declaration_span: Some(parameter.name),
                        kind: SymbolKind::Parameter,
                        type_: parameter_type.clone(),
                    });

                    self.environment.define(name.clone(), symbol_id);

                    hir_parameters.push(HirParameter {
                        symbol: symbol_id,
                        name: parameter.name,
                        type_: parameter_type.clone(),
                        span: parameter.span,
                    });
                }

                let body = self.analyze_block(&function.body, &return_type, true);

                self.environment.pop_scope();

                let function_type = Type::Function {
                    parameters: parameter_types,
                    return_type: Box::new(return_type.clone()),
                };

                let hir_function = HirFunctionExpression {
                    parameters: hir_parameters,
                    return_type,
                    body,
                };

                HirExpression {
                    type_: function_type,
                    span: expression.span,
                    data: HirExpressionData::Function(hir_function),
                }
            }
            ExpressionData::IntegerLiteral => {
                self.analyze_integer_literal(expression, expression.span, expected, false)
            }

            ExpressionData::Name => {
                let name = self.sources.span_text(expression.span);

                let Some(symbol) = self.environment.lookup(name) else {
                    self.diagnostics.error(
                        "Identifier not bound",
                        expression.span,
                        format!("Identifier `{name}` is not bound"),
                    );
                    return hir_error(expression.span);
                };

                self.analyze_symbol_reference(symbol, expression.span, expected)
            }
        }
    }

    pub(super) fn analyze_block(&mut self, block: &Block, return_type: &Type, needs_to_return: bool) -> HirBlock {
        self.environment.push_scope();

        let mut statements = Vec::new();

        fn if_has_guaranteed_return(if_: &If) -> bool {
            let If {
                condition: _,
                then_block,
                else_,
            } = if_;

            if has_guaranteed_return(then_block) {
                match else_ {
                    None => false, // No else branch means the return isn't guaranteed

                    Some(ElseBranch::Else(block)) => {
                        if has_guaranteed_return(block) {
                            true
                        } else {
                            false
                        }
                    }

                    Some(ElseBranch::ElseIf(if_statement)) => {
                        let StatementData::If(if_) = &if_statement.data else {
                            unreachable!()
                        };

                        if_has_guaranteed_return(if_)
                    }
                }
            } else {
                false
            }
        }

        fn has_guaranteed_return(block: &Block) -> bool {
            for statement in block.statements.iter() {
                match &statement.data {
                    StatementData::Return(_) => return true,

                    StatementData::If(if_) => {
                        if if_has_guaranteed_return(if_) {
                            return true;
                        }
                    }

                    StatementData::While(While {
                        condition: _,
                        while_block,
                    }) => {
                        // if has_guaranteed_return(while_block) {
                        //     return true;
                        // }
                        // the body could execute 0 times
                    }

                    _ => {

                    }
                }
            }

            false
        }
        let care_for_return = needs_to_return && *return_type != Type::Unit;
        if care_for_return && !has_guaranteed_return(block) {
            self.diagnostics.error(
                "Block does not return in every code path",
                block.span,
                ":(",
            )
        }

        for statement in &block.statements {
            statements.push(self.analyze_statement(statement, return_type));
        }

        self.environment.pop_scope();

        HirBlock {
            statements,
            span: block.span,
        }
    }

    pub(super) fn analyze_statement(
        &mut self,
        statement: &Statement,
        return_type: &Type,
    ) -> HirStatement {
        match &statement.data {
            StatementData::Expression(expression) => {
                let expression = self.analyze_expression(expression, Some(&Type::Unit));

                HirStatement {
                    span: statement.span,
                    data: HirStatementData::Expression(expression),
                }
            }

            StatementData::Assignment { target, expression } => {
                let Some(target) = self.analyze_place(target) else {
                    return HirStatement::error(statement.span);
                };

                let expression = self.analyze_expression(expression, Some(&target.type_));

                HirStatement {
                    span: statement.span,
                    data: HirStatementData::Assignment { target, expression },
                }
            }

            StatementData::While(While {
                condition,
                while_block,
            }) => {
                let condition = self.analyze_expression(condition, Some(&Type::Bool));

                let while_block = self.analyze_block(while_block, return_type, false);

                HirStatement {
                    span: statement.span,
                    data: HirStatementData::While {
                        condition,
                        while_block,
                    },
                }
            }

            StatementData::If(If {
                condition,
                then_block,
                else_,
            }) => {
                let condition = self.analyze_expression(condition, Some(&Type::Bool));

                let then_block = self.analyze_block(then_block, return_type, false);

                let else_branch = match else_ {
                    None => None,
                    Some(ElseBranch::Else(block)) => {
                        Some(HirElseBranch::Else(self.analyze_block(block, return_type, false)))
                    }
                    Some(ElseBranch::ElseIf(statement)) => {
                        let hir_statement = self.analyze_statement(statement.as_ref(), return_type);

                        Some(HirElseBranch::ElseIf(Box::new(hir_statement)))
                    }
                };

                HirStatement {
                    span: statement.span,
                    data: HirStatementData::If {
                        condition,
                        then_block,
                        else_branch,
                    },
                }
            }

            StatementData::Binding(binding) => {
                let name = self.sources.span_text(binding.name).to_owned();

                let annotated_type = binding
                    .type_annotation
                    .as_ref()
                    .map(|annotation| self.resolve_type_expression(annotation));

                let mut expression =
                    self.analyze_expression(&binding.expression, annotated_type.as_ref());

                if !self.validate_runtime_type(
                    &expression.type_,
                    binding.expression.span,
                    "runtime bindings cannot contain unsized types",
                ) {
                    expression = hir_error(binding.expression.span);
                }

                if let Some(previous_id) = self.environment.lookup_current(&name) {
                    let previous = self.symbols.get(previous_id);

                    self.diagnostics.error_with_extra_labels(
                        "Duplicate local binding",
                        binding.name,
                        "duplicate binding",
                        previous
                            .declaration_span
                            .map(|span| {
                                vec![Label {
                                    span,
                                    text: "previously defined here".to_owned(),
                                }]
                            })
                            .unwrap_or_default(),
                    )
                }

                let symbol_id = self.symbols.insert(Symbol {
                    name: name.clone(),
                    declaration_span: Some(binding.name),
                    visibility: Visibility::Private,
                    kind: SymbolKind::Local {
                        mutable: binding.mutable,
                    },
                    type_: expression.type_.clone(),
                });

                self.environment.define(name, symbol_id);

                HirStatement {
                    span: statement.span,
                    data: HirStatementData::Binding {
                        symbol: symbol_id,
                        expression,
                    },
                }
            }

            StatementData::Return(expression) => {
                let Some(expression) = expression else {
                    return if *return_type == Type::Unit {
                        HirStatement {
                            span: statement.span,
                            data: HirStatementData::Return(None),
                        }
                    } else {
                        self.diagnostics.error(
                            "Return without value in function with non-unit return type".to_owned(),
                            statement.span,
                            format!("Expected a `{:?}`", return_type),
                        );

                        HirStatement {
                            span: statement.span,
                            data: HirStatementData::Return(None),
                        }
                    };
                };

                let hir_expression = self.analyze_expression(&expression, Some(return_type));

                HirStatement {
                    span: statement.span,
                    data: HirStatementData::Return(Some(hir_expression)),
                }
            }
        }
    }

    pub(super) fn resolve_type_expression(&mut self, expression: &TypeExpression) -> Type {
        match &expression.data {
            TypeExpressionData::FixedArray { base_type, size } => {
                let size = match self.resolve_static_array_length(size) {
                    Some(size) => size,
                    None => return Type::Error,
                };

                Type::FixedArray {
                    base_type: Box::new(self.resolve_type_expression(base_type.as_ref())),
                    size,
                }
            }

            TypeExpressionData::Unit => Type::Unit,

            TypeExpressionData::Function {
                parameters,
                return_type,
            } => Type::Function {
                parameters: parameters
                    .iter()
                    .map(|type_| self.resolve_type_expression(type_))
                    .collect(),
                return_type: Box::new(self.resolve_type_expression(return_type)),
            },

            TypeExpressionData::Identifier => {
                let name = self.sources.span_text(expression.span);

                let Some(symbol_id) = self.environment.lookup(name) else {
                    self.diagnostics.error(
                        "Unknown Type",
                        expression.span,
                        ":(",
                    );
                    return Type::Error;
                };

                let (kind, symbol_type) = {
                    let symbol = self.symbols.get(symbol_id);
                    (symbol.kind.clone(), symbol.type_.clone())
                };

                match kind {
                    SymbolKind::BuiltinType(type_) => type_.clone(),
                    SymbolKind::ComptimeParameter => match self.lookup_value(symbol_id) {
                        Some(ComptimeValue::Type(type_)) => type_.clone(),
                        Some(_) => {
                            self.diagnostics.type_mismatch(expression.span, "type", symbol_type);
                            Type::Error
                        }
                        None => Type::Error,
                    },

                    SymbolKind::Binding {
                        phase: Phase::Comptime,
                        ..
                    } => match self.ensure_binding_evaluated(symbol_id) {
                        Ok(ComptimeValue::Type(type_)) => type_.clone(),
                        Ok(_) => {
                            self.diagnostics.type_mismatch(expression.span, "type", symbol_type);
                            Type::Error
                        }
                        Err(_) => Type::Error,
                    },

                    SymbolKind::Binding {
                        phase: Phase::Runtime,
                        ..
                    } => {
                        self.diagnostics.error_with_extra_labels(
                            "Phase mismatch",
                            expression.span,
                            "Binding not known at comptime",
                            match self.symbols.get(symbol_id).declaration_span {
                                Some(span) => vec![Label {
                                    text: "Binding defined here".to_owned(),
                                    span,
                                }],

                                None => vec![],
                            },
                        );

                        Type::Error
                    }
                    _ => {
                        self.diagnostics.error(
                            "Expected a Type found a Value",
                            expression.span,
                            ":(",
                        );
                        Type::Error
                    }
                }
            }
        }
    }

    pub(super) fn analyze_binding(
        &mut self,
        binding: &Binding,
        span: Span,
        symbol_id: SymbolId,
    ) -> Option<HirBinding> {
        let type_annotation = binding
            .type_annotation
            .as_ref()
            .map(|type_expression| self.resolve_type_expression(type_expression));

        let hir_expression = self.analyze_expression(&binding.expression, type_annotation.as_ref());

        self.symbols.get_mut(symbol_id).type_ = hir_expression.type_.clone();

        let hir_binding = HirBinding {
            symbol: symbol_id,
            span,
            phase: binding.phase,
            mutable: binding.mutable,
            expression: hir_expression,
        };

        Some(hir_binding)
    }

    pub(super) fn analyze_place(&mut self, target: &Expression) -> Option<HirPlace> {
        match &target.data {
            ExpressionData::Name => {
                let name = self.sources.span_text(target.span);

                let Some(symbol_id) = self.environment.lookup(name) else {
                    self.diagnostics.error(
                        "Unknown assignment target",
                        target.span,
                        format!("`{name}` is not defined"),
                    );

                    return None;
                };

                let (type_, mutable) = {
                    let symbol = self.symbols.get(symbol_id);

                    (
                        symbol.type_.clone(),
                        matches!(symbol.kind, SymbolKind::Local { mutable: true }),
                    )
                };

                if !mutable {
                    self.diagnostics.error(
                        "Cannot assign to immutable binding",
                        target.span,
                        format!("`{name}` is not mutable"),
                    );
                }

                Some(HirPlace {
                    span: target.span,
                    type_,
                    data: HirPlaceData::Symbol(symbol_id),
                })
            }

            ExpressionData::Index { base, index } => {
                let base_place = self.analyze_place(base)?;

                let HirPlaceData::Symbol(array) = base_place.data else {
                    self.diagnostics.error(
                        "Unsupported assignment target",
                        base.span,
                        "nested indexed places are not supported yet",
                    );

                    return None;
                };

                let Type::FixedArray {
                    size: array_size,
                    base_type,
                } = base_place.type_
                else {
                    self.diagnostics.error(
                        "Value is not indexable",
                        base.span,
                        "assignment target must be an array",
                    );

                    return None;
                };

                let index =
                    self.analyze_expression(index, Some(&Type::Integer(IntegerType::Usize)));

                if index.type_ == Type::Error {
                    return None;
                }

                if let HirExpressionData::Integer(index_value) = &index.data {
                    let valid = usize::try_from(*index_value).is_ok_and(|index| index < array_size);

                    if !valid {
                        self.diagnostics.error(
                            "Array index out of bounds",
                            index.span,
                            format!(
                                "array length is {array_size}, but the index is `{index_value}`"
                            ),
                        );

                        return None;
                    }
                }

                Some(HirPlace {
                    span: target.span,
                    type_: *base_type,
                    data: HirPlaceData::Index {
                        array,
                        index,
                        array_size,
                    },
                })
            }

            _ => {
                self.diagnostics.error(
                    "Invalid assignment target",
                    target.span,
                    "this expression does not identify writable storage",
                );

                None
            }
        }
    }

    fn resolve_static_array_length(&mut self, expression: &Expression) -> Option<usize> {
        let hir = self.analyze_expression(expression, Some(&Type::Integer(IntegerType::Usize)));

        if hir.type_ == Type::Error {
            return None;
        }

        let value = self.evaluate_expression(&hir);

        let ComptimeValue::Integer {
            value,
            type_: IntegerType::Usize,
        } = value
        else {
            if value != ComptimeValue::Error {
                self.diagnostics.error(
                    "Array length is not an integer",
                    expression.span,
                    "expected a compile-time `usize` value",
                );
            }

            return None;
        };

        match usize::try_from(value) {
            Ok(length) => Some(length),

            Err(_) => {
                self.diagnostics.error(
                    "Invalid array length",
                    expression.span,
                    format!("`{value}` is not a valid array length"),
                );

                None
            }
        }
    }
    // TODO: Think if custom suffixes should be allowed, like maybe you can say "10mm" calls a function `mm` defined
    // somewhere that gets you back a "Length" and same for like 10m but there's an ambiguity here if you also wanted 10m to mean minutes
    // interesting questions!
    fn parse_integer_literal(&mut self, span: Span) -> Option<ParsedIntegerLiteral> {
        let text = self.sources.span_text(span);

        let suffix_start = text
            .find(|char: char| !char.is_ascii_digit())
            .unwrap_or(text.len());

        let digits = &text[..suffix_start];
        let suffix = &text[suffix_start..];

        let magnitude = match digits.parse::<u64>() {
            Ok(value) => value,

            Err(_) => {
                self.diagnostics.error(
                    "Integer literal is too large",
                    span,
                    "this literal cannot be represented by the compiler",
                );
                return None;
            }
        };

        let suffix = match suffix {
            "" => None,
            "u8" => Some(IntegerType::U8),
            "i8" => Some(IntegerType::I8),
            "u16" => Some(IntegerType::U16),
            "i16" => Some(IntegerType::I16),
            "u32" => Some(IntegerType::U32),
            "i32" => Some(IntegerType::I32),
            "u64" => Some(IntegerType::U64),
            "i64" => Some(IntegerType::I64),
            "usize" => Some(IntegerType::Usize),
            "isize" => Some(IntegerType::Isize),

            unknown => {
                self.diagnostics.error(
                    "Unknown integer suffix",
                    span,
                    format!("`{unknown}` is not a supported integer suffix"),
                );
                return None;
            }
        };

        Some(ParsedIntegerLiteral { magnitude, suffix })
    }

    fn analyze_integer_literal(
        &mut self,
        expression: &Expression,
        result_span: Span,
        expected: Option<&Type>,
        negate: bool,
    ) -> HirExpression {
        let Some(literal) = self.parse_integer_literal(expression.span) else {
            return hir_error(result_span);
        };

        let expected_integer = expected.and_then(Type::as_integer);

        let integer_type = match literal.suffix {
            Some(suffix) => {
                if let Some(expected_integer) = expected_integer
                    && expected_integer != suffix
                {
                    self.diagnostics.error(
                        "Integer type mismatch",
                        result_span,
                        format!(
                            "expected `{}`, but this \
                               literal has type `{}`",
                            expected_integer.name(),
                            suffix.name(),
                        ),
                    );

                    return hir_error(result_span);
                }

                suffix
            }

            None => {
                if let Some(expected) = expected {
                    match expected.as_integer() {
                        Some(integer) => integer,

                        None => {
                            self.diagnostics.type_mismatch(result_span, expected, "Integer Literal");
                            return hir_error(result_span);
                        }
                    }
                } else {
                    IntegerType::I64
                }
            }
        };

        if let Some(expected) = expected
            && !expected.is_integer()
        {
            self.diagnostics.type_mismatch(result_span, expected, integer_type.name());
            return hir_error(result_span);
        }

        if negate && !integer_type.is_signed() {
            self.diagnostics.error(
                "Cannot negate an unsigned integer",
                result_span,
                format!("`{}` is unsigned", integer_type.name(),),
            );

            return hir_error(result_span);
        }

        let magnitude = literal.magnitude as i128;

        let value = if negate { -magnitude } else { magnitude };

        if !integer_type.contains(value, &self.target) {
            self.diagnostics.error(
                "Integer literal out of range",
                result_span,
                format!("`{value}` does not fit in `{}`", integer_type.name(),),
            );

            return hir_error(result_span);
        }

        HirExpression {
            type_: Type::Integer(integer_type),
            span: result_span,
            data: HirExpressionData::Integer(value),
        }
    }

    fn specialize_function(
        &mut self,
        template_id: FunctionTemplateId,
        arguments: &[Expression],
        span: Span,
    ) -> Option<FunctionId> {
        let template = self.function_templates.get(template_id)?.clone();

        let function = template.ast;

        if arguments.len() != function.comptime_args.len() {
            self.diagnostics.error(
                "Incorrect number of comptime arguments",
                span,
                format!(
                    "expected {}, found {}",
                    function.comptime_args.len(),
                    arguments.len()
                ),
            );

            return None;
        }

        let caller_scope = self.environment.switch_scope(template.defining_scope);

        let comptime_types = function
            .comptime_args
            .iter()
            .map(|parameter| self.resolve_type_expression(&parameter.type_annotation))
            .collect::<Vec<_>>();

        self.environment.switch_scope(caller_scope);

        if comptime_types.iter().any(|type_| *type_ == Type::Error) {
            return None;
        }

        let mut values = Vec::new();
        let mut key_arguments = Vec::new();

        for ((argument, parameter_type), parameter) in arguments
            .iter()
            .zip(&comptime_types)
            .zip(&function.comptime_args)
        {
            let hir_argument = self.analyze_expression(argument, Some(parameter_type));

            if hir_argument.type_ == Type::Error {
                return None;
            }

            let value = self.evaluate_expression(&hir_argument);

            if value == ComptimeValue::Error {
                return None;
            }

            key_arguments.push(self.comptime_key(&value, parameter.span)?);

            values.push(value);
        }

        let key = SpecializationKey {
            template: template_id,
            arguments: key_arguments,
        };

        if let Some(function_id) = self.specializations.get(&key) {
            return Some(*function_id);
        }

        if !self.active_specializations.insert(key.clone()) {
            self.diagnostics.error(
                "Recursive specialization",
                span,
                "this specialization depends on itself",
            ); // TODO: Show path of dependency

            return None;
        }

        let caller_scope = self.environment.switch_scope(template.defining_scope);

        let function_id = self.elaborate_specialization(&function, values);

        self.environment.switch_scope(caller_scope);

        self.active_specializations.remove(&key);

        let function_id = function_id?;

        self.specializations.insert(key, function_id);

        Some(function_id)
    }

    fn elaborate_specialization(
        &mut self,
        function: &FunctionExpression,
        values: Vec<ComptimeValue>,
    ) -> Option<FunctionId> {
        self.environment.push_scope();

        let mut captures = HashMap::new();
        let mut names = HashMap::new();

        for (parameter, value) in function.comptime_args.iter().zip(values) {
            let type_ = self.resolve_type_expression(&parameter.type_annotation);

            if type_ == Type::Error {
                self.environment.pop_scope();
                return None;
            }

            let name = self.sources.span_text(parameter.name).to_owned();

            if let Some(previous) = names.insert(name.clone(), parameter.name) {
                self.diagnostics.error_with_extra_labels(
                    "Duplicate parameter name",
                    parameter.name,
                    "duplicate",
                    vec![Label {
                        span: previous,
                        text: "already defined here".to_owned(),
                    }],
                );
            }

            let symbol = self.symbols.insert(Symbol {
                name: name.clone(),
                declaration_span: Some(parameter.name),
                visibility: Visibility::Private,
                kind: SymbolKind::ComptimeParameter,
                type_,
            });

            self.environment.define(name, symbol);
            captures.insert(symbol, value);
        }

        self.frames.push(captures.clone());

        let mut return_type = self.resolve_type_expression(&function.return_type);

        if !self.validate_runtime_type(
            &return_type,
            function.return_type.span,
            "functions cannot return unsized types at runtime",
        ) {
            return_type = Type::Error;
        }

        let mut hir_parameters = Vec::new();

        for parameter in &function.runtime_args {
            let mut type_ = self.resolve_type_expression(&parameter.type_annotation);

            if !self.validate_runtime_type(
                &type_,
                parameter.type_annotation.span,
                "runtime parameters cannot store unsized values",
            ) {
                type_ = Type::Error;
            }

            let name = self.sources.span_text(parameter.name).to_owned();

            if let Some(previous) = names.insert(name.clone(), parameter.name) {
                self.diagnostics.error_with_extra_labels(
                    "Duplicate parameter name",
                    parameter.name,
                    "duplicate",
                    vec![Label {
                        span: previous,
                        text: "already defined here".to_owned(),
                    }],
                );
            }

            let symbol = self.symbols.insert(Symbol {
                name: name.clone(),
                declaration_span: Some(parameter.name),
                visibility: Visibility::Private,
                kind: SymbolKind::Parameter,
                type_: type_.clone(),
            });

            self.environment.define(name, symbol);

            hir_parameters.push(HirParameter {
                symbol,
                name: parameter.name,
                type_,
                span: parameter.span,
            });
        }

        let body = self.analyze_block(&function.body, &return_type, true);

        let mut referenced_symbols = HashSet::new();
        collect_block_symbols(&body, &mut referenced_symbols);

        captures.retain(|symbol, _| referenced_symbols.contains(symbol));

        self.frames.pop();
        self.environment.pop_scope();

        if return_type == Type::Error
            || hir_parameters
                .iter()
                .any(|parameter| parameter.type_ == Type::Error)
        {
            return None;
        }

        Some(self.functions.insert(ComptimeFunction {
            hir: HirFunctionExpression {
                parameters: hir_parameters,
                return_type,
                body,
            },
            captures,
        }))
    }

    fn comptime_key(&mut self, value: &ComptimeValue, span: Span) -> Option<ComptimeKey> {
        match value {
            ComptimeValue::Type(type_) => Some(ComptimeKey::Type(type_.clone())),

            ComptimeValue::String(string) => Some(ComptimeKey::Str(string.clone())),

            ComptimeValue::Integer { value, type_ } => Some(ComptimeKey::Integer {
                value: *value,
                type_: *type_,
            }),

            ComptimeValue::Bool(value) => Some(ComptimeKey::Bool(*value)),

            unsupported => {
                self.diagnostics.error(
                    "Unsupported compile-time argument",
                    span,
                    format!("`{unsupported:?}` cannot currently be used as a specialization key"),
                );

                None
            }
        }
    }

    pub(super) fn validate_runtime_type(
        &mut self,
        type_: &Type,
        span: Span,
        description: &str,
    ) -> bool {
        let mut diagnostic_unsized = |type_name: &str| {
            self.diagnostics.error(
                format!("`{}` has no runtime representation", type_name),
                span,
                description,
            );
        };

        match type_ {
            Type::Type => {
                diagnostic_unsized("type");
                false
            }
            Type::Str => {
                diagnostic_unsized("str");
                false
            }
            Type::Module(id) => {
                diagnostic_unsized(&format!("module {:?}", id));
                false
            }

            _ => true,
        }
    }

    fn analyze_extern_intrinsic(&mut self, arguments: &[Expression], span: Span) -> HirExpression {
        if arguments.len() != 3 {
            self.diagnostics.error(
                "Incorrect number of arguments to `@extern`",
                span,
                format!("expected 3, but found {}", arguments.len()),
            );

            return hir_error(span);
        }

        let expected_types = [Type::Str, Type::Str, Type::Type];

        let mut values = Vec::with_capacity(3);

        for (argument, expected_type) in arguments.iter().zip(&expected_types) {
            let hir_argument = self.analyze_expression(argument, Some(expected_type));

            if hir_argument.type_ == Type::Error {
                return hir_error(span);
            }

            let value = self.evaluate_expression(&hir_argument);

            if value == ComptimeValue::Error {
                return hir_error(span);
            }

            values.push(value);
        }

        let [
            ComptimeValue::String(abi),
            ComptimeValue::String(link_name),
            ComptimeValue::Type(function_type),
        ] = values.as_slice()
        else {
            // The expected types above should make this impossible unless
            // analysis and compile-time evaluation disagree.
            unreachable!("validated @extern arguments produced unexpected values");
        };

        let abi = match abi.as_str() {
            "C" => ExternAbi::C,

            unsupported => {
                self.diagnostics.error(
                    "Unsupported external ABI",
                    arguments[0].span,
                    format!("ABI `{unsupported}` is not supported; expected `C`"),
                );

                return hir_error(span);
            }
        };

        if !matches!(function_type, Type::Function { .. }) {
            self.diagnostics.error(
                "Invalid external function type",
                arguments[2].span,
                format!("expected a function type, found `{function_type:?}`"),
            );

            return hir_error(span);
        }

        let Some(external_symbol) =
            self.declare_external_function(abi, link_name, function_type, span)
        else {
            return hir_error(span);
        };

        HirExpression {
            type_: function_type.clone(),
            span,
            data: HirExpressionData::Symbol(external_symbol),
        }
    }

    fn declare_external_function(
        &mut self,
        abi: ExternAbi,
        link_name: &str,
        function_type: &Type,
        declaration_span: Span,
    ) -> Option<SymbolId> {
        let existing = self
            .symbols
            .symbols
            .iter()
            .enumerate()
            .find_map(|(index, symbol)| {
                let SymbolKind::ExternFunction {
                    abi: existing_abi,
                    link_name: existing_link_name,
                } = &symbol.kind
                else {
                    return None;
                };

                if *existing_abi == abi && existing_link_name == link_name {
                    Some((
                        SymbolId(index as u32),
                        symbol.type_.clone(),
                        symbol.declaration_span,
                    ))
                } else {
                    None
                }
            });

        if let Some((existing_symbol, existing_type, existing_span)) = existing {
            if existing_type == *function_type {
                return Some(existing_symbol);
            }

            let message = format!(
                "external symbol `{link_name}` was previously declared \
                as `{existing_type:?}`, but this declaration uses \
                `{function_type:?}`"
            );

            if let Some(existing_span) = existing_span {
                self.diagnostics.error_with_extra_labels(
                    "Conflicting external function declarations",
                    declaration_span,
                    message,
                    vec![Label::new(
                        existing_span,
                        "previous declaration is here".to_owned(),
                    )],
                );
            } else {
                self.diagnostics.error(
                    "Conflicting external function declarations",
                    declaration_span,
                    message,
                );
            }

            return None;
        }

        let symbol = self.symbols.insert(Symbol {
            name: link_name.to_owned(),
            declaration_span: Some(declaration_span),
            visibility: Visibility::Private,

            kind: SymbolKind::ExternFunction {
                abi,
                link_name: link_name.to_owned(),
            },

            type_: function_type.clone(),
        });

        Some(symbol)
    }

    fn analyze_symbol_reference(
        &mut self,
        symbol: SymbolId,
        span: Span,
        expected: Option<&Type>,
    ) -> HirExpression {
        let mut actual_type = self.symbols.get(symbol).type_.clone();

        if actual_type == Type::Unknown && self.pending_bindings.contains_key(&symbol) {
            if self.ensure_binding_elaborated(symbol).is_err() {
                return hir_error(span);
            }

            actual_type = self.symbols.get(symbol).type_.clone();
        }

        if actual_type == Type::Unknown {
            let name = &self.symbols.get(symbol).name;

            self.diagnostics.error(
                "Identifier not yet bound",
                span,
                format!("Cannot determine the type of `{name}`"),
            );

            return hir_error(span);
        }

        if actual_type == Type::Error {
            return hir_error(span);
        }

        if let Some(expected_type) = expected {
            if *expected_type != Type::Error && *expected_type != actual_type {
                self.diagnostics.type_mismatch(span, expected_type, actual_type);
                return hir_error(span);
            }
        }

        HirExpression {
            span,
            type_: actual_type,
            data: HirExpressionData::Symbol(symbol),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ParsedIntegerLiteral {
    magnitude: u64,
    suffix: Option<IntegerType>,
}

fn collect_block_symbols(block: &HirBlock, symbols: &mut HashSet<SymbolId>) {
    for statement in &block.statements {
        collect_statement_symbols(statement, symbols);
    }
}

fn collect_statement_symbols(statement: &HirStatement, symbols: &mut HashSet<SymbolId>) {
    match &statement.data {
        HirStatementData::Return(Some(expression)) | HirStatementData::Expression(expression) => {
            collect_expression_symbols(expression, symbols);
        }

        HirStatementData::Return(None) | HirStatementData::Error => {}

        HirStatementData::Binding { expression, .. } => {
            collect_expression_symbols(expression, symbols);
        }

        HirStatementData::Assignment { target, expression } => {
            collect_place_symbols(target, symbols);
            collect_expression_symbols(expression, symbols);
        }

        HirStatementData::If {
            condition,
            then_block,
            else_branch,
        } => {
            collect_expression_symbols(condition, symbols);
            collect_block_symbols(then_block, symbols);

            match else_branch {
                Some(HirElseBranch::ElseIf(statement)) => {
                    collect_statement_symbols(statement, symbols);
                }

                Some(HirElseBranch::Else(block)) => {
                    collect_block_symbols(block, symbols);
                }

                None => {}
            }
        }

        HirStatementData::While {
            condition,
            while_block,
        } => {
            collect_expression_symbols(condition, symbols);
            collect_block_symbols(while_block, symbols);
        }
    }
}

fn collect_place_symbols(place: &HirPlace, symbols: &mut HashSet<SymbolId>) {
    match &place.data {
        HirPlaceData::Symbol(symbol) => {
            symbols.insert(*symbol);
        }

        HirPlaceData::Index { array, index, .. } => {
            symbols.insert(*array);
            collect_expression_symbols(index, symbols);
        }
    }
}

fn collect_expression_symbols(expression: &HirExpression, symbols: &mut HashSet<SymbolId>) {
    use HirExpressionData::*;
    match &expression.data {
        Symbol(symbol) => {
            symbols.insert(*symbol);
        }

        Unary { operand, .. } => {
            collect_expression_symbols(operand, symbols);
        }

        Binary { lhs, rhs, .. } => {
            collect_expression_symbols(lhs, symbols);
            collect_expression_symbols(rhs, symbols);
        }

        Call { callee, arguments } => {
            collect_expression_symbols(callee, symbols);
            for argument in arguments {
                collect_expression_symbols(argument, symbols);
            }
        }

        ArrayRepeatInitialization { value, .. } => {
            collect_expression_symbols(value, symbols);
        }

        Index { base, index } => {
            collect_expression_symbols(base, symbols);
            collect_expression_symbols(index, symbols);
        }

        Function(function) => {
            collect_block_symbols(&function.body, symbols);
        }

        TypeValue(_) | FunctionTemplate(_) | KnownFunction(_) | Integer(_) | Bool(_)
        | StringLiteral(_) | Module(_) | Error => {}
    }
}

fn hir_error(span: Span) -> HirExpression {
    HirExpression::error(span)
}