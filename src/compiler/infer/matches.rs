//! Closed-constructor pattern coverage and lexical arm inference.
use super::{mono, o, p, Engine, Env, InferError, R};
use crate::parser::Expr;
use crate::resolve::SymbolId;
use crate::types::{Prim, Row, Type};
use std::collections::HashSet;

use crate::parser::{MatchCase, MatchPattern, MatchPatternKind};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Tag {
    Constructor(SymbolId),
    Tuple,
    Record,
    Optional(bool),
    Bool(bool),
    Unit,
}

#[derive(Clone)]
enum Pattern {
    Any,
    Head(Tag, Vec<Pattern>),
}

const COVERAGE_WORK_LIMIT: usize = 200_000;

impl Engine<'_> {
    fn pattern_error(&self, span: &std::ops::Range<usize>, message: &str) -> InferError {
        let mut failure = self.err(message);
        failure.start = span.start;
        failure.end = span.end;
        failure
    }

    fn bind_pattern(&mut self, env: &mut Env, pattern: &MatchPattern, ty: &Type) -> R<()> {
        let start = self.start;
        let active = self.active.replace(pattern.span.clone());
        self.start = pattern.span.start;
        let checked = self.bind_pattern_inner(env, pattern, ty);
        self.start = start;
        self.active = active;
        checked
    }

    fn bind_pattern_inner(&mut self, env: &mut Env, pattern: &MatchPattern, ty: &Type) -> R<()> {
        use MatchPatternKind::*;
        match &pattern.kind {
            Wildcard => {}
            Bind(name) => {
                let id = self
                    .resolver
                    .match_binding(pattern)
                    .ok_or_else(|| self.err("missing match pattern binding"))?;
                let sc = mono(ty.clone());
                env.insert(name.clone(), sc.clone());
                self.result.bindings.insert(id, sc);
            }
            Constructor(_, patterns) => {
                let id = self
                    .resolver
                    .match_constructor(pattern)
                    .ok_or_else(|| self.err("missing match constructor"))?;
                let (output, fields) = self.match_constructor_signature(id)?;
                self.unify(&output, ty)?;
                if patterns.len() != fields.len() {
                    return Err(self.err("wrong match constructor arity"));
                }
                for (pattern, field) in patterns.iter().zip(fields) {
                    self.bind_pattern(env, pattern, &field)?;
                }
            }
            Tuple(patterns) => {
                let fields = patterns.iter().map(|_| self.cx.fresh()).collect::<Vec<_>>();
                self.unify(ty, &Type::Tuple(fields.clone()))?;
                for (pattern, field) in patterns.iter().zip(fields) {
                    self.bind_pattern(env, pattern, &field)?;
                }
            }
            Record(fields) => {
                if matches!(self.cx.resolve(ty), Type::Var(_)) {
                    let row = Row {
                        fields: fields
                            .iter()
                            .map(|(name, _)| (name.clone(), self.cx.fresh()))
                            .collect(),
                        tail: Some(self.cx.fresh_row()),
                    };
                    self.unify(ty, &Type::Record(row))?;
                }
                if !matches!(self.cx.resolve(ty), Type::Record(_) | Type::Nominal(_)) {
                    return Err(self.err("record pattern needs a record"));
                }
                for (field, pattern) in fields {
                    let field_ty = self.select(field, ty)?;
                    self.bind_pattern(env, pattern, &field_ty)?;
                }
            }
            Optional(pattern) => {
                let inner = self.cx.fresh();
                self.unify(ty, &o(inner.clone()))?;
                if let Some(pattern) = pattern {
                    self.bind_pattern(env, pattern, &inner)?;
                }
            }
            Bool(_) => self.unify(ty, &p(Prim::B))?,
            Unit => self.unify(ty, &p(Prim::Unit))?,
        }
        Ok(())
    }

    /// A finite outer constructor family, when known. Recursive payload types
    /// are not enumerated until a pattern actually descends into them.
    fn pattern_family(&mut self, ty: &Type) -> R<Vec<(Tag, Vec<Type>)>> {
        Ok(match self.cx.resolve(ty) {
            Type::Prim(Prim::B) => vec![(Tag::Bool(false), vec![]), (Tag::Bool(true), vec![])],
            Type::Prim(Prim::Unit) => vec![(Tag::Unit, vec![])],
            Type::Optional(inner) => vec![
                (Tag::Optional(false), vec![]),
                (Tag::Optional(true), vec![*inner]),
            ],
            Type::Tuple(fields) => vec![(Tag::Tuple, fields)],
            ty @ (Type::Nominal(_) | Type::Applied(..)) => {
                let variants = self.match_constructors(&ty)?;
                if variants.is_empty() {
                    self.row(&ty)
                        .map(|row| {
                            vec![(
                                Tag::Record,
                                row.fields.into_iter().map(|(_, t)| t).collect(),
                            )]
                        })
                        .unwrap_or_default()
                } else {
                    variants
                        .into_iter()
                        .map(|(id, fields)| (Tag::Constructor(id), fields))
                        .collect()
                }
            }
            Type::Record(row) => vec![(
                Tag::Record,
                row.fields.into_iter().map(|(_, t)| t).collect(),
            )],
            _ => vec![],
        })
    }

    fn coverage_pattern(&mut self, pattern: &MatchPattern, ty: &Type) -> R<Pattern> {
        use MatchPatternKind::*;
        let (tag, children) = match &pattern.kind {
            Wildcard | Bind(_) => return Ok(Pattern::Any),
            Constructor(_, xs) => (
                Tag::Constructor(
                    self.resolver
                        .match_constructor(pattern)
                        .ok_or_else(|| self.err("missing match constructor"))?,
                ),
                xs.iter().map(Some).collect(),
            ),
            Tuple(xs) => (Tag::Tuple, xs.iter().map(Some).collect()),
            Record(fields) => {
                let row = self.row(ty)?;
                (
                    Tag::Record,
                    row.fields
                        .iter()
                        .map(|(key, _)| fields.iter().find(|(k, _)| k == key).map(|(_, p)| p))
                        .collect(),
                )
            }
            Optional(value) => (
                Tag::Optional(value.is_some()),
                value.as_deref().into_iter().map(Some).collect(),
            ),
            Bool(value) => (Tag::Bool(*value), vec![]),
            Unit => (Tag::Unit, vec![]),
        };
        let fields = self
            .pattern_family(ty)?
            .into_iter()
            .find(|(key, _)| *key == tag)
            .map(|(_, fields)| fields)
            .ok_or_else(|| self.err("missing checked match pattern family"))?;
        let children = children
            .into_iter()
            .zip(fields)
            .map(|(pattern, ty)| {
                pattern.map_or(Ok(Pattern::Any), |pattern| {
                    self.coverage_pattern(pattern, &ty)
                })
            })
            .collect::<R<Vec<_>>>()?;
        Ok(Pattern::Head(tag, children))
    }

    /// Is there a value matched by query but not by any row of matrix?
    fn useful_pattern(
        &mut self,
        matrix: &[Vec<Pattern>],
        query: &[Pattern],
        types: &[Type],
        work: &mut usize,
        depth: usize,
    ) -> R<bool> {
        *work = work.saturating_add(matrix.len().saturating_add(query.len()));
        if *work > COVERAGE_WORK_LIMIT || depth > 256 {
            return Err(
                self.err("match coverage is too complex; split the match into smaller expressions")
            );
        }
        if matrix.is_empty() {
            return Ok(true);
        }
        if query.is_empty()
            || matrix
                .iter()
                .any(|row| row.iter().all(|p| matches!(p, Pattern::Any)))
        {
            return Ok(false);
        }
        let specialize = |tag: &Tag, fields: usize| {
            matrix
                .iter()
                .filter_map(|row| match &row[0] {
                    Pattern::Any => {
                        let mut out = vec![Pattern::Any; fields];
                        out.extend_from_slice(&row[1..]);
                        Some(out)
                    }
                    Pattern::Head(head, children) if head == tag => {
                        let mut out = children.clone();
                        out.extend_from_slice(&row[1..]);
                        Some(out)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let family = self.pattern_family(&types[0])?;
        if let Pattern::Head(tag, children) = &query[0] {
            let Some((_, mut fields)) = family.into_iter().find(|(f, _)| f == tag) else {
                return Err(self.err("invalid checked match coverage family"));
            };
            let matrix = specialize(tag, fields.len());
            fields.extend_from_slice(&types[1..]);
            let mut inner = children.clone();
            inner.extend_from_slice(&query[1..]);
            return self.useful_pattern(&matrix, &inner, &fields, work, depth + 1);
        }
        let seen = matrix
            .iter()
            .filter_map(|row| match &row[0] {
                Pattern::Head(tag, _) => Some(tag),
                Pattern::Any => None,
            })
            .collect::<HashSet<_>>();
        if !family.is_empty() && family.iter().all(|(tag, _)| seen.contains(tag)) {
            for (tag, mut fields) in family {
                let matrix = specialize(&tag, fields.len());
                let mut inner = vec![Pattern::Any; fields.len()];
                inner.extend_from_slice(&query[1..]);
                fields.extend_from_slice(&types[1..]);
                if self.useful_pattern(&matrix, &inner, &fields, work, depth + 1)? {
                    return Ok(true);
                }
            }
            Ok(false)
        } else {
            let defaults = matrix
                .iter()
                .filter(|row| matches!(row[0], Pattern::Any))
                .map(|row| row[1..].to_vec())
                .collect::<Vec<_>>();
            self.useful_pattern(&defaults, &query[1..], &types[1..], work, depth + 1)
        }
    }

    pub(super) fn match_expr(
        &mut self,
        env: &Env,
        value: &Expr,
        cases: &[MatchCase],
        want: Option<&Type>,
        effects: bool,
    ) -> R<Type> {
        if cases.is_empty() {
            return Err(self.err("match needs at least one case"));
        }
        let inferred = self.expr(env, value, None, effects)?;
        let subject = if inferred == Type::Never {
            self.cx.fresh()
        } else {
            inferred.clone()
        };
        let output = want.cloned().unwrap_or_else(|| self.cx.fresh());
        let mut returns = false;
        for case in cases {
            let mut nested = env.clone();
            self.bind_pattern(&mut nested, &case.pattern, &subject)?;
            returns |= self.expr(&nested, &case.body, Some(&output), effects)? != Type::Never;
        }
        let mut work = 0;
        let mut matrix = Vec::with_capacity(cases.len());
        let subjects = std::slice::from_ref(&subject);
        for case in cases {
            let query = vec![self.coverage_pattern(&case.pattern, &subject)?];
            if !self.useful_pattern(&matrix, &query, subjects, &mut work, 0)? {
                return Err(self.pattern_error(&case.pattern.span, "unreachable match pattern"));
            }
            matrix.push(query);
        }
        if self.useful_pattern(&matrix, &[Pattern::Any], subjects, &mut work, 0)? {
            return Err(self.err("nonexhaustive match; missing an alternative (or a wildcard)"));
        }
        Ok(if inferred == Type::Never || !returns {
            Type::Never
        } else {
            output
        })
    }
}
