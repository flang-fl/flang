use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug)]
pub enum ToolchainError {
    WriteIr {
        path: PathBuf,
        error: std::io::Error,
    },

    StartClang {
        error: std::io::Error,
    },

    ClangFailed {
        status: Option<i32>,
        stderr: String,
    }
}

pub fn build_executable(
    llvm_ir: &str,
    ir_path: &Path,
    executable_path: &Path,
) -> Result<(), ToolchainError> {
    fs::create_dir_all(executable_path.parent().unwrap()).map_err(|error| {
        ToolchainError::WriteIr {
            path: ir_path.to_owned(),
            error,
        }
    })?;

    fs::write(ir_path, llvm_ir).map_err(|error| {
        ToolchainError::WriteIr {
            path: ir_path.to_owned(),
            error,
        }
    })?;

    let output = Command::new("clang")
        .arg(ir_path)
        .arg("-o")
        .arg(executable_path)
        .output()
        .map_err(|error| {
            ToolchainError::StartClang { error }
        })?;

    if !output.status.success() {
        return Err(ToolchainError::ClangFailed {
            status: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    Ok(())
}