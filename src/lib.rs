//! Reference prototype for the Flux language.
pub mod check;
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
