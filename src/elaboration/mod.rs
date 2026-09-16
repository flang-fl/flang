use crate::TargetInfo;
use crate::comptime::{ComptimeValue, FunctionId, FunctionStore, FunctionTemplateStore, ValueStore};
use crate::diagnostics::Diagnostic;
use crate::elaboration::dependencies::{PendingBinding, WorkStatus};
use crate::parser::ast::{ItemData, Program};
use crate::semantic::hir::{HirBinding, HirProgram};
use crate::semantic::symbols::{Environment, ExternAbi, Symbol, SymbolId, SymbolKind, SymbolTable};
use crate::semantic::types::{IntegerType, SpecializationKey, Type};
use crate::source::SourceFileManager;
use std::collections::{HashMap, HashSet};

mod analysis;
mod dependencies;
mod evaluation;

#[derive(Debug)]
pub struct ElaboratedProgram {
    pub symbols: SymbolTable,
    pub values: ValueStore,
    pub functions: FunctionStore,
    pub hir: HirProgram
}

pub struct Elaborator<'src> {
    target: TargetInfo,
    pub(super) sources: &'src SourceFileManager,
    
    pub(super) symbols: SymbolTable,
    pub(super) environment: Environment,
    pub(super) diagnostics: Vec<Diagnostic>,
    
    pub(super) pending_bindings:
        HashMap<SymbolId, PendingBinding>,
    
    pub(super) binding_order: Vec<SymbolId>,
    
    pub(super) elaboration_status:
        HashMap<SymbolId, WorkStatus>,
    
    pub(super) elaborated_bindings:
        HashMap<SymbolId, HirBinding>,
    
    pub(super) elaboration_stack: Vec<SymbolId>,
    
    pub(super) evaluation_status:
        HashMap<SymbolId, WorkStatus>,
    
    pub(super) evaluation_stack: Vec<SymbolId>,
    
    pub(super) values: ValueStore,
    pub(super) functions: FunctionStore,
    pub(super) function_templates: FunctionTemplateStore,
    pub(super) specializations: HashMap<SpecializationKey, FunctionId>,
    pub(super) active_specializations: HashSet<SpecializationKey>,

    pub(super) frames:
        Vec<HashMap<SymbolId, ComptimeValue>>
}

impl<'src> Elaborator<'src> {
    pub fn new(sources: &'src SourceFileManager, target: TargetInfo) -> Self {
        let mut symbols = SymbolTable::new();
        let mut environment = Environment::new();

        for integer_type in [
            IntegerType::U8,
            IntegerType::I8,
            IntegerType::U16,
            IntegerType::I16,
            IntegerType::U32,
            IntegerType::I32,
            IntegerType::U64,
            IntegerType::I64,
            IntegerType::Usize,
            IntegerType::Isize,
        ] {
            let name = integer_type.name();
            let symbol_id = symbols.insert(Symbol {
                name: name.to_owned(),
                kind: SymbolKind::BuiltinType(Type::Integer(integer_type)),
                declaration_span: None,
                type_: Type::Type
            });

            environment.define(name.to_owned(), symbol_id);
        }

        let type_id = symbols.insert(Symbol {
            name: "type".to_owned(),
            kind: SymbolKind::BuiltinType(Type::Type),
            declaration_span: None,
            type_: Type::Type
        });

        environment.define("type".to_owned(), type_id);

        let str_id = symbols.insert(Symbol {
            name: "str".to_owned(),
            kind: SymbolKind::BuiltinType(Type::Str),
            declaration_span: None,
            type_: Type::Type
        });
        
        environment.define("str".to_owned(), str_id);
        
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
            vec![Type::Integer(IntegerType::I64)],
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
            Type::Integer(IntegerType::I64),
        );

        Self::register_external_function(
            &mut symbols,
            &mut environment,
            "print_ascii",
            "flang_print_ascii",
            vec![Type::Integer(IntegerType::I64)],
            Type::Unit,
        );
        
        Self {
            target,
            sources,
            symbols,
            environment,
            diagnostics: Vec::new(),
            
            pending_bindings: HashMap::new(),
            binding_order: Vec::new(),
            
            elaboration_status: HashMap::new(),
            elaborated_bindings: HashMap::new(),
            elaboration_stack: Vec::new(),
            
            evaluation_status: HashMap::new(),
            evaluation_stack: Vec::new(),
            
            values: ValueStore::new(),
            functions: FunctionStore::new(),
            function_templates: FunctionTemplateStore::new(),
            specializations: HashMap::new(),
            active_specializations: HashSet::new(),
            frames: Vec::new(),
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
            kind: SymbolKind::ExternFunction { link_name, abi: ExternAbi::C },
            declaration_span: None,
            type_: Type::Function {
                parameters,
                return_type: Box::new(return_type),
            },
        };

        let symbol_id = symbols.insert(symbol);
        environment.define(name, symbol_id);
    }
    
    pub fn elaborate(
        mut self,
        program: Program
    ) -> Result<ElaboratedProgram, Vec<Diagnostic>> {
        self.collect_declarations(program);
        
        let order = self.binding_order.clone();
        
        for symbol in &order {
            let _ = self.ensure_binding_evaluated(*symbol);
        }
        
        if !self.diagnostics.is_empty() {
            return Err(self.diagnostics);
        }
        
        let bindings = order
            .iter()
            .filter_map(|symbol| {
                self.elaborated_bindings
                    .get(symbol).cloned()
            })
            .collect();
        
        Ok(ElaboratedProgram {
            hir: HirProgram { bindings },
            symbols: self.symbols,
            values: self.values,
            functions: self.functions
        })
    }

    fn collect_declarations(
        &mut self,
        program: Program
    ) {
        for item in program.items {
            let ItemData::Binding(binding) = item.data;
            
            let name = self.sources
                .span_text(binding.name)
                .to_owned();

            if let Some(_) = self.environment.lookup(&name) {
                self.diagnostics.push(Diagnostic::error(
                    "Duplicate binding found",
                    item.span,
                    "Evil :(",
                ));

                continue;
            }
            
            let symbol = Symbol {
                name: name.clone(),
                declaration_span: Some(binding.name),
                kind: SymbolKind::Binding {
                    phase: binding.phase,
                    mutable: binding.mutable
                },
                type_: Type::Unknown
            };
            
            let symbol_id = self.symbols.insert(symbol);
            self.environment.define(name, symbol_id);
            
            self.pending_bindings.insert(
                symbol_id,
                PendingBinding {
                    binding,
                    span: item.span
                }
            );
            
            self.binding_order.push(symbol_id);
            
            self.elaboration_status.insert(
                symbol_id,
                WorkStatus::Pending
            );
            
            self.evaluation_status.insert(
                symbol_id,
                WorkStatus::Pending
            );
        }
    }
}