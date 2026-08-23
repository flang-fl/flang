use ariadne::Source;
use crate::source::{SourceFile, SourceId};

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