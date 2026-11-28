//! Numeric, equality, ordering, and hash obligation checks.
use super::super::{Class, Constraint, Row, Type};
use super::InferCtx;
use super::R;

impl InferCtx {
    pub fn require(&mut self, class: Class, ty: &Type) -> R<()> {
        let mut leaf = Vec::new();
        Self::class_obligations(class, &self.resolve(ty), &mut leaf)?;
        self.register(ty)?;
        self.pending.extend(leaf);
        self.pending.sort();
        self.pending.dedup();
        Ok(())
    }

    pub fn constraints(&self) -> Vec<Constraint> {
        // Unification re-normalizes all pending obligations before committing.
        self.pending.clone()
    }

    pub(super) fn check_pending(&mut self) -> R<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let mut keep = Vec::new();
        for c in &self.pending {
            Self::class_obligations(c.class, &self.resolve(&c.ty), &mut keep)?;
        }
        keep.sort();
        keep.dedup();
        self.pending = keep;
        Ok(())
    }

    fn class_obligations(class: Class, ty: &Type, out: &mut Vec<Constraint>) -> R<()> {
        use Class::*;
        use Type::*;
        let fail = || Err(format!("{ty} does not satisfy {class}"));
        match ty {
            Var(_) => out.push(Constraint {
                class,
                ty: ty.clone(),
            }),
            // Complex values have equality, but not ordering, hashing, or
            // the real/exact Num contract that statistics and Mathesis use.
            Prim(crate::types::Prim::Cx) if class == Eq => {}
            Prim(crate::types::Prim::Cx) => return fail(),
            Prim(p) if class == Ord => {
                if !matches!(
                    p,
                    crate::types::Prim::N
                        | crate::types::Prim::Z
                        | crate::types::Prim::Q
                        | crate::types::Prim::D
                        | crate::types::Prim::R
                        | crate::types::Prim::S
                        | crate::types::Prim::C
                ) {
                    return fail();
                }
            }
            Prim(p)
                if class != Num
                    || matches!(
                        p,
                        crate::types::Prim::N
                            | crate::types::Prim::Z
                            | crate::types::Prim::Q
                            | crate::types::Prim::D
                            | crate::types::Prim::R
                    ) => {}
            Prim(_) => return fail(),
            _ if matches!(class, Num | Ord) => return fail(),
            Optional(x) => Self::class_obligations(class, x, out)?,
            Result(ok, err) if class == Eq => {
                Self::class_obligations(class, ok, out)?;
                Self::class_obligations(class, err, out)?;
            }
            Tuple(xs) => {
                for x in xs {
                    Self::class_obligations(class, x, out)?;
                }
            }
            t if class == Eq && t.array_element().is_some() => {
                Self::class_obligations(class, t.array_element().unwrap(), out)?;
            }
            Seq(x) if matches!(class, Eq | Ord) => Self::class_obligations(class, x, out)?,
            Table(x) if class == Eq => Self::class_obligations(class, x, out)?,
            Dict(k, v) if class == Eq => {
                Self::class_obligations(class, k, out)?;
                Self::class_obligations(class, v, out)?;
            }
            Record(row) if class == Eq => {
                for (_, t) in &row.fields {
                    Self::class_obligations(class, t, out)?;
                }
                if let Some(id) = row.tail {
                    out.push(Constraint {
                        class,
                        ty: Record(Row {
                            fields: vec![],
                            tail: Some(id),
                        }),
                    });
                }
            }
            // Nominal fields are supplied by the inference layer's schema environment.
            Nominal(_) if class == Eq => {}
            _ => return fail(),
        }
        Ok(())
    }
}
