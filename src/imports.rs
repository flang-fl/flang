use crate::diagnostics::Diagnostic;
use crate::parser::Parser;
use crate::parser::ast::{
    Binding, Block, ElseBranch, Expression, ExpressionData, ItemData, Program, Statement,
    StatementData, TypeExpression, TypeExpressionData,
};
use crate::source::{SourceFileManager, SourceId, Span};
use crate::tokenizer::Tokenizer;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub fn load_imports(
    sources: &mut SourceFileManager,
    entry: SourceId,
) -> Result<LoadedProgram, Vec<Diagnostic>> {
    let entry_file = sources.get_file(entry);

    let entry_path = Path::new(&entry_file.name)
        .canonicalize()
        .map_err(|error| {
            vec![Diagnostic::error(
                "Cannot resolve entry file",
                entry_file.span(0, 0),
                format!("Cannot resolve `{}`: {error}", entry_file.name),
            )]
        })?;

    let mut loader = Loader {
        sources,
        files: HashMap::new(),
        result: LoadedProgram {
            modules: Vec::new(),
            imports: Vec::new(),
        },
    };

    loader
        .files
        .insert(entry_path.clone(), (entry, LoadState::Loading));

    loader.visit(entry)?;

    loader.files.insert(entry_path, (entry, LoadState::Loaded));

    Ok(loader.result)
}

struct Loader<'src> {
    sources: &'src mut SourceFileManager,
    files: HashMap<PathBuf, (SourceId, LoadState)>,
    result: LoadedProgram,
}

impl Loader<'_> {
    fn visit(&mut self, source: SourceId) -> Result<(), Vec<Diagnostic>> {
        let program = {
            let file = self.sources.get_file(source);
            let tokens = Tokenizer::new(file).tokenize()?;
            Parser::new(file, &tokens).parse()?
        };

        let imports = collect_imports(&program);

        for path_span in imports {
            let path = resolve_import_path(self.sources, path_span)
                .map_err(|diagnostic| vec![diagnostic])?;

            let target = match self.files.get(&path).copied() {
                Some((_, LoadState::Loading)) => {
                    return Err(vec![Diagnostic::error(
                        "Import cycle",
                        path_span,
                        format!("`{}` is already being loaded", path.display()),
                    )]);
                }

                Some((target, LoadState::Loaded)) => target,

                None => {
                    let content = fs::read_to_string(&path).map_err(|error| {
                        vec![Diagnostic::error(
                            "Cannot read import",
                            path_span,
                            format!("Cannot read `{}`: {error}", path.display()),
                        )]
                    })?;

                    let target = self
                        .sources
                        .add_file(path.to_string_lossy().into_owned(), content);

                    self.files
                        .insert(path.clone(), (target, LoadState::Loading));

                    self.visit(target)?;

                    self.files.insert(path, (target, LoadState::Loaded));

                    target
                }
            };

            self.result.imports.push(ResolvedImport {
                path: path_span,
                target,
            });
        }

        self.result.modules.push(LoadedModule { source, program });

        Ok(())
    }
}

pub struct LoadedModule {
    pub source: SourceId,
    pub program: Program,
}

pub struct ResolvedImport {
    pub path: Span,
    pub target: SourceId,
}

pub struct LoadedProgram {
    pub modules: Vec<LoadedModule>,
    pub imports: Vec<ResolvedImport>,
}

#[derive(Clone, Copy)]
pub enum LoadState {
    Loading,
    Loaded,
}

pub fn resolve_import_path(
    sources: &SourceFileManager,
    path_span: Span,
) -> Result<PathBuf, Diagnostic> {
    let literal = sources.span_text(path_span);

    let path = literal
        .strip_prefix('"')
        .and_then(|text| text.strip_suffix('"'))
        .expect("import paths should be string literals");

    if !path.starts_with("./") && !path.starts_with("../") {
        return Err(Diagnostic::error(
            "Unsupported import path",
            path_span,
            "Use a path beginning with `./` or `../`",
        ));
    }

    let importing_file = sources.get_file(path_span.source);
    let parent = Path::new(&importing_file.name)
        .parent()
        .unwrap_or_else(|| Path::new("."));

    let candidate = parent.join(path);

    candidate.canonicalize().map_err(|error| {
        Diagnostic::error(
            "Cannot resolve import",
            path_span,
            format!("Cannot resolve `{}`: {error}", candidate.display()),
        )
    })
}

pub fn collect_imports(program: &Program) -> Vec<Span> {
    let mut imports = Vec::new();

    for item in &program.items {
        let ItemData::Binding(binding) = &item.data;
        visit_binding(binding, &mut imports);
    }

    imports
}

fn visit_binding(binding: &Binding, imports: &mut Vec<Span>) {
    if let Some(annotation) = &binding.type_annotation {
        visit_type(annotation, imports);
    }

    visit_expression(&binding.expression, imports);
}

fn visit_type(type_: &TypeExpression, imports: &mut Vec<Span>) {
    match &type_.data {
        TypeExpressionData::Identifier | TypeExpressionData::Unit => {}

        TypeExpressionData::FixedArray { size, base_type } => {
            visit_expression(size, imports);
            visit_type(base_type, imports);
        }

        TypeExpressionData::Function {
            parameters,
            return_type,
        } => {
            for parameter in parameters {
                visit_type(parameter, imports);
            }

            visit_type(return_type, imports);
        }
    }
}

fn visit_expression(expression: &Expression, imports: &mut Vec<Span>) {
    match &expression.data {
        ExpressionData::Import { path } => imports.push(*path),

        ExpressionData::Function(function) => {
            for parameter in function.comptime_args.iter().chain(&function.runtime_args) {
                visit_type(&parameter.type_annotation, imports);
            }

            visit_type(&function.return_type, imports);
            visit_block(&function.body, imports);
        }

        ExpressionData::TypeValue(type_) => visit_type(type_, imports),

        ExpressionData::Member { base, .. } => {
            visit_expression(base, imports);
        }

        ExpressionData::Unary { operand, .. } => {
            visit_expression(operand, imports);
        }

        ExpressionData::Binary { lhs, rhs, .. } => {
            visit_expression(lhs, imports);
            visit_expression(rhs, imports);
        }

        ExpressionData::Call { callee, arguments }
        | ExpressionData::Specialize { callee, arguments } => {
            visit_expression(callee, imports);

            for argument in arguments {
                visit_expression(argument, imports);
            }
        }

        ExpressionData::ArrayRepeatInitialization { value, size } => {
            visit_expression(value, imports);
            visit_expression(size, imports);
        }

        ExpressionData::Index { base, index } => {
            visit_expression(base, imports);
            visit_expression(index, imports);
        }

        ExpressionData::Intrinsic { .. }
        | ExpressionData::IntegerLiteral
        | ExpressionData::StringLiteral
        | ExpressionData::Boolean(_)
        | ExpressionData::Name => {}
    }
}

fn visit_block(block: &Block, imports: &mut Vec<Span>) {
    for statement in &block.statements {
        visit_statement(statement, imports);
    }
}

fn visit_statement(statement: &Statement, imports: &mut Vec<Span>) {
    match &statement.data {
        StatementData::Binding(binding) => {
            visit_binding(binding, imports);
        }

        StatementData::Return(value) => {
            if let Some(value) = value {
                visit_expression(value, imports);
            }
        }

        StatementData::Expression(expression) => {
            visit_expression(expression, imports);
        }

        StatementData::Assignment { target, expression } => {
            visit_expression(target, imports);
            visit_expression(expression, imports);
        }

        StatementData::While(while_) => {
            visit_expression(&while_.condition, imports);
            visit_block(&while_.while_block, imports);
        }

        StatementData::If(if_) => {
            visit_expression(&if_.condition, imports);
            visit_block(&if_.then_block, imports);

            match &if_.else_ {
                Some(ElseBranch::Else(block)) => {
                    visit_block(block, imports);
                }
                Some(ElseBranch::ElseIf(statement)) => {
                    visit_statement(statement, imports);
                }
                None => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;
    use crate::source::SourceFileManager;
    use crate::tokenizer::Tokenizer;

    #[test]
    fn discovers_top_level_and_nested_imports() {
        let mut sources = SourceFileManager::new();

        let id = sources.add_file(
            "main.fl".into(),
            r#"
              comp library = @import("./library.fl");

              comp main = fn() -> i64 {
                  if false {
                      return @import("./unused.fl").answer;
                  }

                  return @import("./other.fl").answer;
              };
              "#
                .into(),
        );

        let source = sources.get_file(id);
        let tokens = Tokenizer::new(source)
            .tokenize()
            .expect("tokenization should succeed");

        let program = Parser::new(source, &tokens)
            .parse()
            .expect("parsing should succeed");

        let imports = collect_imports(&program);

        let paths: Vec<_> = imports
            .iter()
            .map(|span| sources.span_text(*span))
            .collect();

        assert_eq!(
            paths,
            vec![r#""./library.fl""#, r#""./unused.fl""#, r#""./other.fl""#,],
        );

        assert!(imports.iter().all(|span| span.source == id));
    }

    #[test]
    fn resolves_imports_relative_to_the_importing_file() {
        let directory = tempfile::tempdir().expect("create temp directory");
        let nested = directory.path().join("nested");

        std::fs::create_dir(&nested).expect("create nested directory");

        let library = directory.path().join("library.fl");
        std::fs::write(&library, "pub comp answer = 42;").expect("write library");

        let mut sources = SourceFileManager::new();

        let id = sources.add_file(
            nested.join("main.fl").to_string_lossy().into_owned(),
            r#""../library.fl""#.into(),
        );

        let source = sources.get_file(id);
        let span = source.span(0, source.text().len());

        let resolved = resolve_import_path(&sources, span).expect("relative import should resolve");

        assert_eq!(
            resolved,
            library.canonicalize().expect("canonical library path"),
        );
    }

    #[test]
    fn missing_import_reports_the_import_path() {
        let directory = tempfile::tempdir().expect("create temp directory");
        let mut sources = SourceFileManager::new();

        let id = sources.add_file(
            directory
                .path()
                .join("main.fl")
                .to_string_lossy()
                .into_owned(),
            r#""./missing.fl""#.into(),
        );

        let source = sources.get_file(id);
        let span = source.span(0, source.text().len());

        let diagnostic =
            resolve_import_path(&sources, span).expect_err("missing import should fail");

        assert_eq!(diagnostic.message, "Cannot resolve import");
        assert_eq!(diagnostic.primary.span.source, id);
        assert_eq!(
            sources.span_text(diagnostic.primary.span),
            r#""./missing.fl""#,
        );
    }

    #[test]
    fn loads_a_dependency_only_once() {
        let directory = tempfile::tempdir().expect("create temp directory");

        let entry_path = directory.path().join("main.fl");
        let library_path = directory.path().join("library.fl");

        let entry_text = r#"
          comp first = @import("./library.fl");
          comp second = @import("./library.fl");
      "#;

        std::fs::write(&entry_path, entry_text).expect("write entry");
        std::fs::write(&library_path, "pub comp answer = 42;").expect("write library");

        let mut sources = SourceFileManager::new();
        let entry = sources.add_file(entry_path.to_string_lossy().into_owned(), entry_text.into());

        let loaded = load_imports(&mut sources, entry).expect("imports should load");

        assert_eq!(sources.files().len(), 2);
        assert_eq!(loaded.modules.len(), 2);
        assert_eq!(loaded.imports.len(), 2);
        assert_eq!(loaded.imports[0].target, loaded.imports[1].target,);

        assert_eq!(loaded.modules.last().expect("entry module").source, entry,);
    }

    #[test]
    fn nested_imports_share_one_dependency() {
        let directory = tempfile::tempdir().expect("create temp directory");
        let nested = directory.path().join("nested");
        std::fs::create_dir(&nested).expect("create nested directory");

        let entry_path = directory.path().join("main.fl");
        let entry_text = r#"
          comp left = @import("./nested/left.fl");
          comp right = @import("./right.fl");
      "#;

        std::fs::write(&entry_path, entry_text).expect("write entry");

        std::fs::write(
            nested.join("left.fl"),
            r#"comp shared = @import("../shared.fl");"#,
        )
            .expect("write left");

        std::fs::write(
            directory.path().join("right.fl"),
            r#"comp shared = @import("./shared.fl");"#,
        )
            .expect("write right");

        std::fs::write(directory.path().join("shared.fl"), "pub comp answer = 42;")
            .expect("write shared");

        let mut sources = SourceFileManager::new();
        let entry = sources.add_file(entry_path.to_string_lossy().into_owned(), entry_text.into());

        let loaded = load_imports(&mut sources, entry).expect("dependency graph should load");

        assert_eq!(sources.files().len(), 4);
        assert_eq!(loaded.modules.len(), 4);
        assert_eq!(loaded.imports.len(), 4);

        let shared_targets: Vec<_> = loaded
            .imports
            .iter()
            .filter(|import| {
                matches!(
                    sources.span_text(import.path),
                    "\"../shared.fl\"" | "\"./shared.fl\""
                )
            })
            .map(|import| import.target)
            .collect();

        assert_eq!(shared_targets.len(), 2);
        assert_eq!(shared_targets[0], shared_targets[1]);

        assert_eq!(loaded.modules.last().expect("entry module").source, entry,);
    }

    #[test]
    fn rejects_import_cycle() {
        let directory = tempfile::tempdir().expect("create temp directory");

        let entry_path = directory.path().join("main.fl");
        let other_path = directory.path().join("other.fl");

        let entry_text = r#"comp other = @import("./other.fl");"#;
        let other_text = r#"comp entry = @import("./main.fl");"#;

        std::fs::write(&entry_path, entry_text).expect("write entry");
        std::fs::write(&other_path, other_text).expect("write other");

        let mut sources = SourceFileManager::new();
        let entry = sources.add_file(entry_path.to_string_lossy().into_owned(), entry_text.into());

        // Avoid expect_err: LoadedProgram doesn't currently implement Debug.
        let diagnostics = match load_imports(&mut sources, entry) {
            Ok(_) => panic!("cycle should be rejected"),
            Err(diagnostics) => diagnostics,
        };

        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message == "Import cycle")
            .expect("expected an import-cycle diagnostic");

        assert_ne!(diagnostic.primary.span.source, entry);
        assert_eq!(sources.span_text(diagnostic.primary.span), r#""./main.fl""#,);

        // The entry was recognized, not loaded a second time.
        assert_eq!(sources.files().len(), 2);
    }
}
