use ariadne::Source;
use std::fmt::{Debug, Display};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct SourceId(pub usize);

pub struct SourceFileManager {
    files: Vec<SourceFile>,
}

impl SourceFileManager {
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
        }
    }

    pub fn add_file(&mut self, name: String, content: String) -> SourceId {
        let new_id = SourceId(self.files.len());
        self.files.push(SourceFile {
            name,
            source: Source::from(content),
            id: new_id,
        });

        new_id
    }

    pub fn files(&self) -> &[SourceFile] {
        &self.files
    }

    pub fn get_file(&self, id: SourceId) -> &SourceFile {
        if id.0 >= self.files.len() {
            panic!("source index out of bounds");
        }
        &self.files[id.0]
    }
}

impl ariadne::Cache<SourceId> for SourceFileManager {
    type Storage = String;

    fn fetch(&mut self, id: &SourceId) -> Result<&Source<Self::Storage>, impl Debug> {
        let file = self.get_file(*id);
        Ok::<&Source, &str>(&file.source)
    }

    fn display<'a>(&self, id: &'a SourceId) -> Option<impl Display + 'a> {
        Some(self.get_file(*id).name.clone())
    }
}

impl AsRef<str> for SourceFile {
    fn as_ref(&self) -> &str {
        self.source.text()
    }
}

pub struct SourceFile {
    pub id: SourceId,
    pub name: String,
    pub source: Source,
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
        return self.span(start_span.start, end_span.end);
    }

    pub fn span_text(&self, span: Span) -> &str {
        &self.source.text()[span.start..span.end]
    }

    pub fn text(&self) -> &str {
        self.source.text()
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Span {
    pub source: SourceId,
    pub start: usize,
    pub end: usize,
}

impl ariadne::Span for Span {
    type SourceId = SourceId;

    fn source(&self) -> &Self::SourceId {
        &self.source
    }

    fn start(&self) -> usize {
        self.start
    }

    fn end(&self) -> usize {
        self.end
    }
}