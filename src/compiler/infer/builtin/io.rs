//! Type/schema validation for CSV and JSON operations.
use super::super::{a, p, result, s, Engine, R};
use crate::types::{NominalId, Prim, Type};
use std::collections::HashSet;

impl Engine<'_> {
    pub(super) fn csv_read_type(&mut self, want: Option<&Type>) -> R<Type> {
        let ok = match want.map(|t| self.cx.resolve(t)) {
            Some(Type::Action(t)) => match *t {
                Type::Result(t, _) => match *t {
                    Type::Table(row) => {
                        self.csv_row(&row)?;
                        Type::Table(row)
                    }
                    _ => s(s(p(Prim::S))),
                },
                _ => s(s(p(Prim::S))),
            },
            _ => s(s(p(Prim::S))),
        };
        Ok(a(result(ok, Self::standard_type("csverr"))))
    }

    pub(super) fn csv_row(&mut self, t: &Type) -> R<()> {
        let r = self.row(t)?;
        if r.fields.is_empty() || r.tail.is_some() {
            return Err(self.err("CSV requires a closed nonempty row schema"));
        }
        for (name, t) in r.fields {
            let t = self.cx.resolve(&t);
            let t = if let Type::Optional(t) = t { *t } else { t };
            if !matches!(
                t,
                Type::Prim(
                    Prim::N | Prim::Z | Prim::Q | Prim::D | Prim::R | Prim::B | Prim::S | Prim::C
                )
            ) {
                return Err(self.err(format!("unsupported CSV column type for {name}: {t}")));
            }
        }
        Ok(())
    }

    pub(super) fn json_decodable(&self, typ: &Type, seen: &mut HashSet<NominalId>) -> bool {
        use Prim::*;
        match self.cx.resolve(typ) {
            Type::Prim(N | Z | Q | D | B | S | C | Unit) => true,
            Type::Optional(t) => {
                !matches!(t.as_ref(), Type::Optional(_) | Type::Prim(Unit))
                    && self.json_decodable(&t, seen)
            }
            Type::Seq(t) => self.json_decodable(&t, seen),
            Type::Tuple(ts) => ts.iter().all(|t| self.json_decodable(t, seen)),
            Type::Record(r) => {
                r.tail.is_none() && r.fields.iter().all(|(_, t)| self.json_decodable(t, seen))
            }
            Type::Nominal(n) => {
                !seen.insert(n.clone())
                    || self.schemas.get(&n).is_some_and(|r| {
                        r.tail.is_none()
                            && r.fields.iter().all(|(_, t)| self.json_decodable(t, seen))
                    })
            }
            _ => false,
        }
    }
}
