use crate::source::{SourceFileManager, SourceId, Span};
use ariadne::Source;
use std::fmt::{Debug, Display};

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