//! Reference prototype for the Flux language.
pub mod builtins;
#[cfg(any(test, feature = "legacy-oracle"))]
#[allow(dead_code)]
#[path = "check.rs"]
mod legacy_check;
#[cfg(any(test, feature = "legacy-oracle"))]
#[allow(dead_code)]
#[path = "eval.rs"]
mod legacy_eval;

pub mod check {
    use crate::{
        parser::Program,
        semantic::{prepare, Mode, Policy},
        Error,
    };

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct Inspection {
        pub type_name: String,
        pub schema: Option<String>,
    }

    pub fn check(program: &Program) -> Result<(), Error> {
        prepare(program, Mode::Script, Policy::TypedOnly).map(|_| ())
    }

    pub fn inspect_entry(program: &Program) -> Result<Inspection, Error> {
        prepare(program, Mode::Inspection, Policy::TypedOnly)?
            .inspection()
            .cloned()
            .ok_or_else(|| Error::new(0, "no expression to inspect"))
    }
}

#[cfg(feature = "legacy-oracle")]
pub mod oracle {
    pub use crate::legacy_check::check;
    pub use crate::legacy_eval::{run, Output};
}
mod csv;
pub mod diagnostic;
mod errors041;
pub mod eval {
    use crate::{
        parser::Program,
        semantic::{prepare, Mode, Policy},
        Error,
    };

    #[derive(Debug)]
    pub struct Output {
        pub stdout: String,
        pub status: u8,
    }

    #[derive(Debug)]
    pub struct ReplOutput {
        pub value: String,
        pub stdout: String,
    }

    pub fn run(program: &Program) -> Result<Output, Error> {
        prepare(program, Mode::Script, Policy::TypedOnly)?.run()
    }

    pub fn view_entry(program: &Program) -> Result<ReplOutput, Error> {
        prepare(program, Mode::Inspection, Policy::TypedOnly)?.view()
    }
}
pub mod hir;
pub mod hir_eval;
pub mod infer;
pub mod lexer;
pub mod modules;
pub mod parser;
pub mod resolve;
pub mod semantic;
pub mod types;

pub use csv::{FsInput, OpenedInput, RuntimeInput};

// Linked names encode a canonical path, but their source names are display metadata.
fn linked_prefix(name: &str) -> Option<usize> {
    let head = name.strip_prefix('\0')?;
    let (tag, _) = head.split_once(':')?;
    (!tag.is_empty()
        && tag.len() % 2 == 0
        && tag
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
    .then_some(tag.len() + 2)
}

pub(crate) fn visible_symbol(name: &str) -> &str {
    linked_prefix(name).map_or(name, |prefix| &name[prefix..])
}

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
        self.diagnostic_with(|offset| Some((path, source, offset)))
    }

    /// Render global byte offsets against their original source, including
    /// notes whose source file differs from the primary error.
    pub fn diagnostic_with<'a>(
        &self,
        locate: impl Fn(usize) -> Option<(&'a str, &'a str, usize)>,
    ) -> String {
        let coordinates = |source: &str, offset: usize| {
            let position = (0..=offset.min(source.len()))
                .rev()
                .find(|p| source.is_char_boundary(*p))
                .unwrap_or(0);
            let preceding = &source[..position];
            let line = preceding.bytes().filter(|c| *c == b'\n').count() + 1;
            let column = preceding.rsplit('\n').next().unwrap_or("").chars().count() + 1;
            (position, line, column)
        };
        let Some((path, source, offset)) = locate(self.start) else {
            return self.message.clone();
        };
        let (position, line, column) = coordinates(source, offset);
        let mut result = format!("{path}:{line}:{column}: {}", self.message);
        if let Some(detail) = self.structured() {
            let line_start = source[..position].rfind('\n').map_or(0, |end| end + 1);
            let line_end = source[position..]
                .find('\n')
                .map_or(source.len(), |end| position + end);
            let end = offset
                .saturating_add(detail.primary.end.saturating_sub(self.start))
                .min(line_end);
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
                if let Some((path, source, offset)) = locate(related.span.start) {
                    let (_, line, column) = coordinates(source, offset);
                    result.push_str(&format!(
                        "\n  note: {path}:{line}:{column}: {}",
                        related.label
                    ));
                } else {
                    result.push_str(&format!("\n  note: {}", related.label));
                }
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
        semantic::Policy::TypedOnly,
    )?;
    Ok(program)
}

pub fn run_source(source: &str) -> Result<eval::Output, Error> {
    let program = parser::parse(source)?;
    semantic::prepare(
        &program,
        semantic::Mode::Script,
        semantic::Policy::TypedOnly,
    )?
    .run()
}

/// Failure from a path-based frontend or runtime operation.
#[derive(Debug)]
pub enum PathError {
    Load(modules::LoadError),
    Semantic { error: Error, rendered: String },
}

impl PathError {
    pub fn diagnostic(&self) -> String {
        match self {
            Self::Load(error) => error.diagnostic(),
            Self::Semantic { rendered, .. } => rendered.clone(),
        }
    }

    pub fn semantic(loaded: &modules::LoadedProgram, error: Error) -> Self {
        let rendered = loaded.diagnostic(&error);
        Self::Semantic { error, rendered }
    }
}

impl From<modules::LoadError> for PathError {
    fn from(error: modules::LoadError) -> Self {
        Self::Load(error)
    }
}

impl std::fmt::Display for PathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.diagnostic())
    }
}

impl std::error::Error for PathError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Load(error) => Some(error),
            Self::Semantic { error, .. } => Some(error),
        }
    }
}

/// Load and type-check a file and its local dependency graph without evaluating.
pub fn check_path(path: impl AsRef<std::path::Path>) -> Result<modules::LoadedProgram, PathError> {
    let loaded = modules::load(path)?;
    semantic::prepare_modules(
        &loaded.program,
        semantic::Mode::Script,
        semantic::Policy::TypedOnly,
        &loaded.module_inputs,
    )
    .map_err(|error| PathError::semantic(&loaded, error))?;
    Ok(loaded)
}

/// Check the complete graph and return the root module's explicit public interface.
pub fn interface_path(
    path: impl AsRef<std::path::Path>,
) -> Result<semantic::ModuleInterface, PathError> {
    let loaded = modules::load(path)?;
    let interfaces = semantic::check_module_interfaces(&loaded.program, &loaded.module_inputs)
        .map_err(|error| PathError::semantic(&loaded, error))?;
    interfaces
        .into_iter()
        .find(|interface| interface.path == loaded.root)
        .ok_or_else(|| PathError::semantic(&loaded, Error::new(0, "missing root module interface")))
}

/// Load, check and execute one file using typed HIR.
pub fn run_path(path: impl AsRef<std::path::Path>) -> Result<eval::Output, PathError> {
    let loaded = modules::load(path)?;
    semantic::prepare_modules(
        &loaded.program,
        semantic::Mode::Script,
        semantic::Policy::TypedOnly,
        &loaded.module_inputs,
    )
    .and_then(|prepared| prepared.run())
    .map_err(|error| PathError::semantic(&loaded, error))
}

/// Parse a file and its local imports, then format only the original root text.
pub fn format_path(path: impl AsRef<std::path::Path>) -> Result<String, PathError> {
    let loaded = modules::load(path)?;
    format_tokens(loaded.root_source()).map_err(|error| PathError::semantic(&loaded, error))
}

/// Conservative early formatter: normalize line endings and remove trailing whitespace.
/// Does not reorder operators, reindent code, or modify comments' textual content.
pub fn format_source(source: &str) -> Result<String, Error> {
    parser::parse(source)?;
    format_tokens(source)
}

fn format_tokens(source: &str) -> Result<String, Error> {
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
    fn mapped_diagnostic_preserves_cross_file_notes_and_byte_ranges() {
        use crate::diagnostic::{Category, Diagnostic, Related, Span};

        let mut detail = Diagnostic::new(Category::Type, Span::new(103, 106), "wrong type");
        detail.related.push(Related {
            label: "declared here".into(),
            span: Span::at(203),
        });
        let display = detail.into_error().diagnostic_with(|offset| match offset {
            100..=106 => Some(("app.flx", "x;\néa;", offset - 100)),
            200..=205 => Some(("lib.flx", "x;\né;", offset - 200)),
            _ => None,
        });
        assert!(display.starts_with("app.flx:2:1: E0200:"), "{display}");
        assert!(display.contains("\n  éa;\n  ^^"), "{display}");
        assert!(
            display.ends_with("note: lib.flx:2:1: declared here"),
            "{display}"
        );
    }

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
