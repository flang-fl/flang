use crate::source::{SourceFileManager, Span};
use codespan_reporting::diagnostic::{Diagnostic as CodespanDiagnostic, Label as CodespanLabel};
use codespan_reporting::files::SimpleFiles;
use codespan_reporting::term::{
    self,
    termcolor::{ColorChoice, StandardStream},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    // Maybe some kind of "note" thing that you can do something in a better way
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub primary: Label,
    pub others: Vec<Label>,
}

pub trait PrintDiagnostics {
    fn print_diagnostics(&self, file_manager: &mut SourceFileManager);
}

impl PrintDiagnostics for [Diagnostic] {
    fn print_diagnostics(&self, file_manager: &mut SourceFileManager) {
        if self.is_empty() {
            return;
        }

        let mut files = SimpleFiles::new();

        // Both managers assign IDs sequentially in insertion order.
        // Borrow names and source text without copying their contents.
        for file in file_manager.files() {
            let id = files.add(file.name.as_str(), file.text());
            debug_assert_eq!(id, file.id.0);
        }

        let config = term::Config {
            before_label_lines: 2,
            after_label_lines: 2,
            ..Default::default()
        };

        let stderr = StandardStream::stderr(ColorChoice::Auto);
        let mut writer = stderr.lock();

        for diagnostic in self {
            let report = match diagnostic.severity {
                Severity::Error => CodespanDiagnostic::error(),
                Severity::Warning => CodespanDiagnostic::warning(),
            };

            let primary = &diagnostic.primary;
            let mut labels = vec![
                CodespanLabel::primary(primary.span.source.0, primary.span.start..primary.span.end)
                    .with_message(primary.text.as_str()),
            ];

            labels.extend(diagnostic.others.iter().map(|label| {
                CodespanLabel::secondary(label.span.source.0, label.span.start..label.span.end)
                    .with_message(label.text.as_str())
            }));

            let report = report
                .with_message(diagnostic.message.as_str())
                .with_labels(labels);

            term::emit_to_write_style(&mut writer, &config, &files, &report)
                .expect("failed to print diagnostic");
        }
    }
}

impl Diagnostic {
    fn new(severity: Severity, message: String, primary: Label, others: Vec<Label>) -> Self {
        Self {
            severity,
            message,
            primary,
            others,
        }
    }

    pub fn error(message: impl Into<String>, span: Span, text: impl Into<String>) -> Self {
        Self::new(
            Severity::Error,
            message.into(),
            Label::new(span, text.into()),
            vec![],
        )
    }

    pub fn error_with_extra_labels(
        message: impl Into<String>,
        span: Span,
        text: impl Into<String>,
        extra_labels: Vec<Label>,
    ) -> Self {
        Self::new(
            Severity::Error,
            message.into(),
            Label::new(span, text.into()),
            extra_labels,
        )
    }

    pub fn warning(message: String, span: Span, text: String) -> Self {
        Self::new(Severity::Warning, message, Label::new(span, text), vec![])
    }
}

#[derive(Debug, Clone)]
pub struct Label {
    pub span: Span,
    pub text: String,
}

impl Label {
    pub fn new(span: Span, text: String) -> Self {
        Self { span, text }
    }
}
