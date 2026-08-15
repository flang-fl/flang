use crate::source::Span;

#[derive(Debug, Clone)]
pub struct Program {
    pub items: Vec<Item>
}

#[derive(Debug, Clone)]
pub struct Item {
    pub span: Span,
    pub data: ItemData,
}

#[derive(Debug, Clone)]
pub enum ItemData {
    Binding(Binding),
}

#[derive(Debug, Clone)]
pub struct Binding {
    pub phase: Phase,
    pub mutable: bool,
    pub name: Span,
    pub type_annotation: Option<TypeExpression>,
    pub expression: Expression,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Comptime,
    Runtime
}

#[derive(Debug, Clone)]
pub struct Expression {
    pub span: Span,
    pub data: ExpressionData
}

#[derive(Debug, Clone)]
pub enum ExpressionData {
    Function(FunctionExpression),
    IntegerLiteral,
    Name,
    Binary {
        lhs: Box<Expression>,
        operator: BinaryOperator,
        rhs: Box<Expression>,
    },
    Call {
        callee: Box<Expression>,
        arguments: Vec<Expression>,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
}

#[derive(Debug, Clone)]
pub struct FunctionExpression {
    pub parameters: Vec<Parameter>,
    pub return_type: TypeExpression,
    pub body: Block,
}

#[derive(Debug, Clone)]
pub struct Parameter {
    pub name: Span,
    pub type_annotation: TypeExpression,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TypeExpression {
    pub span: Span,
    pub data: TypeExpressionData
}

#[derive(Debug, Clone)]
pub enum TypeExpressionData {
    Identifier,
    Unit
}

#[derive(Debug, Clone)]
pub struct Block {
    pub statements: Vec<Statement>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Statement {
    pub span: Span,
    pub data: StatementData
}

#[derive(Debug, Clone)]
pub enum StatementData {
    Return(Option<Expression>),
}