//! Projection, row lookup, and checked collection element types.
use super::{s, Engine, R};
use crate::types::{Row, Type};

impl Engine<'_> {
    pub(super) fn select(&mut self, sel: &str, src: &Type) -> R<Type> {
        let src = self.cx.resolve(src);
        match src {
            Type::Seq(t) | Type::Table(t) => Ok(s(self.select(sel, &t)?)),
            Type::Tuple(ts) => {
                if let Ok(i) = sel.parse::<usize>() {
                    if let Some(t) = i.checked_sub(1).and_then(|i| ts.get(i)) {
                        return Ok(t.clone());
                    }
                }
                Err(self.err(format!("tuple has no position .{sel}")))
            }
            Type::Nominal(n) => self
                .schemas
                .get(&n)
                .and_then(|r| r.fields.iter().find(|(k, _)| k == sel))
                .map(|(_, t)| t.clone())
                .ok_or_else(|| self.err(format!("schema {} has no field {sel}", n.name))),
            Type::Record(r) => {
                if let Some((_, t)) = r.fields.iter().find(|(n, _)| n == sel) {
                    return Ok(t.clone());
                }
                if r.tail.is_none() {
                    return Err(self.err(format!("record has no field {sel}")));
                }
                let out = self.cx.fresh();
                let row = Type::Record(Row {
                    fields: vec![(sel.into(), out.clone())],
                    tail: Some(self.cx.fresh_row()),
                });
                self.unify(&Type::Record(r), &row)?;
                Ok(out)
            }
            Type::Var(_) => {
                if sel.as_bytes().first().is_some_and(u8::is_ascii_digit) {
                    return Err(self.unsupported("tuple projection needs a concrete tuple arity"));
                }
                let out = self.cx.fresh();
                let row = Type::Record(Row {
                    fields: vec![(sel.into(), out.clone())],
                    tail: Some(self.cx.fresh_row()),
                });
                self.unify(&src, &row)?;
                Ok(out)
            }
            _ => Err(self.err(format!("cannot select .{sel} from {src}"))),
        }
    }

    pub(super) fn collection(&mut self, t: &Type) -> R<Type> {
        match self.cx.resolve(t) {
            Type::Seq(x) | Type::Table(x) => Ok(*x),
            t @ Type::Var(_) => {
                let out = self.cx.fresh();
                self.unify(&t, &s(out.clone()))?;
                Ok(out)
            }
            t => Err(self.err(format!("expected a sequence or table, found {t}"))),
        }
    }

    pub(super) fn seq_element(&mut self, t: &Type) -> R<Type> {
        let out = self.cx.fresh();
        self.unify(t, &s(out.clone()))?;
        Ok(out)
    }

    pub(super) fn sequence_or_dense_element(&mut self, ty: &Type) -> R<(Type, bool)> {
        if let Some(element) = self.cx.resolve(ty).array_element() {
            return Ok((element.clone(), true));
        }
        Ok((self.seq_element(ty)?, false))
    }

    pub(super) fn row(&self, t: &Type) -> R<Row> {
        match self.cx.resolve(t) {
            Type::Record(r) => Ok(r),
            Type::Nominal(n) => self
                .schemas
                .get(&n)
                .cloned()
                .ok_or_else(|| self.err(format!("unknown schema {}", n.name))),
            x => Err(self.err(format!("expected a record, found {x}"))),
        }
    }
}
