use crate::comptime::Evaluator;
use crate::diagnostics::{Diagnostic, PrintDiagnostics};
use crate::parser::Parser;
use crate::semantic::Analyzer;
use crate::source::{SourceFile, SourceFileManager};
use crate::tokenizer::Tokenizer;
use std::path::Path;
use std::{env, fs};

mod comptime;
pub mod diagnostics;
mod llvm;
mod parser;
mod semantic;
pub mod source;
pub mod tokenizer;
mod toolchain;

fn main() {
    let mut args = env::args().skip(1);

    let Some(file) = args.next() else {
        println!("No file specified: cargo run -- <file>");
        return;
    };

    let mut file_manager = SourceFileManager::new();
    file_manager.add_file(
        file.clone(),
        fs::read_to_string(&file).expect("Failed to read file"),
    );

    let [file] = file_manager.files() else {
        panic!("More than one file not currently supported");
    };

    match compile(file) {
        Err(diagnostics) => {
            diagnostics.print_diagnostics(&mut file_manager);
        }
        Ok(llvm) => {
            let source_path = Path::new(&file.name);

            let build_dir = source_path.parent().unwrap_or(Path::new(".")).join("build");

            let stem = source_path
                .file_stem()
                .expect("source path should have a filename");

            let artifact_base = build_dir.join(stem);

            let ir_path = artifact_base.with_extension("ll");

            let executable_path = if env::consts::EXE_EXTENSION.is_empty() {
                artifact_base.clone()
            } else {
                artifact_base.with_extension(env::consts::EXE_EXTENSION)
            };

            match toolchain::build_executable(&llvm, &ir_path, &executable_path) {
                Ok(()) => {
                    println!("built {}", executable_path.display());
                }

                Err(error) => {
                    eprintln!("toolchain error: {error:#?}");
                }
            }
        }
    }
}

fn compile(source: &SourceFile) -> Result<String, Vec<Diagnostic>> {
    let tokenizer = Tokenizer::new(&source);
    let tokens = tokenizer.tokenize()?;
    println!("=== Tokens");
    for token in tokens.iter() {
        println!("  {token:?}");
    }
    println!();

    let parser = Parser::new(&source, &tokens);
    let ast = parser.parse()?;
    println!("=== AST");
    println!("{ast:#?}");
    println!();

    let analyzer = Analyzer::new(&source);
    let typed_ast = analyzer.analyze(ast)?;
    println!("=== Typed AST");
    println!("{typed_ast:#?}");
    println!();

    let compile_time_evaluator = Evaluator::new();
    let evaluated = compile_time_evaluator.evaluate(typed_ast)?;
    println!("=== Compiletime Evaluated Program");
    println!("{evaluated:#?}");
    println!();

    let llvm = llvm::emit(&evaluated)?;
    println!("=== LLVM");
    println!("{llvm}");
    println!();

    Ok(llvm)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile_text(text: &str) -> Result<String, Vec<Diagnostic>> {
        let mut sources = SourceFileManager::new();

        let id = sources.add_file("<test>".to_owned(), text.to_owned());

        compile(sources.get_file(id))
    }

    fn assert_compile_error(source: &str, expected: &str) {
        let diagnostics = compile_text(source).expect_err("expected compilation to fail");

        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.message.contains(expected) || diagnostic.primary.text.contains(expected)
            }),
            "expected an error containing {expected:?}, got:\n{diagnostics:#?}",
        );
    }

    fn assert_return_value(source: &str, expected: i32) {
        let llvm = compile_text(source).expect("program should compile");

        let directory = tempfile::tempdir().expect("temporary directory should be created");

        let ir_path = directory.path().join("main.ll");

        let executable = if env::consts::EXE_EXTENSION.is_empty() {
            directory.path().join("main")
        } else {
            directory.path().join("main.exe")
        };

        toolchain::build_executable(&llvm, &ir_path, &executable)
            .expect("Clang should build the executable");

        let status = std::process::Command::new(&executable)
            .status()
            .expect("executable should run");

        assert_eq!(
            status.code().expect("program should have return value"),
            expected
        );
    }

    #[test]
    fn compiles_integer_main_to_llvm() {
        let llvm = compile_text(
            r#"
            comp main = fn() -> i64 {
                return 6;
            };
            "#,
        )
        .expect("program should compile");

        assert!(
            llvm.contains("define i64 @flang_fn_0()"),
            "generated LLVM:\n{llvm}"
        );

        assert!(llvm.contains("ret i64 6"), "generated LLVM:\n{llvm}");

        assert!(
            llvm.contains("define i32 @main()"),
            "generated LLVM:\n{llvm}"
        )
    }

    #[test]
    fn rejects_empty_return_from_i64_function() {
        assert_compile_error(
            r#"
            comp main = fn() -> i64 {
                return;
            };
            "#,
            "Return without value",
        );
    }

    #[test]
    fn rejects_unknown_return_type() {
        assert_compile_error(
            r#"
            comp main = fn() -> mystery {
                return 6;
            };
            "#,
            "Unknown Type",
        );
    }

    #[test]
    fn binding_requires_trailing_semicolon() {
        assert_compile_error(
            r#"
            comp main = fn() -> i64 {
                return 6;
            }
            "#,
            "expected `;` after binding",
        );
    }

    #[test]
    fn builds_and_runs_native_executable() {
        assert_return_value(
            r#"
            comp main = fn() -> i64 {
                return 6;
            };
            "#,
            6,
        );
    }

    #[test]
    fn simple_addition_and_comptime_variable() {
        assert_return_value(
            r#"
            comp answer = 40;

            comp main = fn() -> i64 {
                return answer + 2;
            };
            "#,
            40 + 2
        );
    }

    #[test]
    fn multiplicative_precedence() {
        assert_return_value(
            r#"
            comp main = fn() -> i64 {
                return 2 + 3 * 4;
            };
            "#,
            2 + 3 * 4
        );
    }

    #[test]
    fn left_associative() {
        assert_return_value(
            r#"
            comp main = fn() -> i64 {
                return 8 - 3 - 1;
            };
            "#,
            8 - 3 - 1
        );
    }

    #[test]
    fn all_llvm_arithmetic() {
        assert_return_value(
            r#"
            comp main = fn() -> i64 {
                return 1 + 2 * 3 - 4 / 5;
            };
            "#,
            1 + 2 * 3 - 4 / 5
        );
    }

    #[test]
    fn comptime_division_by_zero() {
        assert_compile_error(
            r#"
            comp test = 5 / 0;

            comp main = fn() -> i64 {
                return test;
            };
            "#,
            "Division by zero",
        );
    }

    #[test]
    fn comptime_overflow() {
        assert_compile_error(
            r#"
            comp overflowed = 9223372036854775806 + 2;

            comp main = fn() -> i64 {
                return overflowed;
            };
            "#,
            "overflow",
        );
    }
}
