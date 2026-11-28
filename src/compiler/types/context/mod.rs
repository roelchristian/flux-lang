//! Type substitutions and unification context.
mod constraints;
mod rows;
mod schemes;

use super::{Constraint, Row, Type};
use schemes::{free, map_type};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default)]
pub struct InferCtx {
    next: u32,
    types: BTreeMap<u32, Type>,
    type_ids: BTreeSet<u32>,
    row_ids: BTreeSet<u32>,
    rigid: BTreeSet<u32>,
    rows: BTreeMap<u32, Row>,
    lacks: BTreeMap<u32, BTreeSet<String>>,
    pending: Vec<Constraint>,
}

type R<T> = Result<T, String>;

impl InferCtx {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn fresh(&mut self) -> Type {
        let id = self.fresh_id();
        self.type_ids.insert(id);
        Type::Var(id)
    }

    /// Abstract variable from an explicitly quantified signature.
    pub fn fresh_rigid(&mut self) -> Type {
        let Type::Var(id) = self.fresh() else {
            unreachable!()
        };
        self.mark_rigid(id);
        Type::Var(id)
    }

    pub fn mark_rigid(&mut self, id: u32) {
        self.next = self.next.max(
            id.checked_add(1)
                .expect("type variable identifier space exhausted"),
        );
        self.rigid.insert(id);
    }

    pub fn unmark_rigid(&mut self, id: u32) {
        self.rigid.remove(&id);
    }

    pub fn fresh_row(&mut self) -> u32 {
        let id = self.fresh_id();
        self.row_ids.insert(id);
        id
    }

    fn fresh_id(&mut self) -> u32 {
        let id = self.next;
        self.next = self
            .next
            .checked_add(1)
            .expect("type variable identifier space exhausted");
        id
    }

    pub fn resolve(&self, ty: &Type) -> Type {
        map_type(
            ty,
            &|v| self.types.get(&v).map_or(Type::Var(v), |t| self.resolve(t)),
            &|row| Type::Record(self.resolve_row(row)),
        )
    }

    fn resolve_row(&self, row: &Row) -> Row {
        let mut fields = row
            .fields
            .iter()
            .map(|(k, t)| (k.clone(), self.resolve(t)))
            .collect::<Vec<_>>();
        let mut tail = row.tail;
        while let Some(next) = tail.and_then(|id| self.rows.get(&id)) {
            fields.extend(
                next.fields
                    .iter()
                    .map(|(k, t)| (k.clone(), self.resolve(t))),
            );
            tail = next.tail;
        }
        Row { fields, tail }
    }

    pub fn unify(&mut self, a: &Type, b: &Type) -> R<()> {
        let before = self.clone();
        let result = self.unify_committing(a, b);
        if result.is_err() {
            *self = before;
        }
        result
    }

    /// Fast path when the caller abandons inference entirely on failure.
    /// A failed call may leave partial bindings; alternative branches must use `unify`.
    pub(crate) fn unify_committing(&mut self, a: &Type, b: &Type) -> R<()> {
        self.register(a)?;
        self.register(b)?;
        self.unify_inner(a, b)?;
        self.check_pending()
    }

    fn register_kind(&mut self, id: u32, row: bool) -> R<()> {
        let (same, other) = if row {
            (&mut self.row_ids, &self.type_ids)
        } else {
            (&mut self.type_ids, &self.row_ids)
        };
        if other.contains(&id) {
            return Err(format!("variable {id} cannot have both type and row kind"));
        }
        self.next = self.next.max(
            id.checked_add(1)
                .ok_or("type variable identifier space exhausted")?,
        );
        same.insert(id);
        Ok(())
    }

    pub(crate) fn register(&mut self, t: &Type) -> R<()> {
        t.try_visit(|t| match t {
            Type::Var(id) => self.register_kind(*id, false),
            Type::Record(row) => {
                if let Some(id) = row.tail {
                    self.register_kind(id, true)?;
                }
                self.register_row(&self.resolve_row(row))
            }
            _ => Ok(()),
        })
    }

    fn unify_inner(&mut self, a: &Type, b: &Type) -> R<()> {
        let a = self.resolve(a);
        let b = self.resolve(b);
        use Type::*;
        match (&a, &b) {
            (Var(x), Var(y)) if x == y => Ok(()),
            (Var(x), Var(y)) if self.rigid.contains(x) && !self.rigid.contains(y) => {
                self.types.insert(*y, a.clone());
                Ok(())
            }
            (Var(v), t) | (t, Var(v)) => {
                if self.rigid.contains(v) {
                    return Err(format!("annotated type variable ~t{v} must remain distinct and general; cannot unify with {t}"));
                }
                let mut vars = BTreeSet::new();
                free(t, &mut vars);
                if vars.contains(v) {
                    return Err(format!("occurs check failed: ~t{v} occurs in {t}"));
                }
                self.types.insert(*v, t.clone());
                Ok(())
            }
            (Seq(x), Seq(y))
            | (Optional(x), Optional(y))
            | (Action(x), Action(y))
            | (Table(x), Table(y)) => self.unify_inner(x, y),
            (Dict(a, b), Dict(c, d)) | (Fun(a, b), Fun(c, d)) | (Result(a, b), Result(c, d)) => {
                self.unify_inner(a, c)?;
                self.unify_inner(b, d)
            }
            (Tuple(xs), Tuple(ys)) if xs.len() == ys.len() => {
                for (x, y) in xs.iter().zip(ys) {
                    self.unify_inner(x, y)?;
                }
                Ok(())
            }
            (Applied(x, xs), Applied(y, ys)) if x == y && xs.len() == ys.len() => {
                for (x, y) in xs.iter().zip(ys) {
                    self.unify_inner(x, y)?;
                }
                Ok(())
            }
            (Record(x), Record(y)) => self.unify_rows(x, y),
            (Nominal(x), Nominal(y)) | (Applied(x, _), Applied(y, _))
                if x != y && crate::visible_symbol(&x.name) == crate::visible_symbol(&y.name) =>
            {
                let left = crate::semantic::public_text(&x.name);
                let right = crate::semantic::public_text(&y.name);
                if left == right {
                    Err(format!(
                        "cannot unify distinct nominal declarations of {left}"
                    ))
                } else {
                    Err(format!(
                        "cannot unify distinct nominal types {left} and {right}"
                    ))
                }
            }
            _ if a == b => Ok(()),
            _ => Err(format!("cannot unify {a} with {b}")),
        }
    }
}
