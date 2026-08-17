use std::collections::HashMap;
use crate::parser::ast::Phase;
use crate::semantic::types::Type;
use crate::source::Span;

pub struct Environment {
    scopes: Vec<HashMap<String, SymbolId>>,
}

impl Environment {
    pub fn new() -> Self {
        Environment { scopes: vec![HashMap::new()] }
    }

    pub fn define(&mut self, name: String, symbol: SymbolId) {
        self.scopes.last_mut().unwrap().insert(name, symbol);
    }

    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub fn pop_scope(&mut self) {
        if self.scopes.len() == 1 {
            panic!("Tried to pop environment scope when only base scope was available");
        }
        self.scopes.pop();
    }

    pub fn lookup(&self, name: &str) -> Option<SymbolId> {
        self.scopes
            .iter()
            .rev()
            .find_map(
                |scope|
                    scope.get(name).copied()
            )
    }

    pub fn lookup_current(&self, name: &str) -> Option<SymbolId> {
        self.scopes
            .last()
            .and_then(|scope| scope.get(name).copied())
    }
}

#[derive(Debug)]
pub struct SymbolTable {
    pub symbols: Vec<Symbol>
}

impl SymbolTable {
    pub fn new() -> Self {
        SymbolTable { symbols: vec![] }
    }

    pub fn insert(&mut self, symbol: Symbol) -> SymbolId {
        let id = SymbolId(self.symbols.len() as u32);
        self.symbols.push(symbol);
        id
    }

    pub fn get(&self, id: SymbolId) -> &Symbol {
        &self.symbols[id.0 as usize]
    }

    pub fn get_mut(&mut self, id: SymbolId) -> &mut Symbol {
        &mut self.symbols[id.0 as usize]
    }

    pub fn find_by_name(&self, name: &str)
        -> Option<(SymbolId, &Symbol)>
    {
        let (id, symbol) = self.symbols
            .iter()
            .enumerate()
            .find(|(_, symbol)| symbol.name == name)?;

        Some((SymbolId(id as u32), symbol))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SymbolId(pub u32);

#[derive(Debug)]
pub struct Symbol {
    pub name: String,
    pub declaration_span: Option<Span>,
    pub kind: SymbolKind,
    pub type_: Type
}

#[derive(Debug)]
pub enum SymbolKind {
    BuiltinType(Type),
    Binding {
        phase: Phase,
        mutable: bool,
    },
    Parameter,
    Local {
        mutable: bool,
    },
}