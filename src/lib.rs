//! Reference prototype for the Flux language.
pub mod check;
mod csv;
pub mod eval;
pub mod lexer;
pub mod parser;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    pub start: usize,
    pub message: String,
}
impl Error {
    pub fn new(start: usize, message: impl Into<String>) -> Self {
        Self {
            start,
            message: message.into(),
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
        format!("{path}:{line}:{column}: {}", self.message)
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
    check::check(&program)?;
    Ok(program)
}

pub fn run_source(source: &str) -> Result<eval::Output, Error> {
    eval::run(&check_source(source)?)
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
