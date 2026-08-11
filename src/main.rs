use std::{env, fs};
use crate::diagnostics::PrintDiagnostics;
use crate::source::SourceFileManager;
use crate::tokenizer::Tokenizer;

pub mod source;
pub mod diagnostics;
pub mod tokenizer;

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
            println!("{tokens:#?}");
        }
    }
}
