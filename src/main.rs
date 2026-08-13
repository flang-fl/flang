use std::{env, fs};
use crate::comptime::Evaluator;
use crate::diagnostics::PrintDiagnostics;
use crate::parser::Parser;
use crate::semantic::Analyzer;
use crate::source::SourceFileManager;
use crate::tokenizer::Tokenizer;

pub mod source;
pub mod diagnostics;
pub mod tokenizer;
mod parser;
mod semantic;
mod comptime;
mod llvm;

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

    let mut tokenizer = Tokenizer::new(&file);

    let result = tokenizer.tokenize();
    match result {
        Err(diagnostics) => {
            diagnostics.print_diagnostics(&mut file_manager);
        }
        Ok(tokens) => {
            for token in &tokens {
                println!("{:?}", token);
            }

            let mut parser = Parser::new(&file, &tokens);
            let result = parser.parse();
            
            match result {
                Err(diagnostics) => {
                    diagnostics.print_diagnostics(&mut file_manager);
                }
                Ok(program) => {
                    println!("{:#?}", program);
                    println!();

                    let analyzer = Analyzer::new(&file);
                    let analyzed = analyzer.analyze(
                        program
                    );

                    match analyzed {
                        Err(diagnostics) => {
                            diagnostics.print_diagnostics(&mut file_manager);
                        }
                        Ok(semantic_program) => {
                            println!("{:#?}", semantic_program);

                            let evaluator = Evaluator::new(&file);

                            match evaluator.evaluate(semantic_program) {
                                Err(diagnostics) => {
                                    diagnostics.print_diagnostics(&mut file_manager);
                                }
                                Ok(evaluated_program) => {
                                    println!("{evaluated_program:#?}");
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
