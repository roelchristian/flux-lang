//! Row-kind constraints, duplicate labels, and row-tail unification.
use super::super::{Row, Type};
use super::InferCtx;
use super::{free, R};
use std::collections::BTreeSet;

impl InferCtx {
    fn add_lacks(&mut self, id: u32, labels: BTreeSet<String>) -> R<()> {
        self.register_kind(id, true)?;
        let known = self.lacks.entry(id).or_default();
        if self.rigid.contains(&id) {
            if let Some(label) = labels.difference(known).next() {
                return Err(format!(
                    "rigid row tail ~r{id} is not known to lack field {label}"
                ));
            }
        } else {
            known.extend(labels);
        }
        Ok(())
    }

    pub(super) fn register_row(&mut self, row: &Row) -> R<()> {
        let mut names = BTreeSet::new();
        for (n, _) in &row.fields {
            if !names.insert(n.clone()) {
                return Err(format!("duplicate row field {n}"));
            }
        }
        if let Some(id) = row.tail {
            self.add_lacks(id, names)?;
        }
        Ok(())
    }

    fn bind_row(&mut self, id: u32, row: Row) -> R<()> {
        let row = self.resolve_row(&row);
        if row.tail == Some(id) && row.fields.is_empty() {
            return Ok(());
        }
        if self.rigid.contains(&id) {
            return Err(format!("annotated row variable ~r{id} must remain distinct and general; cannot unify with {row}"));
        }
        let mut vars = BTreeSet::new();
        free(&Type::Record(row.clone()), &mut vars);
        if vars.contains(&id) {
            return Err(format!("row occurs check failed: ~r{id} occurs in {row}"));
        }
        let banned = self.lacks.get(&id).cloned().unwrap_or_default();
        if let Some((n, _)) = row.fields.iter().find(|(n, _)| banned.contains(n)) {
            return Err(format!("duplicate row field {n}"));
        }
        self.register_row(&row)?;
        if let Some(t) = row.tail {
            self.add_lacks(t, banned)?;
        }
        self.rows.insert(id, row);
        Ok(())
    }

    pub(super) fn unify_rows(&mut self, a: &Row, b: &Row) -> R<()> {
        self.register_row(a)?;
        self.register_row(b)?;
        let mut only_a = vec![];
        let mut only_b = vec![];
        for (n, ty) in &a.fields {
            if let Some((_, other)) = b.fields.iter().find(|(k, _)| k == n) {
                self.unify_inner(ty, other)
                    .map_err(|e| format!("field {n}: {e}"))?;
            } else {
                only_a.push((n.clone(), ty.clone()));
            }
        }
        for (n, ty) in &b.fields {
            if !a.fields.iter().any(|(k, _)| k == n) {
                only_b.push((n.clone(), ty.clone()));
            }
        }
        let absent = |fields: &[(String, Type)]| {
            format!(
                "missing row field {}",
                fields
                    .iter()
                    .map(|(n, _)| n.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        match (a.tail, b.tail) {
            (_, None) if !only_a.is_empty() => Err(absent(&only_a)),
            (None, _) if !only_b.is_empty() => Err(absent(&only_b)),
            (None, None) => Ok(()),
            (Some(x), None) => self.bind_row(
                x,
                Row {
                    fields: only_b,
                    tail: None,
                },
            ),
            (None, Some(y)) => self.bind_row(
                y,
                Row {
                    fields: only_a,
                    tail: None,
                },
            ),
            (Some(x), Some(y)) if x == y => {
                if only_a.is_empty() && only_b.is_empty() {
                    Ok(())
                } else {
                    Err(format!(
                        "shared row tail ~r{x} cannot absorb different fields"
                    ))
                }
            }
            (Some(x), Some(y))
                if self.rigid.contains(&x) && !self.rigid.contains(&y) && only_b.is_empty() =>
            {
                self.bind_row(
                    y,
                    Row {
                        fields: only_a,
                        tail: Some(x),
                    },
                )
            }
            (Some(x), Some(y))
                if self.rigid.contains(&y) && !self.rigid.contains(&x) && only_a.is_empty() =>
            {
                self.bind_row(
                    x,
                    Row {
                        fields: only_b,
                        tail: Some(y),
                    },
                )
            }
            (Some(x), Some(y)) if only_a.is_empty() && only_b.is_empty() => self.bind_row(
                x,
                Row {
                    fields: vec![],
                    tail: Some(y),
                },
            ),
            (Some(x), Some(y)) => {
                let tail = Some(self.fresh_row());
                self.bind_row(
                    x,
                    Row {
                        fields: only_b,
                        tail,
                    },
                )?;
                self.bind_row(
                    y,
                    Row {
                        fields: only_a,
                        tail,
                    },
                )
            }
        }
    }
}
