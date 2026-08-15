use crate::comptime::{ComptimeFunction, ComptimeValue, EvaluatedProgram};
use crate::diagnostics::Diagnostic;
use crate::parser::ast::Phase;
use crate::semantic::hir::{HirExpressionData, HirStatementData};
use crate::semantic::symbols::SymbolKind;
use crate::semantic::types::Type;
use crate::source::{SourceId, Span};

pub fn emit(program: &EvaluatedProgram) -> Result<String, Vec<Diagnostic>> {
    let symbols = &program.symbols;
    let functions = &program.functions;
    let values = &program.values;

    let mut llvm = String::new();

    let Some((main_id, main_symbol)) = symbols.find_by_name("main") else {
        return Err(vec![Diagnostic::error(
            "No main method found",
            Span {
                source: SourceId(0),
                start: 0,
                end: 0,
            },
            "=(",
        )]);
    };

    let SymbolKind::Binding { phase, mutable } = &main_symbol.kind else {
        return Err(vec![Diagnostic::error(
            "`main` needs to be an immutable comptime binding for a runtime function",
            main_symbol.declaration_span.unwrap(),
            "Evil !",
        )]);
    };

    if *phase != Phase::Comptime || *mutable == true {
        todo!("main cant be runtime binding or mutable");
    }

    let Some(main_value) = values.get(main_id) else {
        return Err(vec![Diagnostic::error(
            "`main` has no compile-time value",
            main_symbol.declaration_span.unwrap(),
            "`main` must be available at compile time",
        )]);
    };

    let ComptimeValue::Function(function_id) = main_value else {
        return Err(vec![Diagnostic::error(
            "`main` is not a function",
            main_symbol.declaration_span.unwrap(),
            "expected a function value",
        )]);
    };

    let Some(main_fn) = functions.get(*function_id) else {
        todo!("`main` is not a function");
    };

    if main_fn.hir.return_type != Type::I64 {
        todo!("`main` must return an i64 rn");
    }

    if !main_fn.hir.parameters.is_empty() {
        todo!("`main` must have no parameters");
    }

    let Ok(body) = emit_i64_body(&main_fn) else {
        todo!("explosion");
    };

    llvm.push_str("define i64 @flang_main() {\n");
    llvm.push_str("entry:\n");
    llvm.push_str(&body);
    llvm.push_str("}\n\n");

    llvm.push_str(
        "define i32 @main() {\n\
        entry:\n\
          %result = call i64 @flang_main()\n\
          %status = trunc i64 %result to i32\n\
          ret i32 %status\n\
        }\n"
    );

    Ok(llvm)
}

fn emit_i64_body(
    function: &ComptimeFunction
) -> Result<String, Diagnostic> {
    let [statement] = function.hir.body.statements.as_slice() else {
        todo!("For now: Requires exactly one statement in main");
    };

    match &statement.data {
        HirStatementData::Return(Some(expression)) => {
            match &expression.data {
                HirExpressionData::Integer(value) => {
                    Ok(format!("  ret i64 {value}\n"))
                }

                _ => {
                    todo!("For now: Requires an integer literal return in main");
                }
            }
        }

        HirStatementData::Return(None) => {
            todo!("No empty returns in main")
        }
    }
}
