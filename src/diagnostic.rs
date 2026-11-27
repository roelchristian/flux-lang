//! Structured semantic diagnostics shared by the Draft 0.4 frontend.

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}
impl Span {
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
    pub const fn at(start: usize) -> Self {
        Self::new(start, start)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Category {
    Resolution,
    Type,
    InvalidHir,
    MigrationBoundary,
}
impl Category {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Resolution => "E0100",
            Self::Type => "E0200",
            Self::InvalidHir => "E0300",
            Self::MigrationBoundary => "E0400",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Related {
    pub span: Span,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    pub category: Category,
    pub primary: Span,
    pub related: Vec<Related>,
    pub message: String,
    pub expected: Option<String>,
    pub actual: Option<String>,
    pub constraint: Option<String>,
    pub suggestion: Option<String>,
}
impl Diagnostic {
    pub fn new(category: Category, primary: Span, message: impl Into<String>) -> Self {
        Self {
            category,
            primary,
            related: Vec::new(),
            message: message.into(),
            expected: None,
            actual: None,
            constraint: None,
            suggestion: None,
        }
    }
    pub fn into_error(self) -> crate::Error {
        crate::Error::from_diagnostic(self)
    }
}
