use crate::parser::ast::Phase;
use crate::semantic::types::Type;
use crate::source::{SourceId, Span};
use std::collections::HashMap;
use crate::util::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModuleId(usize);

impl From<usize> for ModuleId {
    fn from(value: usize) -> Self {
        Self(value)
    }
}

impl From<ModuleId> for usize {
    fn from(value: ModuleId) -> Self {
        value.0
    }
}


pub struct Module {
    pub source: SourceId,
    pub scope: ScopeId
}

pub type ModuleStore = Store<ModuleId, Module>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScopeId(usize);

pub struct Scope {
    parent: Option<ScopeId>,
    bindings: HashMap<String, SymbolId>,
}

impl Scope {
    pub fn new(parent: ScopeId) -> Self {
        Self {
            parent: Some(parent),
            bindings: HashMap::new(),
        }
    }
    pub fn get(&self, key: &str) -> Option<&SymbolId> {
        self.bindings.get(key)
    }
}

pub struct Environment {
    scopes: Vec<Scope>,
    current: ScopeId,
}

impl Environment {
    pub fn current_scope(&self) -> ScopeId {
        self.current
    }

    pub fn new() -> Self {
        let root = Scope {
            parent: None,
            bindings: HashMap::new(),
        };

        Environment {
            scopes: vec![root],
            current: ScopeId(0),
        }
    }

    pub fn define(&mut self, name: String, symbol: SymbolId) {
        self.scopes[self.current.0].bindings.insert(name, symbol);
    }

    pub fn push_scope(&mut self) {
        let parent = self.current;

        let new = Scope::new(parent);
        self.scopes.push(new);

        self.current = ScopeId(self.scopes.len() - 1);
    }

    pub fn pop_scope(&mut self) {
        let parent = self.scopes[self.current.0]
            .parent
            .expect("cannot leave the root scope");

        self.current = parent;
    }

    pub fn lookup(&self, name: &str) -> Option<SymbolId> {
        self.lookup_from(self.current, name)
    }

    pub fn lookup_from(&self, scope: ScopeId, name: &str) -> Option<SymbolId> {
        let mut scope_id = Some(scope);

        while let Some(id) = scope_id {
            let scope = &self.scopes[id.0];

            if let Some(symbol) = scope.bindings.get(name) {
                return Some(*symbol);
            }

            scope_id = scope.parent;
        }

        None
    }

    pub fn lookup_current(&self, name: &str) -> Option<SymbolId> {
        self.scopes[self.current.0].bindings.get(name).copied()
    }
}

#[derive(Debug)]
pub struct SymbolTable {
    pub symbols: Vec<Symbol>,
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

    pub fn find_by_name(&self, name: &str) -> Option<(SymbolId, &Symbol)> {
        let (id, symbol) = self
            .symbols
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
    pub type_: Type,
}

#[derive(Debug)]
pub enum SymbolKind {
    BuiltinType(Type),
    Binding { phase: Phase, mutable: bool },
    ComptimeParameter,
    Parameter,
    Local { mutable: bool },
    ExternFunction { abi: ExternAbi, link_name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExternAbi {
    C,
}
