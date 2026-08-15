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