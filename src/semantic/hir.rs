use crate::parser::ast::{BinaryOperator, Expression, Phase};
use crate::semantic::symbols::SymbolId;
use crate::semantic::types::Type;
use crate::source::Span;

#[derive(Debug, Clone)]
pub struct HirProgram {
    pub bindings: Vec<HirBinding>
}

#[derive(Debug, Clone)]
pub struct HirItem {
    pub span: Span,
    pub data: HirItemData,
}

#[derive(Debug, Clone)]
pub enum HirItemData {
    Binding(HirBinding),
}

#[derive(Debug, Clone)]
pub struct HirBinding {
    pub symbol: SymbolId,
    pub phase: Phase,
    pub mutable: bool,
    pub expression: HirExpression,
    pub span: Span
}

#[derive(Debug, Clone)]
pub struct HirExpression {
    pub type_: Type,
    pub span: Span,
    pub data: HirExpressionData,
}

impl HirExpression {
    pub fn error(span: Span) -> Self {
        Self {
            type_: Type::Error,
            data: HirExpressionData::Error,
            span
        }
    }
}

#[derive(Debug, Clone)]
pub enum HirExpressionData {
    Function(HirFunctionExpression),
    Integer(i64),
    Symbol(SymbolId),
    Binary {
        lhs: Box<HirExpression>,
        operator: BinaryOperator,
        rhs: Box<HirExpression>,
    },
    Error,
}

#[derive(Debug, Clone)]
pub struct HirFunctionExpression {
    pub parameters: Vec<HirParameter>,
    pub return_type: Type,
    pub body: HirBlock,
}

#[derive(Debug, Clone)]
pub struct HirParameter {
    pub name: Span,
    pub type_: Type,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct HirBlock {
    pub statements: Vec<HirStatement>,
    pub span: Span
}

#[derive(Debug, Clone)]
pub struct HirStatement {
    pub span: Span,
    pub data: HirStatementData,
}

#[derive(Debug, Clone)]
pub enum HirStatementData {
    Return(Option<HirExpression>),
}