use ariadne::Source;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct SourceId(pub usize);

pub struct SourceFile {
    pub id: SourceId,
    pub name: String,
    pub source: Source,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
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

    pub fn eof_span(&self) -> Span {
        Span {
            source: self.id,
            start: self.source.len() - 1,
            end: self.source.len() - 1
        }
    }

    pub fn fromto(&self, start_span: Span, end_span: Span) -> Span {
        assert_eq!(start_span.source, end_span.source);
        self.span(start_span.start, end_span.end)
    }

    pub fn span_text(&self, span: Span) -> &str {
        let text = self.source.text();
        let start = Self::character_to_byte(text, span.start);
        let end = Self::character_to_byte(text, span.end);
        &self.source.text()[start..end]
    }

    fn character_to_byte(text: &str, character_offset: usize) -> usize {
        text.char_indices()
            .nth(character_offset)
            .map_or(text.len(), |(byte_offset, _)| byte_offset)
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