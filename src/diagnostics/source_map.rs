//! Locations in a linked tree of source files.
use crate::Error;
use std::{path::PathBuf, sync::Arc};

/// Original file text and its position in the graph's virtual byte-address space.
#[derive(Clone, Debug)]
pub struct SourceFile {
    pub path: PathBuf,
    pub base: usize,
    pub text: Arc<str>,
    pub(crate) label: String,
}
#[derive(Clone, Debug, Default)]
pub struct SourceMap {
    pub(crate) files: Vec<SourceFile>,
}
impl SourceMap {
    pub fn files(&self) -> &[SourceFile] {
        &self.files
    }
    pub(crate) fn file_at(&self, global: usize) -> Option<&SourceFile> {
        let i = self
            .files
            .partition_point(|file| file.base <= global)
            .checked_sub(1)?;
        let file = &self.files[i];
        (global - file.base <= file.text.len()).then_some(file)
    }
    /// Resolve a global byte offset, including a file's own EOF, to original source.
    pub fn locate(&self, global: usize) -> Option<(&str, &str, usize)> {
        let file = self.file_at(global)?;
        Some((&file.label, &file.text, global - file.base))
    }
    pub fn diagnostic(&self, error: &Error) -> String {
        error.diagnostic_with(|offset| self.locate(offset))
    }
}
