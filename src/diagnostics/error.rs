use super::Diagnostic;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    pub start: usize,
    pub message: String,
    specification: Option<Box<Diagnostic>>,
}
impl Error {
    pub fn new(start: usize, message: impl Into<String>) -> Self {
        Self {
            start,
            message: message.into(),
            specification: None,
        }
    }

    pub fn structured(&self) -> Option<&Diagnostic> {
        self.specification.as_deref()
    }

    pub(crate) fn from_diagnostic(detail: Diagnostic) -> Self {
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

#[cfg(test)]
mod tests {
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
}
