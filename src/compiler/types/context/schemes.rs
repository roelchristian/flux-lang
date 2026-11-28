//! Capture-aware quantification, scheme instantiation, and type substitution.
use super::super::{Constraint, Row, Scheme, Type};
use super::InferCtx;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn free(t: &Type, out: &mut BTreeSet<u32>) {
    t.visit(|t| {
        if let Type::Var(id) | Type::Record(Row { tail: Some(id), .. }) = t {
            out.insert(*id);
        }
    });
}

fn row_labels(t: &Type, out: &mut BTreeMap<u32, BTreeSet<String>>) {
    t.visit(|t| {
        if let Type::Record(Row {
            fields,
            tail: Some(id),
        }) = t
        {
            out.entry(*id)
                .or_default()
                .extend(fields.iter().map(|(name, _)| name.clone()));
        }
    });
}

fn remap(t: &Type, ids: &BTreeMap<u32, u32>) -> Type {
    map_type(
        t,
        &|v| Type::Var(ids.get(&v).copied().unwrap_or(v)),
        &|row| {
            Type::Record(Row {
                fields: row
                    .fields
                    .iter()
                    .map(|(name, t)| (name.clone(), remap(t, ids)))
                    .collect(),
                tail: row.tail.map(|id| ids.get(&id).copied().unwrap_or(id)),
            })
        },
    )
}
pub(super) fn map_type(t: &Type, var: &impl Fn(u32) -> Type, row: &impl Fn(&Row) -> Type) -> Type {
    let inner = |t: &Type| Box::new(map_type(t, var, row));
    match t {
        Type::Var(v) => var(*v),
        Type::Record(r) => row(r),
        Type::Prim(_) | Type::Nominal(_) | Type::Never => t.clone(),
        Type::Seq(x) => Type::Seq(inner(x)),
        Type::Optional(x) => Type::Optional(inner(x)),
        Type::Result(x, y) => Type::Result(inner(x), inner(y)),
        Type::Action(x) => Type::Action(inner(x)),
        Type::Table(x) => Type::Table(inner(x)),
        Type::Dict(a, b) => Type::Dict(inner(a), inner(b)),
        Type::Fun(a, b) => Type::Fun(inner(a), inner(b)),
        Type::Tuple(ts) => Type::Tuple(ts.iter().map(|x| map_type(x, var, row)).collect()),
        Type::Applied(n, ts) => Type::Applied(
            n.clone(),
            ts.iter().map(|x| map_type(x, var, row)).collect(),
        ),
    }
}

impl InferCtx {
    pub fn free_vars(&self, t: &Type) -> BTreeSet<u32> {
        let mut vars = BTreeSet::new();
        free(&self.resolve(t), &mut vars);
        vars
    }

    /// Look for skolems in pre-existing bindings after checking a nested signature.
    /// Quantified variables in an existing scheme are abstract, not outer captures.
    pub fn rigid_escapes(&self, ids: &[u32], outer: &[Scheme]) -> bool {
        outer.iter().any(|s| {
            let vars = self.free_vars(&s.ty);
            let leaked = |v: &u32| !s.vars.contains(v) && ids.contains(v);
            vars.iter().any(leaked)
                || s.constraints
                    .iter()
                    .any(|c| self.free_vars(&c.ty).iter().any(leaked))
        })
    }

    /// Generalize variables free in this type, excluding variables free in the environment.
    /// Constraints belong to a scheme when they refer to one of its quantified variables.
    pub fn generalize(&self, ty: &Type, env: &[Scheme], constraints: &[Constraint]) -> Scheme {
        let ty = self.resolve(ty);
        let mut vars = BTreeSet::new();
        free(&ty, &mut vars);
        for s in env {
            let mut env_vars = BTreeSet::new();
            free(&self.resolve(&s.ty), &mut env_vars);
            for c in &s.constraints {
                free(&self.resolve(&c.ty), &mut env_vars);
            }
            for id in &s.vars {
                env_vars.remove(id);
            }
            vars.retain(|id| !env_vars.contains(id));
        }
        let mut constraints = constraints
            .iter()
            .map(|c| Constraint {
                class: c.class,
                ty: self.resolve(&c.ty),
            })
            .filter(|c| {
                let mut cv = BTreeSet::new();
                free(&c.ty, &mut cv);
                !cv.is_disjoint(&vars)
            })
            .collect::<Vec<_>>();
        constraints.sort();
        constraints.dedup();
        let mut row_lacks = BTreeMap::new();
        row_labels(&ty, &mut row_lacks);
        for c in &constraints {
            row_labels(&c.ty, &mut row_lacks);
        }
        row_lacks.retain(|id, labels| {
            if let Some(known) = self.lacks.get(id) {
                labels.extend(known.iter().cloned());
            }
            vars.contains(id)
        });
        Scheme {
            vars: vars.into_iter().collect(),
            ty,
            constraints,
            row_lacks,
        }
    }

    /// Return a freshly renamed type and constraints for the caller to enforce.
    pub fn instantiate(&mut self, scheme: &Scheme) -> (Type, Vec<Constraint>) {
        let mut vars = BTreeSet::new();
        free(&scheme.ty, &mut vars);
        for c in &scheme.constraints {
            free(&c.ty, &mut vars);
        }
        vars.extend(scheme.vars.iter().copied());
        if let Some(max) = vars.last() {
            self.next = self.next.max(
                max.checked_add(1)
                    .expect("type variable identifier space exhausted"),
            );
        }
        let mut rows = scheme.row_lacks.clone();
        row_labels(&scheme.ty, &mut rows);
        for c in &scheme.constraints {
            row_labels(&c.ty, &mut rows);
        }
        let ids = scheme
            .vars
            .iter()
            .map(|&v| {
                let id = self.fresh_id();
                if let Some(labels) = rows.remove(&v) {
                    self.row_ids.insert(id);
                    self.lacks.insert(id, labels);
                } else {
                    self.type_ids.insert(id);
                }
                (v, id)
            })
            .collect::<BTreeMap<_, _>>();
        let ty = remap(&scheme.ty, &ids);
        let constraints = scheme
            .constraints
            .iter()
            .map(|c| Constraint {
                class: c.class,
                ty: remap(&c.ty, &ids),
            })
            .collect();
        (ty, constraints)
    }
}
