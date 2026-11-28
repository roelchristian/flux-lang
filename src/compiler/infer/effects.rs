//! Fallible boundaries, recovery typing, and error defaulting.
use super::{mono, p, result, Engine, Env, R};
use crate::parser::{Expr, RecoveryCase, RecoveryPattern, Stmt};
use crate::types::{Prim, Type};
use std::collections::HashSet;

pub(super) fn immediate_raise(e: &Expr) -> bool {
    use Expr::*;
    match e {
        Raise(_) => true,
        Call(_, xs) | Tuple(xs) | Array(xs) | Pipeline(xs) => xs.iter().any(immediate_raise),
        Record(fs) | NamedRecord(_, fs) => fs.iter().any(|(_, x)| immediate_raise(x)),
        Block(ss, tail) => {
            ss.iter().any(|s| match s {
                Stmt::Def { params, body, .. } if params.is_empty() => immediate_raise(body),
                Stmt::Effect { body, .. } => immediate_raise(body),
                _ => false,
            }) || tail.as_deref().is_some_and(immediate_raise)
        }
        Cases(cs, x) => {
            cs.iter()
                .any(|(c, b)| immediate_raise(c) || immediate_raise(b))
                || immediate_raise(x)
        }
        Recover(x, cs) => immediate_raise(x) || cs.iter().any(|c| immediate_raise(&c.body)),
        Propagate(x) | AutoPropagate(x) => immediate_raise(x),
        _ => false,
    }
}

impl Engine<'_> {
    pub(super) fn at_boundary(&mut self, e: &Expr) -> R<Type> {
        let id_of_site = self.syntax_id(e);
        let Some((id, err, used)) = self.boundaries.last_mut() else {
            return Err(self.err("no lexically enclosing fallible boundary"));
        };
        *used = true;
        self.result.boundaries.insert(id_of_site, *id);
        Ok(err.clone())
    }

    pub(super) fn recover(
        &mut self,
        env: &Env,
        value: &Expr,
        cases: &[RecoveryCase],
        want: Option<&Type>,
        effects: bool,
    ) -> R<Type> {
        let ok = want.cloned().unwrap_or_else(|| self.cx.fresh());
        let err = self.cx.fresh();
        let v = self.expr(env, value, Some(&result(ok.clone(), err.clone())), effects)?;
        if cases.is_empty() {
            return Err(self.err("recovery needs at least one case"));
        }
        let mut seen = HashSet::new();
        let mut catchall = false;
        for case in cases {
            let previous = self.start;
            self.start = case.start;
            if catchall {
                return Err(self.err("unreachable recovery case after catchall"));
            }
            if let Type::Nominal(owner) | Type::Applied(owner, _) = self.cx.resolve(&err) {
                if self
                    .sums
                    .get(&owner)
                    .is_some_and(|ids| !ids.is_empty() && ids.iter().all(|id| seen.contains(id)))
                {
                    return Err(
                        self.err("unreachable recovery case; all constructors are already covered")
                    );
                }
            }
            let mut nested = env.clone();
            if let Some(id) = self.resolver.recovery_constructor(case) {
                if !seen.insert(id) {
                    return Err(self.err("duplicate recovery constructor case"));
                }
                let payload = self.match_constructor(id, &err)?;
                let names = match &case.pattern {
                    RecoveryPattern::Constructor(_, xs) => xs.as_slice(),
                    _ => &[],
                };
                if names.len() != payload.len() {
                    return Err(self.err("wrong recovery constructor arity"));
                }
                let ids = self.resolver.recovery_bindings(case);
                for ((name, ty), id) in names
                    .iter()
                    .zip(payload)
                    .filter(|(n, _)| n.as_str() != "_")
                    .zip(ids)
                {
                    nested.insert(name.clone(), mono(ty.clone()));
                    self.result.bindings.insert(*id, mono(ty));
                }
            } else {
                catchall = true;
                match &case.pattern {
                    RecoveryPattern::Bind(name) => {
                        nested.insert(name.clone(), mono(err.clone()));
                        if let Some(id) = self.resolver.recovery_bindings(case).first() {
                            self.result.bindings.insert(*id, mono(err.clone()));
                        }
                    }
                    RecoveryPattern::Wildcard => {}
                    RecoveryPattern::Constructor(..) => {
                        return Err(self.err("unknown recovery constructor"))
                    }
                }
            }
            self.expr(&nested, &case.body, Some(&ok), effects)?;
            self.start = previous;
        }
        if !catchall {
            let owner = match self.cx.resolve(&err) {
                Type::Nominal(owner) | Type::Applied(owner, _) => owner,
                _ => return Err(self.err("nonexhaustive recovery; add a catchall")),
            };
            let variants = self
                .sums
                .get(&owner)
                .ok_or_else(|| self.err("nonexhaustive recovery; add a catchall"))?;
            let missed = variants
                .iter()
                .filter(|id| !seen.contains(id))
                .map(|id| self.resolver.symbol(*id).name.as_str())
                .collect::<Vec<_>>();
            if !missed.is_empty() {
                return Err(self.err(format!(
                    "nonexhaustive recovery; missing {}",
                    missed.join(", ")
                )));
            }
        }
        Ok(if v == Type::Never { Type::Never } else { ok })
    }

    pub(super) fn default_combined_errors<'a>(
        &mut self,
        from: usize,
        arguments: impl Iterator<Item = &'a Type>,
        visible: &Env,
    ) -> R<()> {
        if self.combined_error_defaults.len() == from {
            return Ok(());
        }
        // Let all contextual constraints in this inference scope settle first.
        // A parameter or a real raise may establish a generic error type.
        let protected = arguments
            .flat_map(|ty| self.cx.free_vars(ty))
            .chain(
                self.raised_error_types
                    .iter()
                    .flat_map(|ty| self.cx.free_vars(ty)),
            )
            .chain(visible.values().flat_map(|scheme| {
                self.cx
                    .free_vars(&scheme.ty)
                    .into_iter()
                    .filter(|id| !scheme.vars.contains(id))
            }))
            .collect::<HashSet<_>>();
        for ty in self.combined_error_defaults.split_off(from) {
            if let Type::Var(id) = self.cx.resolve(&ty) {
                if !self.explicit_variables.contains(&id) && !protected.contains(&id) {
                    self.unify(&ty, &p(Prim::Unit))?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn reset_boundary<T>(&mut self, op: impl FnOnce(&mut Self) -> R<T>) -> R<T> {
        let saved = std::mem::take(&mut self.boundaries);
        let outcome = op(self);
        self.boundaries = saved;
        outcome
    }
}
