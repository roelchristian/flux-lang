//! Expression inference in its checked lexical and effect context.
use super::declarations::scan_dependencies;
use super::effects::immediate_raise;
use super::{a, f, function, mono, p, result, s, Engine, Env, R};
use crate::parser::{Expr, Stmt};
use crate::types::{NominalId, Prim, Row, Type};
use std::collections::{HashMap, HashSet};

impl Engine<'_> {
    pub(super) fn lookup(&mut self, env: &Env, n: &str) -> R<Type> {
        let sc = env
            .get(n)
            .ok_or_else(|| self.err(format!("unknown value {n}")))?;
        let (t, cs) = self.cx.instantiate(sc);
        for c in cs {
            self.req(c.class, &c.ty)?;
        }
        Ok(t)
    }

    fn block_body(
        &mut self,
        env: &Env,
        defs: &[Stmt],
        tail: Option<&Expr>,
        want: Option<&Type>,
        effects: bool,
    ) -> R<Type> {
        let mut nested = env.clone();
        self.schema_scopes.push(HashMap::new());
        let checked = (|| {
            self.schema_headers(defs)?;
            self.local_defs(defs, &mut nested, effects)?;
            let terminal = defs.iter().any(|stmt| match stmt {
                Stmt::Def { params, body, .. } if params.is_empty() => {
                    self.result.type_of(body) == Some(&Type::Never)
                }
                Stmt::Effect { body, .. } => self.result.type_of(body) == Some(&Type::Never),
                _ => false,
            });
            let t = if let Some(ex) = tail {
                self.expr(&nested, ex, (!terminal).then_some(want).flatten(), effects)?
            } else {
                p(Prim::Unit)
            };
            Ok(if terminal { Type::Never } else { t })
        })();
        self.schema_scopes.pop();
        checked
    }

    pub(super) fn expr(
        &mut self,
        env: &Env,
        e: &Expr,
        want: Option<&Type>,
        effects: bool,
    ) -> R<Type> {
        let previous_start = self.start;
        let previous_span = self.active.take();
        self.active = self.resolver.span(e);
        if let Some(start) = self.resolver.offset(e) {
            self.start = start;
        }
        let outcome = self.expr_inner(env, e, want, effects);
        self.start = previous_start;
        self.active = previous_span;
        outcome
    }

    fn expr_inner(&mut self, env: &Env, e: &Expr, want: Option<&Type>, effects: bool) -> R<Type> {
        use Prim::*;
        let t = match e {
            Expr::Int(n) if self.entry_natural_tail == Some(self.syntax_id(e)) => {
                self.int(*n, Some(&p(N)))?
            }
            Expr::Int(n) => self.int(*n, want)?,
            Expr::Num(_) if self.entry_natural_tail == Some(self.syntax_id(e)) => {
                self.int_signed(false, Some(&p(N)))?
            }
            Expr::Num(raw)
                if raw
                    .trim_start_matches('-')
                    .bytes()
                    .all(|b| b.is_ascii_digit()) =>
            {
                self.int_signed(raw.starts_with('-'), want)?
            }
            Expr::Num(raw) => {
                let k = match raw.as_bytes().last() {
                    Some(b'n') => N,
                    Some(b'z') => Z,
                    Some(b'q') => Q,
                    Some(b'd') => D,
                    Some(b'r') => R,
                    _ if raw.contains('.') => Q,
                    _ => Z,
                };
                if k == N && raw.starts_with('-') {
                    return Err(self.err("negative natural literal"));
                }
                if matches!(k, N | Z) && raw.contains('.') {
                    return Err(self.err("integral suffix on a fraction"));
                }
                p(k)
            }
            Expr::Str(_) => p(S),
            Expr::Char(_) => p(C),
            Expr::Bool(_) => p(B),
            Expr::Unit => p(Unit),
            Expr::Name(n) => match self.constructor_type(e) {
                Some(t) => t,
                None => self.lookup(env, n)?,
            },
            Expr::Ref(n) => {
                if let Some(t) = self.constructor_type(e) {
                    t
                } else if env.contains_key(n) {
                    self.lookup(env, n)?
                } else {
                    self.builtin_ref(n, want)?
                }
            }
            Expr::Tuple(es) => {
                let target = want.map(|t| self.cx.resolve(t));
                let xs = match &target {
                    Some(Type::Tuple(xs)) if xs.len() == es.len() => Some(xs),
                    _ => None,
                };
                let ts = es
                    .iter()
                    .enumerate()
                    .map(|(i, x)| self.expr(env, x, xs.map(|ts| &ts[i]), effects))
                    .collect::<crate::infer::R<Vec<_>>>()?;
                if ts.contains(&Type::Never) {
                    Type::Never
                } else {
                    Type::Tuple(ts)
                }
            }
            Expr::Array(es) => {
                let element = match want.map(|t| self.cx.resolve(t)) {
                    Some(Type::Seq(t)) => *t,
                    _ => self.cx.fresh(),
                };
                if es.is_empty() {
                    let end = self.active.as_ref().map_or(self.start, |span| span.end);
                    self.empty_sequences
                        .push((element.clone(), self.start, end));
                }
                let mut bottom = false;
                for e in es {
                    bottom |= self.expr(env, e, Some(&element), effects)? == Type::Never;
                }
                if bottom {
                    Type::Never
                } else {
                    s(element)
                }
            }
            Expr::Record(fs) | Expr::NamedRecord(_, fs) => {
                let nominal = if let Expr::NamedRecord(name, _) = e {
                    let id = self
                        .resolver
                        .reference(e)
                        .ok_or_else(|| self.err(format!("missing identity for schema {name}")))?;
                    Some(NominalId {
                        id: id.0,
                        name: name.clone(),
                    })
                } else {
                    None
                };
                let expected = match nominal.as_ref() {
                    Some(id) => Some(
                        self.schemas
                            .get(id)
                            .cloned()
                            .ok_or_else(|| self.err(format!("unknown schema {}", id.name)))?,
                    ),
                    _ => match want.map(|t| self.cx.resolve(t)) {
                        Some(Type::Record(r)) => Some(r),
                        _ => None,
                    },
                };
                let mut fields = Vec::new();
                for (n, ex) in fs {
                    if fields.iter().any(|(k, _)| k == n) {
                        return Err(self.err(format!("duplicate field {n}")));
                    }
                    let typ = expected
                        .as_ref()
                        .and_then(|r| r.fields.iter().find(|(name, _)| name == n).map(|(_, t)| t));
                    fields.push((n.clone(), self.expr(env, ex, typ, effects)?));
                }
                if fields.iter().any(|(_, t)| t == &Type::Never) {
                    Type::Never
                } else if let Some(id) = nominal {
                    let typ = Type::Record(expected.expect("nominal schema"));
                    self.unify(&Type::Record(Row { fields, tail: None }), &typ)?;
                    Type::Nominal(id)
                } else {
                    Type::Record(Row { fields, tail: None })
                }
            }
            Expr::Cases(cases, alt) => {
                let t = want.cloned().unwrap_or_else(|| self.cx.fresh());
                let mut normal = false;
                let mut reachable = true;
                for (cond, body) in cases {
                    let c = self.expr(env, cond, Some(&p(B)), effects)?;
                    if c == Type::Never {
                        reachable = false;
                    }
                    let ty = self.expr(env, body, Some(&t), effects)?;
                    if reachable && ty != Type::Never {
                        normal = true;
                    }
                }
                let other = self.expr(env, alt, Some(&t), effects)?;
                if normal || (reachable && other != Type::Never) {
                    t
                } else {
                    Type::Never
                }
            }
            Expr::Lambda(ps, body) | Expr::TypedLambda(ps, _, body) => {
                let anns = match e {
                    Expr::TypedLambda(_, ts, _) => ts.as_slice(),
                    _ => &[],
                };
                let sig = self.signature(ps, anns, None)?;
                let t = function(sig.args.iter().cloned(), sig.out.clone());
                if let Some(w) = want {
                    self.unify(&t, w)?;
                }
                let mut nested = env.clone();
                for ((name, t), id) in ps
                    .iter()
                    .zip(sig.args.iter())
                    .zip(self.resolver.lambda_parameters(e))
                {
                    nested.insert(name.clone(), mono(t.clone()));
                    self.result.bindings.insert(*id, mono(t.clone()));
                }
                // Parameter IDs are metadata, not a prerequisite for lexical checking.
                for (name, t) in ps.iter().zip(sig.args) {
                    nested.insert(name.clone(), mono(t));
                }
                self.reset_boundary(|this| this.expr(&nested, body, Some(&sig.out), false))?;
                t
            }
            Expr::Block(defs, tail) => self.block_body(env, defs, tail.as_deref(), want, false)?,
            Expr::Action(defs, tail) => {
                let expected = match want.map(|x| self.cx.resolve(x)) {
                    Some(Type::Action(t)) => Some(*t),
                    _ => None,
                };
                a(self.reset_boundary(|this| {
                    this.block_body(env, defs, tail.as_deref(), expected.as_ref(), true)
                })?)
            }
            Expr::Fallible(defs, tail) | Expr::FallibleActionBody(defs, tail) => {
                self.saw_result = true;
                let (ok, err) = match want.map(|x| self.cx.resolve(x)) {
                    Some(Type::Result(ok, err)) => (*ok, *err),
                    _ => (self.cx.fresh(), self.cx.fresh()),
                };
                let id = self.syntax_id(e);
                self.result.boundaries.insert(id, id);
                self.boundaries.push((id, err.clone(), false));
                let output = self.block_body(env, defs, tail.as_deref(), Some(&ok), effects);
                let (_, _, used) = self.boundaries.pop().expect("active fallible boundary");
                let output = output?;
                if output != Type::Never {
                    self.unify(&ok, &output)?;
                } else if self.entry_boundary == Some(id)
                    && matches!(self.cx.resolve(&ok), Type::Var(_))
                {
                    // A top-level boundary with no successful control-flow
                    // path has a trivially valid unit process success type.
                    self.unify(&ok, &p(Unit))?;
                }
                if matches!(e, Expr::FallibleActionBody(..))
                    && !used
                    && matches!(self.cx.resolve(&err), Type::Var(_))
                {
                    self.combined_error_defaults.push(err.clone());
                }
                result(ok, err)
            }
            Expr::Raise(value) => {
                let err = self.at_boundary(e)?;
                let old = self.raise_payload.replace(self.syntax_id(value));
                let typed = self.expr(env, value, Some(&err), effects);
                self.raise_payload = old;
                typed?;
                self.raised_error_types.push(err);
                Type::Never
            }
            Expr::Propagate(value) => {
                let err = self.at_boundary(e)?;
                let ok = want.cloned().unwrap_or_else(|| self.cx.fresh());
                if self.expr(env, value, Some(&result(ok.clone(), err)), effects)? == Type::Never {
                    Type::Never
                } else {
                    ok
                }
            }
            Expr::AutoPropagate(value) => {
                let executed = self.expr(env, value, None, effects)?;
                match self.cx.resolve(&executed) {
                    Type::Never => Type::Never,
                    Type::Prim(Unit) => p(Unit),
                    Type::Result(ok, err) => {
                        if self.unify(&ok, &p(Unit)).is_err() {
                            return Err(self.err(
                                "standalone execution has a non-unit success value; bind or explicitly handle its result",
                            ));
                        }
                        let expected = self.at_boundary(e)?;
                        self.unify(&err, &expected)?;
                        p(Unit)
                    }
                    _ => return Err(self.err(
                        "standalone execution must produce unit or a unit result; bind or explicitly handle its value",
                    )),
                }
            }
            Expr::Match(value, cases) => self.match_expr(env, value, cases, want, effects)?,
            Expr::Recover(value, cases) => {
                self.saw_result = true;
                self.recover(env, value, cases, want, effects)?
            }
            Expr::Pipeline(stages) => {
                let Some((head, tail)) = stages.split_first() else {
                    return Err(self.err("empty pipeline"));
                };
                let mut t = self.expr(
                    env,
                    head,
                    if tail.is_empty() { want } else { None },
                    effects,
                )?;
                let mut nested = env.clone();
                let mut prev = head;
                let mut terminal = t == Type::Never;
                for (i, ex) in tail.iter().enumerate() {
                    let possibly_result =
                        matches!(self.cx.resolve(&t), Type::Result(..) | Type::Var(_));
                    let used = !possibly_result
                        || self.resolver.placeholder(ex).is_some_and(|id| {
                            let mut found = HashSet::new();
                            scan_dependencies(ex, self.resolver, &HashSet::from([id]), &mut found);
                            found.contains(&id)
                        });
                    if !used {
                        let at = self.resolver.offset(prev).unwrap_or(self.start);
                        let span = self.resolver.span(prev).unwrap_or(at..at);
                        self.discarded_results
                            .push((t.clone(), span.start, span.end));
                    }
                    nested.insert("$".into(), mono(t.clone()));
                    t = self.expr(
                        &nested,
                        ex,
                        if !terminal && i + 1 == tail.len() {
                            want
                        } else {
                            None
                        },
                        effects,
                    )?;
                    terminal |= t == Type::Never;
                    prev = ex;
                }
                if terminal {
                    Type::Never
                } else {
                    t
                }
            }
            Expr::Call(n, args) => {
                if let Some(callee) = self.constructor_type(e) {
                    self.result
                        .callees
                        .insert(self.syntax_id(e), callee.clone());
                    self.apply(env, callee, args, want, effects)?
                } else if env.contains_key(n) {
                    let callee = self.lookup(env, n)?;
                    self.result
                        .callees
                        .insert(self.syntax_id(e), callee.clone());
                    self.apply(env, callee, args, want, effects)?
                } else {
                    self.builtin_call(env, e, n, args, want, effects)?
                }
            }
        };
        if let Some(w) = want {
            if t != Type::Never {
                if self.raise_payload == Some(self.syntax_id(e)) {
                    let target = self.cx.resolve(w);
                    let actual = self.cx.resolve(&t);
                    if target != actual
                        && self.cx.free_vars(&target).is_empty()
                        && self.cx.free_vars(&actual).is_empty()
                    {
                        return Err(self.incompatible(&actual, &target));
                    }
                }
                self.unify(&t, w)?;
            }
        }
        self.result.expressions.insert(self.syntax_id(e), t.clone());
        Ok(t)
    }

    fn apply(
        &mut self,
        env: &Env,
        mut callee: Type,
        args: &[Expr],
        want: Option<&Type>,
        effects: bool,
    ) -> R<Type> {
        let mut bottom = false;
        // Check strict operands before the final contextual return type when
        // an argument contains an immediate lexical escape.
        let delay = !self.boundaries.is_empty() && args.iter().any(immediate_raise);
        for (i, ex) in args.iter().enumerate() {
            let arg = self.cx.fresh();
            let out = if !delay && i + 1 == args.len() {
                want.cloned().unwrap_or_else(|| self.cx.fresh())
            } else {
                self.cx.fresh()
            };
            self.unify(&callee, &f(arg.clone(), out.clone()))?;
            bottom |= self.expr(env, ex, Some(&arg), effects)? == Type::Never;
            callee = out;
        }
        Ok(if bottom { Type::Never } else { callee })
    }
}
