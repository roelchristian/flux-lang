//! Constraint enforcement, nominal capability evidence, and inference diagnostics.
use super::{Engine, InferError, InferErrorKind, R};
use crate::types::{Class, NominalId, Type};
use std::collections::HashSet;

fn type_has_nominal(t: &Type) -> bool {
    t.find_map(|node| matches!(node, Type::Nominal(_) | Type::Applied(_, _)).then_some(()))
        .is_some()
}

impl Engine<'_> {
    pub(super) fn err(&self, text: impl Into<String>) -> InferError {
        self.make_error(InferErrorKind::TypeError, text)
    }

    pub(super) fn unsupported(&self, text: impl Into<String>) -> InferError {
        self.make_error(InferErrorKind::Unsupported, text)
    }

    fn make_error(&self, kind: InferErrorKind, text: impl Into<String>) -> InferError {
        InferError {
            kind,
            start: self.start,
            end: self
                .active
                .as_ref()
                .filter(|s| s.start == self.start)
                .map_or(self.start, |s| s.end),
            expected: None,
            actual: None,
            constraint: None,
            message: text.into(),
        }
    }

    fn error_conflict(&self, actual: &Type, expected: &Type) -> Option<(Type, Type)> {
        match (actual, expected) {
            (Type::Action(x), Type::Action(y)) => self.error_conflict(x, y),
            (Type::Result(_, a), Type::Result(_, b))
                if a != b && self.cx.free_vars(a).is_empty() && self.cx.free_vars(b).is_empty() =>
            {
                Some((a.as_ref().clone(), b.as_ref().clone()))
            }
            _ => None,
        }
    }

    pub(super) fn incompatible(&self, actual: &Type, expected: &Type) -> InferError {
        let mut e = self.err(format!(
            "incompatible error type: expected {expected}, found {actual}"
        ));
        e.kind = InferErrorKind::IncompatibleError;
        e.expected = Some(expected.to_string());
        e.actual = Some(actual.to_string());
        e
    }

    pub(super) fn unify(&mut self, x: &Type, y: &Type) -> R<()> {
        // The generic unifier cannot resolve nominal schema names. Check any
        // pending class obligations that are about to take on a nominal type.
        let left = self.cx.resolve(x);
        let right = self.cx.resolve(y);
        let action_result = matches!((&left, &right), (Type::Action(a), Type::Action(b))
            if matches!(a.as_ref(), Type::Result(..))
                && !matches!(b.as_ref(), Type::Result(..) | Type::Var(_) | Type::Never));
        if action_result
            || (matches!(left, Type::Result(..))
                && !matches!(right, Type::Result(..) | Type::Var(_) | Type::Never))
        {
            let mut e = self.err(format!(
                "a fallible result cannot be used as {right} without handling its error"
            ));
            e.kind = InferErrorKind::ResultNeedsHandling;
            e.expected = Some(right.to_string());
            e.actual = Some(left.to_string());
            return Err(e);
        }
        let has_nominal = type_has_nominal(&left) || type_has_nominal(&right);
        let error_conflict = if self.saw_result {
            self.error_conflict(&left, &right)
        } else {
            None
        };
        let pending = if has_nominal {
            self.cx.constraints()
        } else {
            Vec::new()
        };
        self.cx
            .unify_committing(x, y)
            .map_err(|e| match error_conflict {
                Some((actual, expected)) => self.incompatible(&actual, &expected),
                None => {
                    let mut failure = self.err(e);
                    if left != right {
                        failure.actual = Some(left.to_string());
                        failure.expected = Some(right.to_string());
                    }
                    failure
                }
            })?;
        for c in pending {
            self.nominal_evidence(c.class, &c.ty, &mut HashSet::new())?;
        }
        Ok(())
    }

    pub(super) fn req(&mut self, c: Class, t: &Type) -> R<()> {
        self.nominal_evidence(c, t, &mut HashSet::new())?;
        self.cx.require(c, t).map_err(|e| {
            let mut failure = self.err(e);
            failure.actual = Some(self.cx.resolve(t).to_string());
            failure.constraint = Some(format!("{c} {}", self.cx.resolve(t)));
            failure
        })
    }

    fn nominal_evidence(&mut self, c: Class, t: &Type, seen: &mut HashSet<NominalId>) -> R<()> {
        if !matches!(c, Class::Eq | Class::Hash) {
            return Ok(());
        }
        match self.cx.resolve(t) {
            Type::Nominal(n) if seen.insert(n.clone()) => {
                // Hash remains intentionally unavailable for nominal records.
                if c == Class::Hash {
                    return Err(
                        self.err(format!("nominal record {} does not satisfy hash", n.name))
                    );
                }
                if let Some(ids) = self.sums.get(&n).cloned() {
                    for id in ids {
                        let payload = self.result.constructors[&id].payload.clone();
                        for t in payload {
                            self.nominal_evidence(c, &t, seen)?;
                            self.cx.require(c, &t).map_err(|text| {
                                self.err(format!(
                                    "constructor {} payload: {text}",
                                    self.resolver.symbol(id).name
                                ))
                            })?;
                        }
                    }
                } else {
                    let row = self
                        .schemas
                        .get(&n)
                        .cloned()
                        .ok_or_else(|| self.err(format!("unknown schema {}", n.name)))?;
                    for (field, t) in row.fields {
                        self.nominal_evidence(c, &t, seen)?;
                        self.cx.require(c, &t).map_err(|text| {
                            self.err(format!("field {}.{field}: {text}", n.name))
                        })?;
                    }
                }
            }
            ref t if t.array_element().is_some() => {
                self.nominal_evidence(c, t.array_element().unwrap(), seen)?
            }
            Type::Seq(x) | Type::Table(x) | Type::Optional(x) => {
                self.nominal_evidence(c, &x, seen)?
            }
            Type::Dict(k, v) | Type::Result(k, v) => {
                self.nominal_evidence(c, &k, seen)?;
                self.nominal_evidence(c, &v, seen)?;
            }
            Type::Tuple(ts) => {
                for t in ts {
                    self.nominal_evidence(c, &t, seen)?;
                }
            }
            Type::Record(r) => {
                for (_, t) in r.fields {
                    self.nominal_evidence(c, &t, seen)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
