use std::{env, fs};
use std::path::Path;
use crate::comptime::Evaluator;
use crate::diagnostics::{Diagnostic, PrintDiagnostics};
use crate::parser::Parser;
use crate::semantic::Analyzer;
use crate::source::{SourceFile, SourceFileManager};
use crate::tokenizer::Tokenizer;

pub mod source;
pub mod diagnostics;
pub mod tokenizer;
mod parser;
mod semantic;
mod comptime;
mod llvm;
mod toolchain;

fn main() {
    let mut args = env::args().skip(1);

    let Some(file) = args.next() else {
        println!("No file specified: cargo run -- <file>");
        return;
    };

    let mut file_manager = SourceFileManager::new();
    file_manager.add_file(file.clone(), fs::read_to_string(&file).expect("Failed to read file"));

    let [file] = file_manager.files() else {
        panic!("More than one file not currently supported");
    };

    match compile(file) {
        Err(diagnostics) => {
            diagnostics.print_diagnostics(&mut file_manager);
        }
        Ok(llvm) => {
            let source_path = Path::new(&file.name);

            let build_dir = source_path
                .parent()
                .unwrap_or(Path::new("."))
                .join("build");

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

            match toolchain::build_executable(
                &llvm,
                &ir_path,
                &executable_path,
            ) {
                Ok(()) => {
                    println!(
                        "built {}",
                        executable_path.display()
                    );
                }

                Err(error) => {
                    eprintln!("toolchain error: {error:#?}");
                }
            }
        }
    }
}

fn compile(
    source: &SourceFile,
) -> Result<String, Vec<Diagnostic>>
{
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

    fn compile_text(
        text: &str,
    ) -> Result<String, Vec<Diagnostic>> {
        let mut sources = SourceFileManager::new();

        let id = sources.add_file(
            "<test>".to_owned(),
            text.to_owned(),
        );

        compile(sources.get_file(id))
    }

    fn assert_compile_error(source: &str, expected: &str) {
        let diagnostics =
            compile_text(source).expect_err("expected compilation to fail");

        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.message.contains(expected)
                    || diagnostic.primary.text.contains(expected)
            }),
            "expected an error containing {expected:?}, got:\n{diagnostics:#?}",
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
        ).expect("program should compile");

        assert!(
            llvm.contains("define i64 @flang_main()"),
            "generated LLVM:\n{llvm}"
        );

        assert!(
            llvm.contains("ret i64 6"),
            "generated LLVM:\n{llvm}"
        );

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
            "Return without value"
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
            "expected `;` after binding"
        );
    }

    #[test]
    fn builds_and_runs_native_executable() {
        let llvm = compile_text(
            r#"
            comp main = fn() -> i64 {
                return 6;
            };
            "#
        ).expect("program should compile");

        let directory = tempfile::tempdir()
            .expect("temporary directory should be created");

        let ir_path = directory.path().join("main.ll");

        let executable = if env::consts::EXE_EXTENSION.is_empty() {
            directory.path().join("main")
        } else {
            directory.path().join("main.exe")
        };

        toolchain::build_executable(
            &llvm,
            &ir_path,
            &executable,
        )
            .expect("Clang should build the executable");

        let status = std::process::Command::new(&executable)
            .status()
            .expect("executable should run");

        assert_eq!(status.code(), Some(6));
    }
}