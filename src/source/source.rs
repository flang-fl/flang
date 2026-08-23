use ariadne::Source;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct SourceId(pub usize);

pub struct SourceFile {
    pub id: SourceId,
    pub name: String,
    pub source: Source,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Span {
    pub source: SourceId,
    pub start: usize,
    pub end: usize,
}

impl SourceFile {
    pub fn span(&self, start: usize, end: usize) -> Span {
        Span {
            source: self.id,
            start,
            end
        }
    }

    pub fn fromto(&self, start_span: Span, end_span: Span) -> Span {
        assert_eq!(start_span.source, end_span.source);
        self.span(start_span.start, end_span.end)
    }

    pub fn span_text(&self, span: Span) -> &str {
        &self.source.text()[span.start..span.end]
    }

    pub fn text(&self) -> &str {
        self.source.text()
    }
}


impl AsRef<str> for SourceFile {
    fn as_ref(&self) -> &str {
        self.source.text()
    }
}