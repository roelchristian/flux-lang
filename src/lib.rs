//! Reference prototype for the Flux language.
pub mod builtins;
pub mod check;
mod csv;
pub mod diagnostic;
mod errors041;
pub mod eval;
pub mod hir;
pub mod hir_eval;
pub mod infer;
pub mod lexer;
pub mod parser;
pub mod resolve;
pub mod semantic;
pub mod types;

pub use csv::{FsInput, OpenedInput, RuntimeInput};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    pub start: usize,
    pub message: String,
    specification: Option<Box<diagnostic::Diagnostic>>,
}
impl Error {
    pub fn new(start: usize, message: impl Into<String>) -> Self {
        Self {
            start,
            message: message.into(),
            specification: None,
        }
    }

    pub fn structured(&self) -> Option<&diagnostic::Diagnostic> {
        self.specification.as_deref()
    }

    pub(crate) fn from_diagnostic(detail: diagnostic::Diagnostic) -> Self {
        Self {
            start: detail.primary.start,
            message: format!("{}: {}", detail.category.code(), detail.message),
            specification: Some(Box::new(detail)),
        }
    }

    pub fn diagnostic(&self, path: &str, source: &str) -> String {
        let position = (0..=self.start.min(source.len()))
            .rev()
            .find(|p| source.is_char_boundary(*p))
            .unwrap_or(0);
        let preceding = &source[..position];
        let line = preceding.bytes().filter(|c| *c == b'\n').count() + 1;
        let column = preceding.rsplit('\n').next().unwrap_or("").chars().count() + 1;
        let mut result = format!("{path}:{line}:{column}: {}", self.message);
        if let Some(detail) = self.structured() {
            let line_start = preceding.rfind('\n').map_or(0, |end| end + 1);
            let line_end = source[position..]
                .find('\n')
                .map_or(source.len(), |end| position + end);
            let end = detail.primary.end.min(line_end);
            if let Some(mark) = source.get(position..end).filter(|s| !s.is_empty()) {
                result.push_str(&format!(
                    "\n  {}\n  {}{}",
                    &source[line_start..line_end],
                    " ".repeat(column - 1),
                    "^".repeat(mark.chars().count().min(120)),
                ));
            }
            for (kind, value) in [
                ("expected", &detail.expected),
                ("actual", &detail.actual),
                ("constraint", &detail.constraint),
            ] {
                if let Some(value) = value {
                    result.push_str(&format!("\n  {kind}: {value}"));
                }
            }
            for related in &detail.related {
                let prefix = source.get(..related.span.start).unwrap_or("");
                let line = prefix.bytes().filter(|b| *b == b'\n').count() + 1;
                let column = prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1;
                result.push_str(&format!(
                    "\n  note: {path}:{line}:{column}: {}",
                    related.label
                ));
            }
            if let Some(suggestion) = &detail.suggestion {
                result.push_str(&format!("\n  help: {suggestion}"));
            }
        }
        result
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for Error {}

pub fn check_source(source: &str) -> Result<parser::Program, Error> {
    let program = parser::parse(source)?;
    semantic::prepare(
        &program,
        semantic::Mode::Script,
        semantic::Policy::Compatible,
    )?;
    Ok(program)
}

pub fn run_source(source: &str) -> Result<eval::Output, Error> {
    let program = parser::parse(source)?;
    semantic::prepare(
        &program,
        semantic::Mode::Script,
        semantic::Policy::Compatible,
    )?
    .run()
}

/// Conservative early formatter: normalize line endings and remove trailing whitespace.
/// Does not reorder operators, reindent code, or modify comments' textual content.
pub fn format_source(source: &str) -> Result<String, Error> {
    parser::parse(source)?;
    let mut result = source
        .lines()
        .map(|line| line.trim_end_matches([' ', '\t']))
        .collect::<Vec<_>>()
        .join("\n");
    if !result.is_empty() {
        result.push('\n');
    }
    let old = lexer::lex(source)?;
    let new = lexer::lex(&result)?;
    if old
        .iter()
        .map(|token| &token.text)
        .ne(new.iter().map(|token| &token.text))
    {
        return Err(Error::new(0, "formatting would change one or more tokens"));
    }
    Ok(result)
}

/// A source documentation line. Triple-hyphen comments have no runtime effect.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DocComment<'a> {
    pub line: usize,
    pub text: &'a str,
}

pub fn documentation_comments(source: &str) -> Result<Vec<DocComment<'_>>, Error> {
    lexer::lex(source)?;
    Ok(source
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let comment = line.trim_start_matches([' ', '\t']).strip_prefix("---")?;
            if comment.starts_with('-') {
                return None;
            }
            Some(DocComment {
                line: index + 1,
                text: comment.strip_prefix(' ').unwrap_or(comment).trim_end(),
            })
        })
        .collect())
}

#[cfg(test)]
mod format_tests {
    use super::*;

    #[test]
    fn preserves_guard_alignment_and_documents_only_standalone_lines() {
        let source = "--- absolute value\nabs(x:z):z := ?{\n    < x 0 : -: x;\n    _     : x;\n}; -- ordinary\n";
        assert_eq!(format_source(source).unwrap(), source);
        assert_eq!(
            documentation_comments(source).unwrap(),
            [DocComment {
                line: 1,
                text: "absolute value"
            }]
        );
        assert!(
            documentation_comments("---- ruler\ns := \"---not a comment\";")
                .unwrap()
                .is_empty()
        );
    }
}
