use super::formatter;
use crate::{modules, parser, Error, PathError};

/// Parse a file and its local imports, then format only the original root text.
pub fn format_path(path: impl AsRef<std::path::Path>) -> Result<String, PathError> {
    let loaded = modules::load_for_format(path)?;
    let base = loaded
        .sources
        .files()
        .iter()
        .find(|file| file.path == loaded.root)
        .map_or(0, |file| file.base);
    formatter::format(
        loaded.root_source(),
        base,
        loaded.program.formatting_applications(),
    )
    .map_err(|error| PathError::semantic(&loaded, error))
}

/// Format original source without changing lexemes or adjacency-sensitive syntax.
pub fn format_source(source: &str) -> Result<String, Error> {
    let program = parser::parse_with_layout(source)?;
    formatter::format(source, 0, program.formatting_applications())
}
