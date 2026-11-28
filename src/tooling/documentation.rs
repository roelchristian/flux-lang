use crate::{lexer, Error};

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
mod tests {
    use super::*;
    use crate::format_source;

    #[test]
    fn preserves_guard_alignment_and_documents_only_standalone_lines() {
        let source = "--- absolute value\nabs(x:z):z := ?{\n    < x 0 : -: x;\n    _     : x;\n}; -- ordinary\n";
        assert_eq!(
            format_source(source).unwrap(),
            "--- absolute value\nabs(x: z): z := ?{\n    < x 0 : -: x;\n    _     : x;\n}; -- ordinary\n"
        );
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
