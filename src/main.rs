use crate::comptime::Evaluator;
use crate::diagnostics::{Diagnostic, PrintDiagnostics};
use crate::parser::Parser;
use crate::semantic::Analyzer;
use crate::source::{SourceFile, SourceFileManager, SourceId, Span};
use crate::tokenizer::Tokenizer;
use std::path::Path;
use std::time::{Duration, Instant};
use std::{env, fs};

mod comptime;
pub mod diagnostics;
mod llvm_inkwell;
mod parser;
mod semantic;
pub mod source;
pub mod tokenizer;
mod toolchain;

struct CompilationTimings {
    tokenize: Duration,
    parse: Duration,
    semantic: Duration,
    comptime: Duration,
    llvm_ir: Duration,
}

impl CompilationTimings {
    fn before_llvm(&self) -> Duration {
        self.tokenize + self.parse + self.semantic + self.comptime
    }

    fn compiler_total(&self) -> Duration {
        self.before_llvm() + self.llvm_ir
    }
}

fn print_duration(label: &str, duration: Duration) {
    eprintln!("{label:<24} {:>10.3} ms", duration.as_secs_f64() * 1000.0);
}

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
        Ok((llvm, timings)) => {
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

            let (build_exe_result, build_exe_time) =
                measure(|| toolchain::build_executable(&llvm, &ir_path, &executable_path));

            match build_exe_result {
                Ok(()) => {
                    println!("built {}", executable_path.display());

                    print_duration("Tokenization", timings.tokenize);
                    print_duration("Parsing", timings.parse);
                    print_duration("Semantic Analysis", timings.semantic);
                    print_duration("Comptime Evaluation", timings.comptime);
                    println!();
                    print_duration("Compilation", timings.before_llvm());
                    print_duration("LLVM Generation", timings.llvm_ir);
                    print_duration("Total", timings.compiler_total());
                    println!();
                    print_duration("Clang", build_exe_time);
                    print_duration("True Total", timings.compiler_total() + build_exe_time);
                    println!();
                }

                Err(error) => {
                    eprintln!("toolchain error: {error:#?}");
                }
            }
        }
    }
}

fn measure<T>(operation: impl FnOnce() -> T) -> (T, Duration) {
    let started = Instant::now();
    let result = operation();
    (result, started.elapsed())
}

fn compile(source: &SourceFile) -> Result<(String, CompilationTimings), Vec<Diagnostic>> {
    let (tokens, tokenize_time) = measure(|| {
        let tokenizer = Tokenizer::new(source);
        tokenizer.tokenize()
    });
    let tokens = tokens?;

    println!("=== Tokens");
    for token in tokens.iter() {
        println!("  {token:?}");
    }
    println!();

    let (ast, parse_time) = measure(|| {
        let parser = Parser::new(&source, &tokens);
        parser.parse()
    });
    let ast = ast?;

    println!("=== AST");
    println!("{ast:#?}");
    println!();

    let (semantic_program, semantic_time) = measure(|| {
        let analyzer = Analyzer::new(&source);
        analyzer.analyze(ast)
    });
    let semantic_program = semantic_program?;

    println!("=== Typed AST");
    println!("{semantic_program:#?}");
    println!();

    let (evaluated, comptime_time) = measure(|| {
        let compile_time_evaluator = Evaluator::new();
        compile_time_evaluator.evaluate(semantic_program)
    });
    let evaluated = evaluated?;

    println!("=== Compiletime Evaluated Program");
    println!("{evaluated:#?}");
    println!();

    let (llvm_result, llvm_time) = measure(|| llvm_inkwell::emit(&evaluated));
    let llvm = llvm_result.map_err(|error| {
        vec![Diagnostic::error(
            error,
            Span {
                source: SourceId(0),
                start: 0,
                end: 0,
            },
            ":(",
        )]
    })?;

    println!("=== LLVM");
    println!("{llvm}");
    println!();

    Ok((
        llvm,
        CompilationTimings {
            tokenize: tokenize_time,
            parse: parse_time,
            semantic: semantic_time,
            comptime: comptime_time,
            llvm_ir: llvm_time,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile_text(text: &str) -> Result<String, Vec<Diagnostic>> {
        let mut sources = SourceFileManager::new();

        let id = sources.add_file("<test>".to_owned(), text.to_owned());

        let result = compile(sources.get_file(id));

        result.map(|(compile, _time)| compile)
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
            40 + 2,
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
            2 + 3 * 4,
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
            8 - 3 - 1,
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
            1 + 2 * 3 - 4 / 5,
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

    #[test]
    fn runtime_if_takes_then_branch() {
        assert_return_value(
            r#"
            comp choose = fn(condition: bool) -> i64 {
                if condition {
                    return 11;
                } else {
                    return 22;
                }
            };

            comp main = fn() -> i64 {
                return choose(true);
            };
            "#,
            11,
        );
    }

    #[test]
    fn runtime_if_takes_else_branch() {
        assert_return_value(
            r#"
            comp choose = fn(condition: bool) -> i64 {
                if condition {
                    return 11;
                } else {
                    return 22;
                }
            };

            comp main = fn() -> i64 {
                return choose(false);
            };
            "#,
            22,
        );
    }

    #[test]
    fn runtime_else_if_takes_middle_branch() {
        assert_return_value(
            r#"
            comp classify = fn(value: i64) -> i64 {
               if value < 10 {
                   return 1;
               } else if value < 20 {
                   return 2;
               } else {
                   return 3;
               }
            };

            comp main = fn() -> i64 {
                return classify(15);
            };
            "#,
            2,
        );
    }

    #[test]
    fn if_without_else_can_continue() {
        assert_return_value(
            r#"
            comp choose = fn(condition: bool) -> i64 {
                if condition {
                    return 9;
                }

                return 4;
            };

            comp main = fn() -> i64 {
                return choose(false);
            };
            "#,
            4,
        );
    }

    #[test]
    fn comptime_if_selects_branch() {
        assert_return_value(
            r#"
            comp choose = fn(condition: bool) -> i64 {
                if condition {
                    return 31;
                } else {
                    return 32;
                }
            };

            comp selected = choose(false);

            comp main = fn() -> i64 {
                return selected;
            };
            "#,
            32,
        );
    }

    #[test]
    fn rejects_non_boolean_if_condition() {
        assert_compile_error(
            r#"
        comp main = fn() -> i64 {
            if 42 {
                return 1;
            } else {
                return 2;
            }
        };
        "#,
            "Type mismatch",
        );
    }

    #[test]
    fn branch_local_binding_does_not_escape() {
        assert_compile_error(
            r#"
        comp main = fn() -> i64 {
            if true {
                let answer = 42;
            }

            return answer;
        };
        "#,
            "Identifier not bound",
        );
    }
}
