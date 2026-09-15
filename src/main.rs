use crate::diagnostics::{Diagnostic, PrintDiagnostics};
use crate::elaboration::Elaborator;
use crate::parser::Parser;
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
mod elaboration;
pub mod util;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TargetInfo {
    pub pointer_bit_width: u32,
}

impl TargetInfo {
    pub fn native() -> TargetInfo {
        TargetInfo {
            pointer_bit_width: usize::BITS
        }
    }
}

struct CompilationTimings {
    tokenize: Duration,
    parse: Duration,
    elaboration: Duration,
    llvm_ir: Duration,
}

impl CompilationTimings {
    fn before_llvm(&self) -> Duration {
        self.tokenize + self.parse + self.elaboration
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
        "std".to_owned(),
        r#"
        comp add = fn(a: i64, b: i64) -> i64 {
            return a + b;
        }
        "#.to_owned()
    );

    let entry = file_manager.add_file(
        file.clone(),
        fs::read_to_string(&file).expect("Failed to read file"),
    );

    let target_info = TargetInfo::native();

    match compile(&file_manager, entry, target_info) {
        Err(diagnostics) => {
            diagnostics.print_diagnostics(&mut file_manager);

            std::process::exit(1);
        }
        Ok((llvm, timings)) => {
            let entry_file = file_manager.get_file(entry);
            let source_path = Path::new(&entry_file.name);

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
                    print_duration("Elaboration", timings.elaboration);
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

fn compile(sources: &SourceFileManager, entry: SourceId, target: TargetInfo) -> Result<(String, CompilationTimings), Vec<Diagnostic>> {
    let source = sources.get_file(entry);

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


    let (program_result, elaboration_time) = measure(|| {
        Elaborator::new(sources, entry, target).elaborate(ast)
    });

    let elaborated = program_result?;

    let (llvm_result, llvm_time) = measure(|| llvm_inkwell::emit(&elaborated, target));
    let llvm = llvm_result.map_err(|error| {
        vec![Diagnostic::error(
            error,
            Span {
                source: entry,
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
            elaboration: elaboration_time,
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

        let result = compile(&sources, id, TargetInfo::native());

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

    #[test]
    fn comptime_array_length() {
        assert_return_value(
            r#"
            comp N = 4usize * 2;

            comp main = fn() -> i64 {
                let values: [N]i64 = [0; N];
                return values[7];
            };
            "#,
            0
        )
    }

    #[test]
    fn accepts_inferred_comptime_string() {
        assert_return_value(
            r#"
          comp symbol = "putchar";

          comp main = fn() -> i64 {
              return 0;
          };
          "#,
            0,
        );
    }

    #[test]
    fn accepts_unicode_comptime_string() {
        assert_return_value(
            r#"
          comp message = "héllo 世界";

          comp main = fn() -> i64 {
              return 0;
          };
          "#,
            0,
        );
    }

    #[test]
    fn accepts_string_comptime_parameter() {
        assert_return_value(
            r#"
          comp select = fn<symbol: str>() -> i64 {
              return 42;
          };

          comp main = fn() -> i64 {
              return select<"putchar">();
          };
          "#,
            42,
        );
    }

    #[test]
    fn rejects_string_where_integer_is_expected() {
        assert_compile_error(
            r#"
          comp invalid = fn() -> i64 {
              return "putchar";
          };

          comp main = fn() -> i64 {
              return 0;
          };
          "#,
            "Type mismatch",
        );
    }

    #[test]
    fn rejects_string_in_runtime_local() {
        assert_compile_error(
            r#"
          comp main = fn() -> i64 {
              let symbol = "putchar";
              return 0;
          };
          "#,
            "`str` has no runtime representation",
        );
    }

    #[test]
    fn rejects_string_runtime_parameter() {
        assert_compile_error(
            r#"
          comp invalid = fn(symbol: str) -> i64 {
              return 0;
          };

          comp main = fn() -> i64 {
              return 0;
          };
          "#,
            "`str` has no runtime representation",
        );
    }

    #[test]
    fn rejects_string_runtime_return_type() {
        assert_compile_error(
            r#"
          comp invalid = fn() -> str {
              return "putchar";
          };

          comp main = fn() -> i64 {
              return 0;
          };
          "#,
            "`str` has no runtime representation",
        );
    }

    #[test]
    fn rejects_string_runtime_parameter_after_specialization() {
        assert_compile_error(
            r#"
          comp invalid = fn<tag: i64>(symbol: str) -> i64 {
              return tag;
          };

          comp main = fn() -> i64 {
              return invalid<1>("putchar");
          };
          "#,
            "`str` has no runtime representation",
        );
    }

    #[test]
    fn accepts_comptime_function_type_value() {
        assert_return_value(
            r#"
          comp Signature = fn(i32) -> i64;

          comp main = fn() -> i64 {
              return 0;
          };
          "#,
            0,
        );
    }

    #[test]
    fn accepts_zero_parameter_function_type_value() {
        assert_return_value(
            r#"
          comp Signature = fn() -> i32;

          comp main = fn() -> i64 {
              return 0;
          };
          "#,
            0,
        );
    }

    #[test]
    fn accepts_nested_function_type_value() {
        assert_return_value(
            r#"
          comp Callback = fn(i32) -> i64;
          comp HigherOrder = fn(fn(i32) -> i64) -> i64;

          comp main = fn() -> i64 {
              return 0;
          };
          "#,
            0,
        );
    }

    #[test]
    fn rejects_function_type_value_where_integer_is_expected() {
        assert_compile_error(
            r#"
          comp invalid = fn() -> i64 {
              return fn(i32) -> i64;
          };

          comp main = fn() -> i64 {
              return 0;
          };
          "#,
            "Type mismatch",
        );
    }

    #[test]
    fn rejects_type_value_in_runtime_local() {
        assert_compile_error(
            r#"
          comp main = fn() -> i64 {
              let Signature = fn(i32) -> i64;
              return 0;
          };
          "#,
            "`type` has no runtime representation",
        );
    }

    #[test]
    fn rejects_type_as_runtime_parameter() {
        assert_compile_error(
            r#"
          comp invalid = fn(Signature: type) -> i64 {
              return 0;
          };

          comp main = fn() -> i64 {
              return 0;
          };
          "#,
            "`type` has no runtime representation",
        );
    }

    #[test]
    fn rejects_type_as_runtime_return() {
        assert_compile_error(
            r#"
          comp invalid = fn() -> type {
              return fn(i32) -> i64;
          };

          comp main = fn() -> i64 {
              return 0;
          };
          "#,
            "`type` has no runtime representation",
        );
    }

    #[test]
    fn accepts_function_type_as_comptime_argument() {
        assert_return_value(
            r#"
          comp inspect = fn<Signature: type>() -> i64 {
              return 42;
          };

          comp main = fn() -> i64 {
              return inspect<fn(i32) -> i64>();
          };
          "#,
            42,
        );
    }

    #[test]
    fn compiles_and_calls_extern_function() {
        let llvm = compile_text(
            r#"
          comp c_put_char =
              @extern<"C", "putchar", fn(i32) -> i32>;

          comp main = fn() -> i64 {
              let result = c_put_char(65i32);
              return 0;
          };
          "#,
        )
            .expect("extern function should compile");

        assert!(
            llvm.contains("declare i32 @putchar(i32)"),
            "generated LLVM:\n{llvm}",
        );

        assert!(
            llvm.contains("call i32 @putchar(i32 65)"),
            "generated LLVM:\n{llvm}",
        );
    }

    #[test]
    fn compiles_extern_with_computed_arguments() {
        let llvm = compile_text(
            r#"
          comp ABI = "C";
          comp NAME = "getchar";
          comp Signature = fn() -> i32;

          comp c_get_char = @extern<ABI, NAME, Signature>;

          comp main = fn() -> i64 {
              let character = c_get_char();
              return 0;
          };
          "#,
        )
            .expect("extern function with computed arguments should compile");

        assert!(
            llvm.contains("declare i32 @getchar()"),
            "generated LLVM:\n{llvm}",
        );

        assert!(
            llvm.contains("call i32 @getchar()"),
            "generated LLVM:\n{llvm}",
        );
    }

    #[test]
    fn rejects_too_few_extern_arguments() {
        assert_compile_error(
            r#"
          comp f = @extern<"C", "getchar">;
          "#,
            "Incorrect number of arguments to `@extern`",
        );
    }

    #[test]
    fn rejects_too_many_extern_arguments() {
        assert_compile_error(
            r#"
          comp f = @extern<
              "C",
              "getchar",
              fn() -> i32,
              "extra"
          >;
          "#,
            "Incorrect number of arguments to `@extern`",
        );
    }

    #[test]
    fn rejects_non_string_extern_abi() {
        assert_compile_error(
            r#"
          comp f = @extern<true, "getchar", fn() -> i32>;
          "#,
            "Type mismatch",
        );
    }

    #[test]
    fn rejects_non_string_extern_link_name() {
        assert_compile_error(
            r#"
          comp f = @extern<"C", 123, fn() -> i32>;
          "#,
            "Type mismatch",
        );
    }

    #[test]
    fn rejects_non_type_extern_signature() {
        assert_compile_error(
            r#"
          comp f = @extern<"C", "getchar", 123>;
          "#,
            "Type mismatch",
        );
    }

    #[test]
    fn rejects_non_function_extern_type() {
        assert_compile_error(
            r#"
          comp f = @extern<"C", "getchar", i32>;
          "#,
            "Invalid external function type",
        );
    }

    #[test]
    fn rejects_unsupported_external_abi() {
        assert_compile_error(
            r#"
          comp f = @extern<"Rust", "getchar", fn() -> i32>;
          "#,
            "Unsupported external ABI",
        );
    }

    #[test]
    fn rejects_unknown_specialized_intrinsic() {
        assert_compile_error(
            r#"
          comp f = @whatever<"C">;
          "#,
            "Unknown intrinsic",
        );
    }
}
