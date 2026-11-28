//! Binding generalisation, local dependencies, and declaration groups.
use super::{function, mono, p, Engine, Env, Operation, Signature, R};
use crate::parser::{Expr, Stmt};
use crate::resolve::{Resolution, SymbolId};
use crate::types::{Class, Constraint, Prim, Row, Scheme, Type};
use std::collections::{HashMap, HashSet};

// The empty open-record marker is used only in the annotation variable
// map. Row variables never inhabit ordinary Type::Var positions.
fn explicit_variable_id(t: &Type) -> Option<u32> {
    match t {
        Type::Var(id) => Some(*id),
        Type::Record(Row {
            fields,
            tail: Some(id),
        }) if fields.is_empty() => Some(*id),
        _ => None,
    }
}
fn syntactic_value(e: &Expr) -> bool {
    match e {
        Expr::Int(_)
        | Expr::Num(_)
        | Expr::Str(_)
        | Expr::Char(_)
        | Expr::Bool(_)
        | Expr::Unit
        | Expr::Name(_)
        | Expr::Ref(_)
        | Expr::Lambda(..)
        | Expr::TypedLambda(..) => true,
        Expr::Tuple(es) | Expr::Array(es) => es.iter().all(syntactic_value),
        Expr::Record(fs) | Expr::NamedRecord(_, fs) => fs.iter().all(|(_, x)| syntactic_value(x)),
        // A generalized tacit block is a closure only after all eager
        // captures are initialized. Do not generalize expansive captures.
        Expr::Block(defs, Some(tail)) if !defs.is_empty() => {
            defs.iter().all(|stmt| {
                crate::parser::is_capture(stmt)
                    && stmt
                        .body_and_arity()
                        .is_some_and(|(body, _)| syntactic_value(body))
            }) && syntactic_value(tail)
        }
        _ => false,
    }
}
pub(super) fn scan_dependencies(
    e: &Expr,
    res: &Resolution,
    local: &HashSet<SymbolId>,
    out: &mut HashSet<SymbolId>,
) {
    if let Some(id) = res.reference(e).filter(|id| local.contains(id)) {
        out.insert(id);
    }
    e.for_each_child(|child| scan_dependencies(child, res, local, out));
}

impl Engine<'_> {
    fn bind(&mut self, env: &mut Env, stmt: &Stmt, name: &str, scheme: Scheme) {
        env.insert(name.to_owned(), scheme.clone());
        if let Some(id) = self.resolver.declaration(stmt) {
            self.result.bindings.insert(id, scheme);
        }
    }

    fn body(&mut self, env: &Env, stmt: &Stmt, sig: &Signature, effect: bool) -> R<()> {
        let Stmt::Def {
            name,
            params,
            body,
            start,
            ..
        } = stmt
        else {
            unreachable!()
        };
        self.start = *start;
        let checked = if params.is_empty() {
            self.expr(env, body, Some(&sig.out), effect)
        } else {
            let mut nested = env.clone();
            let ids = self.resolver.parameters(stmt);
            for (i, (name, t)) in params.iter().zip(&sig.args).enumerate() {
                nested.insert(name.clone(), mono(t.clone()));
                if let Some(id) = ids.get(i) {
                    self.result.bindings.insert(*id, mono(t.clone()));
                }
            }
            self.reset_boundary(|this| this.expr(&nested, body, Some(&sig.out), false))
        };
        checked.map_err(|mut e| {
            e.message = format!("{name}: {}", e.message);
            e
        })?;
        Ok(())
    }

    fn finish_signature(&mut self, sig: &Signature, outer: &Env) -> R<()> {
        if sig.explicit.is_empty() {
            return Ok(());
        }
        let ids = sig
            .explicit
            .values()
            .filter_map(explicit_variable_id)
            .collect::<Vec<_>>();
        let outer_types = outer.values().cloned().collect::<Vec<_>>();
        if self.cx.rigid_escapes(&ids, &outer_types) {
            return Err(
                self.err("a quantified signature variable escapes into a captured outer binding")
            );
        }
        for c in self.cx.constraints() {
            let t = self.cx.resolve(&c.ty);
            if self.cx.free_vars(&t).iter().any(|id| ids.contains(id))
                && !sig
                    .qualifiers
                    .iter()
                    .any(|q| q.class == c.class && self.cx.resolve(&q.ty) == t)
            {
                let name = sig
                    .explicit
                    .iter()
                    .find_map(|(name, declared)| (declared == &t).then_some(name.as_str()))
                    .map(str::to_owned)
                    .unwrap_or_else(|| t.to_string());
                return Err(self.err(format!("annotated signature lacks {} {name}", c.class)));
            }
        }
        for id in ids {
            self.cx.unmark_rigid(id)
        }
        Ok(())
    }

    fn def_signature(&mut self, stmt: &Stmt) -> R<Signature> {
        let Stmt::Def {
            params,
            param_types,
            result_type,
            body,
            start,
            ..
        } = stmt
        else {
            unreachable!()
        };
        self.start = *start;
        let mut sig = self.signature(params, param_types, result_type.as_ref())?;
        // Checking a polymorphic annotation against rigid variables and then
        // storing it as a monotype would silently specialize that annotation.
        if params.is_empty() && !sig.explicit.is_empty() && !syntactic_value(body) {
            return Err(self.err(
                "the value restriction does not allow a quantified signature on an expansive binding",
            ));
        }
        if let Some(qs) = self.qualifiers.get(start) {
            for (c, v) in qs {
                let class = match c.as_str() {
                    "num" => Class::Num,
                    "eq" => Class::Eq,
                    "ord" => Class::Ord,
                    "hash" => Class::Hash,
                    _ => return Err(self.err(format!("unknown type constraint {c}"))),
                };
                let ty = sig
                    .explicit
                    .get(v)
                    .ok_or_else(|| {
                        self.err(format!("constraint variable {v} is not in the signature"))
                    })?
                    .clone();
                if !matches!(ty, Type::Var(_)) {
                    return Err(self.err(format!(
                        "class qualifier {c} expects an ordinary type variable; {v} has row kind"
                    )));
                }
                sig.qualifiers.push(Constraint { class, ty });
            }
        }
        for id in sig.explicit.values().filter_map(explicit_variable_id) {
            self.cx.mark_rigid(id);
            self.explicit_variables.insert(id);
        }
        for c in &sig.qualifiers {
            self.req(c.class, &c.ty)?;
        }
        Ok(sig)
    }

    fn scheme(&self, env: &Env, typ: &Type, allow: bool) -> Scheme {
        if !allow {
            return mono(self.cx.resolve(typ));
        }
        let candidate = self.cx.generalize(typ, &[], &[]);
        if candidate.vars.is_empty() {
            return candidate;
        }
        let others = env.values().cloned().collect::<Vec<_>>();
        self.cx.generalize(typ, &others, &self.cx.constraints())
    }

    fn closed_integer_type(&mut self, e: &Expr, env: &Env) -> Option<Type> {
        // This is equivalent to defaulting each connected literal metavariable to
        // z. Only recognize closed scalar integer terms whose operators preserve z.
        // Stage metadata until the entire term has been classified.
        fn collect<'a>(e: &'a Expr, env: &Env, out: &mut Vec<&'a Expr>) -> bool {
            match e {
                Expr::Int(_) => {}
                Expr::Call(n, args)
                    if matches!(n.as_str(), "+" | "-" | "*")
                        && args.len() == 2
                        && !env.contains_key(n) =>
                {
                    if !args.iter().all(|x| collect(x, env, out)) {
                        return false;
                    }
                }
                _ => return false,
            }
            out.push(e);
            true
        }
        let mut nodes = Vec::with_capacity(8);
        if !collect(e, env, &mut nodes) {
            return None;
        }
        let z = p(Prim::Z);
        for node in nodes {
            let addr = self.syntax_id(node);
            self.result.expressions.insert(addr, z.clone());
            if let Expr::Call(op, _) = node {
                self.result.operations.insert(
                    addr,
                    Operation::Numeric {
                        operator: op.clone(),
                        result: z.clone(),
                    },
                );
            }
        }
        Some(z)
    }

    pub(super) fn group(&mut self, defs: Vec<&Stmt>, env: &mut Env, effect: bool) -> R<()> {
        let default_from = self.combined_error_defaults.len();
        // Ordinary immutable values do not add a recursive header. Infer them
        // against the existing environment instead of cloning every prior value.
        if let [stmt @ Stmt::Def {
            name,
            params,
            body,
            result_type,
            start,
            ..
        }] = defs.as_slice()
        {
            if params.is_empty() {
                if result_type.is_none() && !self.qualifiers.contains_key(start) {
                    if let Some(ty) = self.closed_integer_type(body, env) {
                        self.bind(env, stmt, name, mono(ty));
                        return Ok(());
                    }
                }
                let sig = self.def_signature(stmt)?;
                let pos = self.literals.len();
                self.body(env, stmt, &sig, effect)?;
                self.default_literals(pos)?;
                self.default_combined_errors(default_from, sig.args.iter(), env)?;
                self.start = *start;
                self.finish_signature(&sig, env)?;
                if matches!(body,Expr::Array(es) if es.is_empty())
                    && matches!(self.cx.resolve(&sig.out),Type::Seq(x) if matches!(*x,Type::Var(_)))
                {
                    return Err(self.err("empty sequence needs an element type annotation"));
                }
                let sc = self.scheme(env, &sig.out, syntactic_value(body));
                self.bind(env, stmt, name, sc);
                return Ok(());
            }
        }
        let outer = env.clone();
        let mut sigs = Vec::new();
        let literals = self.literals.len();
        for stmt in defs {
            match stmt {
                Stmt::Schema { .. } | Stmt::Sum { .. } => {}
                Stmt::Def { name, params, .. } => {
                    let sig = self.def_signature(stmt)?;
                    if !params.is_empty() {
                        let t = function(sig.args.iter().cloned(), sig.out.clone());
                        env.insert(name.clone(), mono(t));
                    }
                    sigs.push((stmt, sig));
                }
                Stmt::Effect { name, body, start } => {
                    self.start = *start;
                    if !effect && self.boundaries.is_empty() {
                        return Err(self.err("expression statement in pure block"));
                    }
                    let t = self.expr(env, body, None, effect)?;
                    if let Some(n) = name {
                        self.bind(env, stmt, n, mono(t));
                    } else {
                        let span = self.resolver.span(body).unwrap_or(*start..*start);
                        self.discarded_results.push((t, span.start, span.end));
                    }
                }
            }
        }
        for (stmt, sig) in &sigs {
            self.body(env, stmt, sig, effect)?;
        }
        self.default_literals(literals)?;
        if !sigs.is_empty() {
            self.default_combined_errors(
                default_from,
                sigs.iter().flat_map(|(_, sig)| &sig.args),
                &outer,
            )?;
        }
        for (stmt, sig) in sigs {
            let Stmt::Def {
                params,
                name,
                body,
                start,
                ..
            } = stmt
            else {
                unreachable!()
            };
            self.start = *start;
            self.finish_signature(&sig, &outer)?;
            let t = function(sig.args.iter().cloned(), sig.out.clone());
            if params.is_empty()
                && matches!(body,Expr::Array(es) if es.is_empty())
                && matches!(self.cx.resolve(&t), Type::Seq(x) if matches!(*x, Type::Var(_)))
            {
                return Err(self.err("empty sequence needs an element type annotation"));
            }
            let sc = self.scheme(&outer, &t, !params.is_empty() || syntactic_value(body));
            self.bind(env, stmt, name, sc);
        }
        Ok(())
    }

    pub(super) fn local_defs(&mut self, defs: &[Stmt], env: &mut Env, effect: bool) -> R<()> {
        // Order value visibility by the declaration itself. A local recursive
        // group stays monomorphic until all transitively referenced functions
        // have been checked, then its variables can be freshly instantiated.
        let base = env.clone();
        let local = defs
            .iter()
            .filter_map(|stmt| match stmt {
                Stmt::Def { params, .. } if !params.is_empty() => self.resolver.declaration(stmt),
                _ => None,
            })
            .collect::<HashSet<_>>();
        let local_names = local
            .iter()
            .map(|&id| self.resolver.symbol(id).name.as_str())
            .collect::<HashSet<_>>();
        let mut signatures = HashMap::new();
        let mut deps = HashMap::new();
        for stmt in defs {
            match stmt {
                Stmt::Schema { .. } | Stmt::Sum { .. } => {}
                Stmt::Def {
                    name, params, body, ..
                } if !params.is_empty() => {
                    let id = self
                        .resolver
                        .declaration(stmt)
                        .ok_or_else(|| self.err("missing local callable symbol"))?;
                    let sig = self.def_signature(stmt)?;
                    env.insert(
                        name.clone(),
                        mono(function(sig.args.iter().cloned(), sig.out.clone())),
                    );
                    let mut incoming = HashSet::new();
                    scan_dependencies(body, self.resolver, &local, &mut incoming);
                    deps.insert(id, incoming);
                    signatures.insert(id, sig);
                }
                _ => {}
            }
        }
        let mut pending: Vec<(SymbolId, &Stmt, Signature, Vec<Type>)> = Vec::new();
        let mut checked = HashSet::new();
        for stmt in defs {
            match stmt {
                Stmt::Def { params, .. } if !params.is_empty() => {
                    let id = self.resolver.declaration(stmt).expect("local symbol");
                    let sig = signatures.remove(&id).expect("local header signature");
                    let pos = self.literals.len();
                    let defaults = self.combined_error_defaults.len();
                    self.body(env, stmt, &sig, effect)?;
                    self.default_literals(pos)?;
                    let defaults = self.combined_error_defaults.split_off(defaults);
                    checked.insert(id);
                    pending.push((id, stmt, sig, defaults));
                    // A candidate can be generalized exactly when its whole
                    // reachable local-callable set has been checked.
                    let ready = |i| {
                        let mut stack = vec![i];
                        let mut seen = HashSet::new();
                        while let Some(id) = stack.pop() {
                            if !seen.insert(id) {
                                continue;
                            }
                            if !checked.contains(&id) {
                                return false;
                            }
                            if let Some(ds) = deps.get(&id) {
                                stack.extend(ds.iter().copied());
                            }
                        }
                        true
                    };
                    let mut done = Vec::new();
                    let mut waiting = Vec::new();
                    for item in pending.drain(..) {
                        if ready(item.0) {
                            done.push(item);
                        } else {
                            waiting.push(item);
                        }
                    }
                    pending = waiting;
                    if !done.is_empty() {
                        let mut visible = base.clone();
                        visible.extend(
                            env.iter()
                                .filter(|(n, _)| !local_names.contains(n.as_str()))
                                .map(|(n, t)| (n.clone(), t.clone())),
                        );
                        let default_from = self.combined_error_defaults.len();
                        for (_, _, _, defaults) in &mut done {
                            self.combined_error_defaults.append(defaults);
                        }
                        self.default_combined_errors(
                            default_from,
                            done.iter().flat_map(|(_, _, sig, _)| sig.args.iter()),
                            &visible,
                        )?;
                        for (_, stmt, sig, _) in done {
                            let Stmt::Def { name, start, .. } = stmt else {
                                unreachable!()
                            };
                            self.start = *start;
                            self.finish_signature(&sig, &visible)?;
                            let t = function(sig.args.into_iter(), sig.out);
                            let sc = self.scheme(&visible, &t, true);
                            self.bind(env, stmt, name, sc);
                        }
                    }
                }
                Stmt::Schema { .. } | Stmt::Sum { .. } => {}
                _ => self.group(vec![stmt], env, effect)?,
            }
        }
        Ok(())
    }
}
