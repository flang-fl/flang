use crate::semantic::hir::HirFunctionExpression;
use crate::semantic::symbols::SymbolId;
use crate::semantic::types::{IntegerType, Type};
use std::cmp::PartialEq;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum ComptimeValue {
    Integer {
        value: u64,
        type_: IntegerType
    },
    Function(FunctionId),
    Type(Type),
    Bool(bool),
    Unit,
    Error,
}

#[derive(Debug)]
pub struct ValueStore {
    map: HashMap<SymbolId, ComptimeValue>,
}

impl ValueStore {
    pub fn new() -> Self {
        Self {
            map: HashMap::new()
        }
    }

    pub fn insert(&mut self, symbol: SymbolId, value: ComptimeValue) {
        self.map.insert(symbol, value);
    }

    pub fn get(&self, symbol: SymbolId) -> Option<&ComptimeValue> {
        self.map.get(&symbol)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FunctionId(u32);

impl FunctionId {
    pub fn index(self) -> u32 {
        self.0
    }
}

#[derive(Debug)]
pub struct FunctionStore {
    functions: Vec<ComptimeFunction>,
}

impl FunctionStore {
    pub fn new() -> Self {
        Self {
            functions: Vec::new()
        }
    }

    pub fn insert(&mut self, function: ComptimeFunction) -> FunctionId {
        let id = FunctionId(self.functions.len() as u32);
        self.functions.push(function);
        id
    }

    pub fn get(&self, id: FunctionId) -> Option<&ComptimeFunction> {
        self.functions.get(id.0 as usize)
    }

    pub fn iter(&self) -> impl Iterator<Item = (FunctionId, &ComptimeFunction)> {
        self.functions
            .iter()
            .enumerate()
            .map(|(index, function)| (FunctionId(index as u32), function))
    }
}

#[derive(Debug)]
pub struct ComptimeFunction {
    pub hir: HirFunctionExpression,
}
