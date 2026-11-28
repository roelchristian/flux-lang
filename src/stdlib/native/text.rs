//! Bounded Unicode text primitives.
use super::super::{as_seq, error, invariant, resources, CoreOp, Span, R, V};
use super::{sequence, string, TEXT_LIMIT};

fn text_case(s: &str, upper: bool, span: Span) -> R<V> {
    if s.len() > TEXT_LIMIT {
        return Err(error(span, "case conversion input exceeds the text limit"));
    }
    let value = if upper {
        s.to_uppercase()
    } else {
        s.to_lowercase()
    };
    if value.len() > TEXT_LIMIT {
        return Err(error(span, "case conversion output exceeds the text limit"));
    }
    Ok(string(value))
}

fn text_split(separator: &str, input: &str, span: Span) -> R<V> {
    if input.len() > TEXT_LIMIT {
        return Err(error(span, "split input exceeds the text limit"));
    }
    let mut parts = Vec::new();
    for item in input.split(separator) {
        if parts.len() >= resources::limits().max_elements {
            return Err(error(span, "split result exceeds the collection limit"));
        }
        parts.push(string(item));
    }
    Ok(sequence(parts))
}

fn text_join(separator: &str, input: &V, span: Span) -> R<V> {
    let seq = as_seq(input).map_err(|e| invariant(span, e))?;
    let mut remaining = TEXT_LIMIT
        .checked_sub(separator.len().saturating_mul(seq.len().saturating_sub(1)))
        .ok_or_else(|| error(span, "join result exceeds the text limit"))?;
    let mut parts = Vec::with_capacity(seq.len());
    for value in seq {
        let V::String(part) = value else {
            return Err(invariant(span, "str.join requires a sequence of strings"));
        };
        remaining = remaining
            .checked_sub(part.len())
            .ok_or_else(|| error(span, "join result exceeds the text limit"))?;
        parts.push(part.as_ref());
    }
    Ok(string(parts.join(separator)))
}

pub(super) fn apply(op: CoreOp, args: &[V], span: Span) -> R<V> {
    use CoreOp::*;
    match (op, args) {
        (StrUpper, [V::String(s)]) => text_case(s, true, span),
        (StrLower, [V::String(s)]) => text_case(s, false, span),
        (StrTrim, [V::String(s)]) => Ok(string(s.trim())),
        (StrSplit, [V::String(delimiter), V::String(s)]) => text_split(delimiter, s, span),
        (StrJoin, [V::String(delimiter), seq]) => text_join(delimiter, seq, span),
        _ => Err(invariant(span, "invalid standard-library operands")),
    }
}
