use crate::diagnostics::{Diagnostic, Label};
use crate::parser::ast::{
    BinaryOperator, Binding, Block, ElseBranch, Expression, ExpressionData, If, Item, ItemData,
    Program, Statement, StatementData, TypeExpression, TypeExpressionData, While,
};
use crate::semantic::hir::{
    HirBinding, HirBlock, HirElseBranch, HirExpression, HirExpressionData, HirFunctionExpression,
    HirParameter, HirPlace, HirPlaceData, HirProgram, HirStatement, HirStatementData,
};
use crate::semantic::symbols::{Environment, Symbol, SymbolId, SymbolKind, SymbolTable};
use crate::semantic::types::Type;
use crate::source::{SourceFile, Span};
use std::collections::HashMap;

pub mod hir;
pub mod symbols;
pub mod types;

#[derive(Debug)]
pub struct SemanticProgram {
    pub hir: HirProgram,
    pub symbols: SymbolTable,
}

pub struct Analyzer<'src> {
    source: &'src SourceFile,
    symbols: SymbolTable,
    environment: Environment,
    diagnostics: Vec<Diagnostic>,
}

impl<'src> Analyzer<'src> {
    pub fn new(source: &'src SourceFile) -> Self {
        let mut symbols = SymbolTable::new();
        let mut environment = Environment::new();

        let i64_id = symbols.insert(Symbol {
            name: "i64".to_owned(),
            kind: SymbolKind::BuiltinType(Type::I64),
            declaration_span: None,
            type_: Type::Type,
        });

        environment.define("i64".to_owned(), i64_id);

        let unit_id = symbols.insert(Symbol {
            name: "unit".to_owned(),
            kind: SymbolKind::BuiltinType(Type::Unit),
            declaration_span: None,
            type_: Type::Type,
        });

        environment.define("unit".to_owned(), unit_id);

        let bool_id = symbols.insert(Symbol {
            name: "bool".to_owned(),
            kind: SymbolKind::BuiltinType(Type::Bool),
            declaration_span: None,
            type_: Type::Type,
        });

        environment.define("bool".to_owned(), bool_id);

        Self::register_external_function(
            &mut symbols,
            &mut environment,
            "print_i64",
            "flang_print_i64",
            vec![Type::I64],
            Type::Unit,
        );

        Self::register_external_function(
            &mut symbols,
            &mut environment,
            "print_bool",
            "flang_print_bool",
            vec![Type::Bool],
            Type::Unit,
        );

        Self::register_external_function(
            &mut symbols,
            &mut environment,
            "read_byte",
            "flang_read_byte",
            vec![],
            Type::I64,
        );

        Self {
            source,
            symbols,
            environment,
            diagnostics: Vec::new(),
        }
    }

    fn register_external_function(
        symbols: &mut SymbolTable,
        environment: &mut Environment,
        name: impl Into<String>,
        link_name: impl Into<String>,
        parameters: Vec<Type>,
        return_type: Type,
    ) {
        let name = name.into();
        let link_name = link_name.into();

        let symbol = Symbol {
            name: name.clone(),
            kind: SymbolKind::ExternFunction { link_name },
            declaration_span: None,
            type_: Type::Function {
                parameters,
                return_type: Box::new(return_type),
            },
        };

        let symbol_id = symbols.insert(symbol);
        environment.define(name, symbol_id);
    }

    pub fn analyze(mut self, program: Program) -> Result<SemanticProgram, Vec<Diagnostic>> {
        let mut collected_bindings = Vec::new();
        for item in program.items.iter() {
            let Item {
                span: item_span,
                data,
            } = item;
            match data {
                ItemData::Binding(binding) => {
                    let name = self.source.span_text(binding.name);

                    if let Some(_) = self.environment.lookup(name) {
                        self.diagnostics.push(Diagnostic::error(
                            "Duplicate binding found",
                            *item_span,
                            "Evil :(",
                        ));

                        continue;
                    }

                    let symbol = Symbol {
                        name: name.to_owned(),
                        declaration_span: Some(binding.name),
                        kind: SymbolKind::Binding {
                            phase: binding.phase,
                            mutable: binding.mutable,
                        },
                        type_: Type::Unknown,
                    };

                    let symbol_id = self.symbols.insert(symbol);
                    self.environment.define(name.to_owned(), symbol_id);
                    collected_bindings.push((binding, *item_span, symbol_id));
                }
            }
        }

        let mut hir_bindings = Vec::new();

        for (binding, item_span, symbol_id) in collected_bindings {
            if let Some(hir_binding) = self.analyze_binding(binding, item_span, symbol_id) {
                hir_bindings.push(hir_binding);
            }
        }

        if self.diagnostics.is_empty() {
            Ok(SemanticProgram {
                hir: HirProgram {
                    bindings: hir_bindings,
                },
                symbols: self.symbols,
            })
        } else {
            Err(self.diagnostics.clone())
        }
    }

    fn analyze_place(&mut self, target: &Expression) -> Option<HirPlace> {
        match &target.data {
            ExpressionData::Name => {
                let name = self.source.span_text(target.span);

                let Some(symbol_id) = self.environment.lookup(name) else {
                    self.diagnostics.push(Diagnostic::error(
                        "Unknown assignment target",
                        target.span,
                        format!("`{name}` is not defined"),
                    ));

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
                    self.diagnostics.push(Diagnostic::error(
                        "Cannot assign to immutable binding",
                        target.span,
                        format!("`{name}` is not mutable")
                    ));
                }

                Some(HirPlace {
                    span: target.span,
                    type_,
                    data: HirPlaceData::Symbol(symbol_id)
                })
            }

            ExpressionData::Index { base, index } => {
                let base_place = self.analyze_place(base)?;

                let HirPlaceData::Symbol(array) = base_place.data else {
                    self.diagnostics.push(Diagnostic::error(
                        "Unsupported assignment target",
                        base.span,
                        "nested indexed places are not supported yet"
                    ));

                    return None;
                };

                let Type::FixedArray {
                    size: array_size,
                    base_type,
                } = base_place.type_ else {
                    self.diagnostics.push(Diagnostic::error(
                        "Value is not indexable",
                        base.span,
                        "assignment target must be an array"
                    ));

                    return None;
                };

                let index = self.analyze_expression(
                    index, Some(base_type.as_ref())
                );

                if index.type_ == Type::Error {
                    return None;
                }

                if let HirExpressionData::Integer(index_value) = &index.data {
                    let valid = usize::try_from(*index_value)
                        .is_ok_and(|index| index < array_size);

                    if !valid {
                        self.diagnostics.push(Diagnostic::error(
                            "Array index out of bounds",
                            index.span,
                            format!(
                                "array length is {array_size}, but the index is `{index_value}`"
                            )
                        ));

                        return None;
                    }
                }

                Some(HirPlace {
                    span: target.span,
                    type_: *base_type,
                    data: HirPlaceData::Index {
                        array,
                        index,
                        array_size
                    }
                })
            }

            _ => {
                self.diagnostics.push(Diagnostic::error(
                    "Invalid assignment target",
                    target.span,
                    "this expression does not identify writable storage"
                ));

                None
            },
        }
    }

    fn analyze_binding(
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

    fn resolve_type_expression(&mut self, expression: &TypeExpression) -> Type {
        match &expression.data {
            TypeExpressionData::FixedArray { base_type, size } => {
                let size = match &size.data {
                    ExpressionData::IntegerLiteral => {
                        self.source.span_text(size.span).parse().unwrap()
                    }
                    _ => {
                        todo!("Unsupported expression for array type length")
                    }
                };

                Type::FixedArray {
                    base_type: Box::new(self.resolve_type_expression(base_type.as_ref())),
                    size,
                }
            }
            TypeExpressionData::Unit => Type::Unit,
            TypeExpressionData::Identifier => {
                let name = self.source.span_text(expression.span);

                let Some(symbol_id) = self.environment.lookup(name) else {
                    self.diagnostics.push(Diagnostic::error(
                        "Unknown Type".to_owned(),
                        expression.span,
                        ":(".to_owned(),
                    ));
                    return Type::Error;
                };

                match &self.symbols.get(symbol_id).kind {
                    SymbolKind::BuiltinType(type_) => type_.clone(),
                    _ => {
                        self.diagnostics.push(Diagnostic::error(
                            "Expected a Type found a Value".to_owned(),
                            expression.span,
                            ":(".to_owned(),
                        ));
                        Type::Error
                    }
                }
            }
        }
    }

    fn analyze_expression(
        &mut self,
        expression: &Expression,
        expected: Option<&Type>,
    ) -> HirExpression {
        match &expression.data {
            ExpressionData::Index { base, index } => {
                let base = self.analyze_expression(base, None);
                let index = self.analyze_expression(index, Some(&Type::I64));

                if base.type_ == Type::Error || index.type_ == Type::Error {
                    return HirExpression::error(expression.span);
                }

                let (element_type, array_size) = match &base.type_ {
                    Type::FixedArray { base_type, size } => (base_type.as_ref().clone(), *size),

                    other => {
                        self.diagnostics.push(Diagnostic::error(
                            "Value is not indexable",
                            base.span,
                            format!("expected an array, found `{other:?}`"),
                        ));

                        return HirExpression::error(expression.span);
                    }
                };

                if let Some(expected) = expected {
                    if *expected != Type::Error && *expected != element_type {
                        self.diagnostics.push(Diagnostic::error(
                            "Type mismatch",
                            expression.span,
                            format!(
                                "expected `{expected:?}`, but indexing this array produces `{element_type:?}`"
                            )
                        ));

                        return HirExpression::error(expression.span);
                    }
                }

                if let HirExpressionData::Integer(index_value) = &index.data {
                    let valid_index =
                        usize::try_from(*index_value).is_ok_and(|index| index < array_size);

                    if !valid_index {
                        self.diagnostics.push(Diagnostic::error(
                            "Array index out of bounds",
                            index.span,
                            format!("array length is {array_size}, but the index is {index_value}"),
                        ));

                        return HirExpression::error(expression.span);
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
                let amount: usize = match &size.as_ref().data {
                    ExpressionData::IntegerLiteral => {
                        self.source.span_text(size.span).parse().unwrap()
                    }

                    other => todo!("Unsupported expression for array init size"),
                };

                let base_type = if let Some(expected) = expected {
                    match expected {
                        Type::FixedArray { base_type, size } => {
                            if *size != amount {
                                self.diagnostics.push(Diagnostic::error(
                                    "Array Length Mismatch",
                                    expression.span,
                                    format!(
                                        "Expected Array of length {size} but got length {amount}"
                                    ),
                                ));
                                return HirExpression::error(expression.span);
                            }
                            Some(base_type.as_ref())
                        }

                        other => {
                            self.diagnostics.push(Diagnostic::error(
                                "Type mismatch",
                                expression.span,
                                format!(
                                    "expected expression of type `{:?}` but got `{:?}`",
                                    expected, other
                                ),
                            ));
                            return HirExpression::error(expression.span);
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
                        self.diagnostics.push(Diagnostic::error(
                            "Type mismatch",
                            expression.span,
                            format!(
                                "Expected expression of type `{:?}` but got `{:?}`",
                                expected,
                                Type::Bool
                            ),
                        ));
                        return HirExpression::error(expression.span);
                    }
                }

                HirExpression {
                    span: expression.span,
                    type_: Type::Bool,
                    data: HirExpressionData::Bool(*bool),
                }
            }

            ExpressionData::Binary { lhs, operator, rhs } => {
                let expected_type = if operator.requires_number_operands() {
                    Some(&Type::I64)
                } else {
                    None
                };

                let lhs = self.analyze_expression(lhs, expected_type);

                let expected_rhs_type = if let Some(expected_type) = expected_type {
                    expected_type
                } else {
                    &lhs.type_
                };

                let rhs = self.analyze_expression(rhs, Some(expected_rhs_type));

                if lhs.type_ == Type::Error || rhs.type_ == Type::Error {
                    return HirExpression::error(expression.span);
                }

                if matches!(operator, BinaryOperator::Equal | BinaryOperator::NotEqual)
                    && !matches!(lhs.type_, Type::I64 | Type::Bool)
                {
                    self.diagnostics.push(Diagnostic::error(
                        "Type does not support Equality",
                        expression.span,
                        format!("{:?}", lhs.type_),
                    ));
                    return HirExpression::error(expression.span);
                }

                let resulting_type = match operator {
                    BinaryOperator::Equal
                    | BinaryOperator::NotEqual
                    | BinaryOperator::GreaterThanOrEqual
                    | BinaryOperator::GreaterThan
                    | BinaryOperator::LessThanOrEqual
                    | BinaryOperator::LessThan => Type::Bool,

                    BinaryOperator::Add
                    | BinaryOperator::Subtract
                    | BinaryOperator::Multiply
                    | BinaryOperator::Divide => Type::I64,
                };

                if let Some(expected) = expected {
                    if *expected != resulting_type {
                        self.diagnostics.push(Diagnostic::error(
                            "Type mismatch",
                            expression.span,
                            format!(
                                "Expected `{:?}`, but this operator produces `{:?}`",
                                expected, resulting_type
                            ),
                        ));

                        return HirExpression::error(expression.span);
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

            ExpressionData::Call { callee, arguments } => {
                let callee = self.analyze_expression(callee, None);
                if callee.type_ == Type::Error {
                    return HirExpression::error(expression.span);
                }

                let (parameter_types, return_type) = match &callee.type_ {
                    Type::Function {
                        parameters,
                        return_type,
                    } => (parameters.clone(), return_type.as_ref().clone()),

                    actual_type => {
                        self.diagnostics.push(Diagnostic::error(
                            "Expression is not callable",
                            callee.span,
                            format!("expected a function, found `{:?}`", actual_type),
                        ));

                        return HirExpression::error(expression.span);
                    }
                };

                if arguments.len() != parameter_types.len() {
                    self.diagnostics.push(Diagnostic::error(
                        "Incorrect number of arguments",
                        expression.span,
                        format!(
                            "expected {} arguments, found {}",
                            parameter_types.len(),
                            arguments.len()
                        ),
                    ));

                    return HirExpression::error(expression.span);
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
                    return HirExpression::error(expression.span);
                }

                if let Some(expected_type) = expected {
                    if *expected_type != Type::Error && *expected_type != return_type {
                        self.diagnostics.push(Diagnostic::error(
                            "Call result has the wrong type",
                            expression.span,
                            format!("Expected `{:?}`, found `{:?}`", expected_type, return_type),
                        ));

                        return HirExpression::error(expression.span);
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
                let return_type = self.resolve_type_expression(&function.return_type);

                let parameter_types = function
                    .parameters
                    .iter()
                    .map(|parameter| self.resolve_type_expression(&parameter.type_annotation))
                    .collect::<Vec<_>>();

                self.environment.push_scope();

                let mut hir_parameters = Vec::new();
                let mut names = HashMap::new();

                for (parameter, parameter_type) in
                    function.parameters.iter().zip(parameter_types.iter())
                {
                    let name = self.source.span_text(parameter.name).to_owned();

                    if let Some(old) = names.insert(name.clone(), parameter.name) {
                        self.diagnostics.push(Diagnostic::error_with_extra_labels(
                            "Duplicate parameter name",
                            parameter.name,
                            "duplicate",
                            vec![Label {
                                span: old,
                                text: "already defined here".to_owned(),
                            }],
                        ));
                    }

                    let symbol_id = self.symbols.insert(Symbol {
                        name: name.clone(),
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

                let body = self.analyze_block(&function.body, &return_type);

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
                if let Some(expected) = expected {
                    if *expected != Type::I64 {
                        self.diagnostics.push(Diagnostic::error(
                            "Type mismatch".to_owned(),
                            expression.span,
                            format!(
                                "Expected an expression of type `{:?}` but got a number",
                                expected
                            ),
                        ));
                        return HirExpression::error(expression.span);
                    }
                }

                let parsed = self.source.span_text(expression.span).parse::<i64>();

                let Ok(parsed) = parsed else {
                    self.diagnostics.push(Diagnostic::error(
                        "Number overflow".to_owned(),
                        expression.span,
                        ":(".to_owned(),
                    ));
                    return HirExpression::error(expression.span);
                };

                HirExpression {
                    type_: Type::I64,
                    span: expression.span,
                    data: HirExpressionData::Integer(parsed),
                }
            }

            ExpressionData::Name => {
                let name = self.source.span_text(expression.span);
                let symbol_id = self.environment.lookup(name);
                let Some(symbol_id) = symbol_id else {
                    self.diagnostics.push(Diagnostic::error(
                        "Identifier not bound",
                        expression.span,
                        format!("Identifier {name} is not bound"),
                    ));
                    return HirExpression::error(expression.span);
                };

                let actual_type = self.symbols.get(symbol_id).type_.clone();

                if actual_type == Type::Unknown {
                    self.diagnostics.push(Diagnostic::error(
                        "Identifier not yet bound",
                        expression.span,
                        format!("Identifier {name} is not yet bound, in the future this will be allowed but rn stuff is evaluated top to bottom"),
                    ));
                    return HirExpression::error(expression.span);
                }

                if actual_type == Type::Error {
                    // The binding already produced a diagnostic
                    return HirExpression::error(expression.span);
                }

                if let Some(expected_type) = expected {
                    if *expected_type != Type::Error && *expected_type != actual_type {
                        self.diagnostics.push(Diagnostic::error(
                            format!(
                                "Expected an expression of type `{:?}` but got an expression of type `{:?}`",
                                expected_type, actual_type
                            ),
                            expression.span,
                            format!("Should be of type `{:?}`", expected_type),
                        ));

                        return HirExpression::error(expression.span);
                    }
                }

                HirExpression {
                    span: expression.span,
                    type_: actual_type,
                    data: HirExpressionData::Symbol(symbol_id),
                }
            }
        }
    }

    fn analyze_block(&mut self, block: &Block, return_type: &Type) -> HirBlock {
        self.environment.push_scope();

        let mut statements = Vec::new();

        for statement in &block.statements {
            statements.push(self.analyze_statement(statement, return_type));
        }

        self.environment.pop_scope();

        HirBlock {
            statements,
            span: block.span,
        }
    }

    fn analyze_statement(&mut self, statement: &Statement, return_type: &Type) -> HirStatement {
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

                let while_block = self.analyze_block(while_block, return_type);

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

                let then_block = self.analyze_block(then_block, return_type);

                let else_branch = match else_ {
                    None => None,
                    Some(ElseBranch::Else(block)) => {
                        Some(HirElseBranch::Else(self.analyze_block(block, return_type)))
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
                let name = self.source.span_text(binding.name).to_owned();

                let annotated_type = binding
                    .type_annotation
                    .as_ref()
                    .map(|annotation| self.resolve_type_expression(annotation));

                let expression =
                    self.analyze_expression(&binding.expression, annotated_type.as_ref());

                if let Some(previous_id) = self.environment.lookup_current(&name) {
                    let previous = self.symbols.get(previous_id);

                    self.diagnostics.push(Diagnostic::error_with_extra_labels(
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
                    ))
                }

                let symbol_id = self.symbols.insert(Symbol {
                    name: name.clone(),
                    declaration_span: Some(binding.name),
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
                        self.diagnostics.push(Diagnostic::error(
                            "Return without value in function with non-unit return type".to_owned(),
                            statement.span,
                            format!("Expected a `{:?}`", return_type),
                        ));

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
}
