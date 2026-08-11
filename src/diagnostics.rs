use ariadne::{Color, Report, ReportKind};
use ariadne::Label as AriadneLabel;
use crate::source::{SourceFile, SourceFileManager, Span};

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

impl PrintDiagnostics for Vec<Diagnostic> {
    fn print_diagnostics(&self, file_manager: &mut SourceFileManager) {
        for diagnostic in self.iter() {
            let mut report = Report::build(match diagnostic.severity {
                Severity::Error => ReportKind::Error,
                Severity::Warning => ReportKind::Warning,
            }, diagnostic.primary.span)
                .with_message(&diagnostic.message)
                .with_label(
                    AriadneLabel::new(diagnostic.primary.span)
                        .with_message(&diagnostic.primary.text)
                        .with_color(Color::Red),
                );

            for label in &diagnostic.others {
                report = report.with_label(
                    AriadneLabel::new(label.span)
                        .with_message(&label.text)
                        .with_color(Color::Yellow),
                );
            }

            report
                .finish()
                .eprint(&mut *file_manager)
                .expect("failed to print diagnostic");
        }
    }
}

impl Diagnostic {
    fn new(severity: Severity, message: String, primary: Label, others: Vec<Label>) -> Self {
        Self {
            severity, message, primary, others,
        }
    }

    pub fn error(
        message: String,
        span: Span,
        text: String,
    ) -> Self {
        Self::new(Severity::Error, message, Label::new(span, text), vec![])
    }

    pub fn warning(
        message: String,
        span: Span,
        text: String,
    ) -> Self {
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