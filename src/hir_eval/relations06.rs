//! Typed set, row projection and explicit missing-value transformations.
use super::*;
use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
};

const UNHASHABLE_COMPARISON_LIMIT: usize = 8_000_000;

fn table(value: &V, span: Span) -> R<(&[V], &Rc<TableSchema>)> {
    match value {
        V::Table(rows, schema) => Ok((rows, schema)),
        _ => Err(invariant(span, "expected a checked table")),
    }
}

// Store a small fingerprint per supported row and verify equality on a hash
// collision. We avoid retaining duplicate encodings of wide or nested fields.
fn fingerprint(
    row: &V,
    hash: &mut DefaultHasher,
    remaining: &mut usize,
    depth: usize,
    span: Span,
) -> R<bool> {
    if depth >= 128 {
        return Err(error(span, "table set row nesting limit exceeded"));
    }
    let charge = match row {
        V::Record(fields) => fields
            .iter()
            .map(|(name, _)| name.len())
            .sum::<usize>()
            .saturating_add(fields.len().saturating_mul(std::mem::size_of::<&V>())),
        V::String(value) => value.len(),
        _ => 0,
    }
    .saturating_add(std::mem::size_of::<V>());
    *remaining = remaining
        .checked_sub(charge)
        .ok_or_else(|| error(span, "table set hash work limit exceeded"))?;
    match row {
        V::Record(fields) => {
            1u8.hash(hash);
            fields.len().hash(hash);
            // Record equality ignores field order.
            let mut fields = fields.iter().collect::<Vec<_>>();
            fields.sort_unstable_by(|a, b| a.0.cmp(&b.0));
            for (name, value) in fields {
                name.hash(hash);
                if !fingerprint(value, hash, remaining, depth + 1, span)? {
                    return Ok(false);
                }
            }
        }
        V::Sequence(xs) | V::Tuple(xs) => {
            (if matches!(row, V::Sequence(_)) {
                2u8
            } else {
                3u8
            })
            .hash(hash);
            xs.len().hash(hash);
            for value in xs.iter() {
                if !fingerprint(value, hash, remaining, depth + 1, span)? {
                    return Ok(false);
                }
            }
        }
        V::Optional(Some(x)) => {
            4u8.hash(hash);
            return fingerprint(x, hash, remaining, depth + 1, span);
        }
        V::Exact(_)
        | V::Wide(_)
        | V::Real(_)
        | V::Bool(_)
        | V::String(_)
        | V::Char(_)
        | V::Unit
        | V::Optional(None) => {
            5u8.hash(hash);
            key_with_budget(row, remaining, 0)
                .ok_or_else(|| error(span, "table set hash work limit exceeded"))?
                .hash(hash);
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn stable_set<'a>(rows: impl IntoIterator<Item = &'a V>, span: Span) -> R<Vec<V>> {
    let mut out = Vec::new();
    let mut keys = HashMap::<u64, Vec<&V>>::new();
    // Hash support is structural: two equal rows are both hashable or
    // both unhashable, so neither branch can miss an equal row in the other.
    let mut complex = Vec::<&V>::new();
    let mut comparisons = 0usize;
    let mut hash_budget = 256 * 1024 * 1024;
    for row in rows {
        let mut hash = DefaultHasher::new();
        let bucket = if fingerprint(row, &mut hash, &mut hash_budget, 0, span)? {
            keys.entry(hash.finish()).or_default()
        } else {
            &mut complex
        };
        let mut found = false;
        for old in bucket.iter() {
            comparisons += 1;
            if comparisons > UNHASHABLE_COMPARISON_LIMIT {
                return Err(error(
                    span,
                    "structural row distinct comparison limit exceeded",
                ));
            }
            if equal(old, row) {
                found = true;
                break;
            }
        }
        if !found {
            if out.len() >= COLLECTION_LIMIT {
                return Err(error(span, "table set result exceeds the row limit"));
            }
            bucket.push(row);
            out.push(row.clone());
        }
    }
    Ok(out)
}

fn checked_callback(types: &[Type], span: Span) -> R<(&Type, &Type)> {
    match types {
        [Type::Fun(_, out), Type::Table(row)] => Ok((row.as_ref(), out.as_ref())),
        _ => Err(invariant(span, "checked table callback type is missing")),
    }
}

fn fits(row: &V, schema: &TableSchema) -> bool {
    match (row, schema) {
        (V::Record(fields), TableSchema::Named(names)) => {
            fields.len() == names.len()
                && names
                    .iter()
                    .all(|(column, _)| fields.iter().any(|(name, _)| &**name == column))
        }
        (V::Tuple(fields), TableSchema::Positional(columns)) => fields.len() == columns.len(),
        _ => false,
    }
}

fn same_shape(a: &TableSchema, b: &TableSchema) -> bool {
    match (a, b) {
        (TableSchema::Named(a), TableSchema::Named(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(name, _)| b.iter().any(|(other, _)| name == other))
        }
        (TableSchema::Positional(a), TableSchema::Positional(b)) => a.len() == b.len(),
        _ => false,
    }
}

fn transform(
    select: bool,
    callback: &V,
    input: &V,
    types: &[Type],
    schema: Option<&Rc<TableSchema>>,
    eval: &mut Eval<'_>,
    span: Span,
) -> R<V> {
    let (rows, original) = table(input, span)?;
    let (row_type, output_type) = checked_callback(types, span)?;
    let target = if select {
        layout(schema, span)?
    } else {
        original.clone()
    };
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let value = eval.apply_checked(
            callback.clone(),
            vec![row.clone()],
            Some(vec![row_type.clone()]),
            Some(output_type.clone()),
            span,
        )?;
        if select {
            if !fits(&value, &target) {
                return Err(invariant(
                    span,
                    "df.select result disagrees with its checked schema",
                ));
            }
            out.push(value);
        } else {
            match value {
                V::Optional(Some(_)) => out.push(row.clone()),
                V::Optional(None) => {}
                _ => {
                    return Err(invariant(
                        span,
                        "df.dropna callback did not return an optional",
                    ))
                }
            }
        }
    }
    Ok(wrap(out, Some(target)))
}

fn optional_sequence(items: &V, replacement: Option<&V>, span: Span) -> R<V> {
    let rows = as_seq(items).map_err(|e| invariant(span, e))?;
    let mut out = Vec::with_capacity(rows.len());
    for item in rows {
        match item {
            V::Optional(Some(value)) => out.push(value.as_ref().clone()),
            V::Optional(None) => {
                if let Some(value) = replacement {
                    out.push(value.clone());
                }
            }
            _ => return Err(invariant(span, "expected a checked optional sequence")),
        }
    }
    Ok(wrap(out, None))
}

pub(super) fn apply(
    op: CoreOp,
    args: &[V],
    types: &[Type],
    _ty: &Type,
    schema: Option<&Rc<TableSchema>>,
    eval: &mut Eval<'_>,
    span: Span,
) -> Option<R<V>> {
    use CoreOp::*;
    if !matches!(
        op,
        TableUnion | TableDistinct | TableSelect | DropNa | Fill | PresentOnly
    ) {
        return None;
    }
    Some((|| match (op, args) {
        (TableUnion, [left, right]) => {
            let (left, left_schema) = table(left, span)?;
            let (right, right_schema) = table(right, span)?;
            // The checker establishes type/nominal equality. Anonymous
            // fields may have appeared in different lexical order.
            if !same_shape(left_schema, right_schema) {
                return Err(invariant(
                    span,
                    "df.union received different checked schemas",
                ));
            }
            Ok(wrap(
                stable_set(left.iter().chain(right), span)?,
                Some(left_schema.clone()),
            ))
        }
        (TableDistinct, [input]) => {
            let (rows, original) = table(input, span)?;
            Ok(wrap(stable_set(rows, span)?, Some(original.clone())))
        }
        (TableSelect | DropNa, [callback, input]) => transform(
            op == TableSelect,
            callback,
            input,
            types,
            schema,
            eval,
            span,
        ),
        (Fill, [replacement, input]) => optional_sequence(input, Some(replacement), span),
        (PresentOnly, [input]) => optional_sequence(input, None, span),
        _ => Err(invariant(span, "invalid relational operation operands")),
    })())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_record_wins_despite_field_order_and_numeric_scale() {
        let short = decimal05::decimal_literal("3.0").unwrap();
        let long = decimal05::decimal_literal("3.00").unwrap();
        let row = |amount, reverse| {
            let mut fields = vec![
                ("amount".into(), amount),
                ("other".into(), V::Optional(None)),
            ];
            if reverse {
                fields.reverse();
            }
            V::Record(fields.into())
        };
        let rows = [row(short, false), row(long, true)];
        let dedup = stable_set(&rows, Span::default()).unwrap_or_else(|_| panic!("valid rows"));
        assert_eq!(dedup.len(), 1);
        let V::Record(fields) = &dedup[0] else {
            panic!("expected record");
        };
        assert_eq!(&*fields[0].0, "amount");
        assert_eq!(repr(&fields[0].1), "3.0");
    }

    #[test]
    fn outcomes_without_hash_representation_use_structural_equality() {
        let rows = [
            operations041::success(exact_i128(3, Domain::Integer)),
            operations041::success(exact_i128(3, Domain::Rational)),
            operations041::success(exact_i128(9, Domain::Integer)),
        ];
        let stable = stable_set(&rows, Span::default()).unwrap_or_else(|_| panic!("valid rows"));
        assert_eq!(
            stable.iter().map(repr).collect::<Vec<_>>(),
            ["ok 3", "ok 9"]
        );
    }
}
