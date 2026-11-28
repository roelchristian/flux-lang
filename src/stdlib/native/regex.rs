//! Bounded regex compilation and first-match extraction.
use super::super::{error, invariant, succeeded, HirProgram, Span, R, V};
use super::{optional, string, typed_failure, TEXT_LIMIT};
use ::regex::RegexBuilder;

const REGEX_PATTERN_LIMIT: usize = 64 * 1024;
const REGEX_SIZE_LIMIT: usize = 1024 * 1024;

fn regex_find(pattern: &str, input: &str, program: &HirProgram, span: Span) -> R<V> {
    let bad_pattern = |detail| typed_failure(program, "rxerr", "badpattern", pattern, detail, span);
    if pattern.len() > REGEX_PATTERN_LIMIT {
        return bad_pattern("regular expression exceeds the pattern byte limit".into());
    }
    if input.len() > TEXT_LIMIT {
        return Err(error(
            span,
            "regular expression input exceeds the text limit",
        ));
    }
    let regex = match RegexBuilder::new(pattern)
        .size_limit(REGEX_SIZE_LIMIT)
        .dfa_size_limit(REGEX_SIZE_LIMIT)
        .nest_limit(128)
        .build()
    {
        Ok(re) => re,
        Err(failure) => return bad_pattern(failure.to_string()),
    };
    Ok(succeeded(optional(
        regex.find(input).map(|m| string(m.as_str())),
    )))
}

pub(super) fn apply(args: &[V], program: &HirProgram, span: Span) -> R<V> {
    match args {
        [V::String(pattern), V::String(input)] => regex_find(pattern, input, program, span),
        _ => Err(invariant(span, "invalid standard-library operands")),
    }
}
