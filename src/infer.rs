//! Rank-one inference for the Draft 0.4 checked subset.
use crate::{
    parser::{Expr, Program, Stmt},
    resolve::{Resolution, SymbolId},
    types::{Class, Constraint, InferCtx, Prim, Row, Scheme, Type},
};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InferErrorKind {
    TypeError,
    Unsupported,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InferError {
    pub kind: InferErrorKind,
    pub start: usize,
    pub end: usize,
    pub message: String,
}
impl std::fmt::Display for InferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for InferError {}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    Builtin(String),
    Numeric { operator: String, result: Type },
}
#[derive(Debug, Default)]
pub struct InferResult {
    pub expressions: HashMap<usize, Type>,
    pub bindings: HashMap<SymbolId, Scheme>,
    pub operations: HashMap<usize, Operation>,
    pub schemas: HashMap<String, Row>,
}
fn address(e: &Expr) -> usize {
    (e as *const Expr) as usize
}
impl InferResult {
    pub fn nominal_schema(&self, name: &str) -> Option<&Row> {
        self.schemas.get(name)
    }
    pub fn type_of(&self, e: &Expr) -> Option<&Type> {
        self.expressions.get(&address(e))
    }
    pub fn binding_type(&self, id: SymbolId) -> Option<&Scheme> {
        self.bindings.get(&id)
    }
    pub fn operation_of(&self, e: &Expr) -> Option<&Operation> {
        self.operations.get(&address(e))
    }
}
type R<T> = Result<T, InferError>;
type Env = HashMap<String, Scheme>;
fn b(t: Type) -> Box<Type> {
    Box::new(t)
}
fn p(t: Prim) -> Type {
    Type::Prim(t)
}
fn s(t: Type) -> Type {
    Type::Seq(b(t))
}
fn o(t: Type) -> Type {
    Type::Optional(b(t))
}
fn a(t: Type) -> Type {
    Type::Action(b(t))
}
fn f(arg: Type, result: Type) -> Type {
    Type::Fun(b(arg), b(result))
}
fn function(args: impl DoubleEndedIterator<Item = Type>, out: Type) -> Type {
    args.rev().fold(out, |out, arg| f(arg, out))
}
fn mono(ty: Type) -> Scheme {
    Scheme {
        vars: vec![],
        ty,
        constraints: vec![],
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
        _ => false,
    }
}
fn scan_dependencies(
    e: &Expr,
    res: &Resolution,
    local: &HashSet<SymbolId>,
    out: &mut HashSet<SymbolId>,
) {
    if let Some(id) = res.reference(e) {
        if local.contains(&id) {
            out.insert(id);
        }
    }
    use Expr::*;
    let mut visit = |e: &Expr| scan_dependencies(e, res, local, out);
    match e {
        Call(_, xs) | Array(xs) | Tuple(xs) | Pipeline(xs) => {
            for x in xs {
                visit(x)
            }
        }
        Lambda(_, x) | TypedLambda(_, _, x) => visit(x),
        Cases(cs, alt) => {
            for (c, e) in cs {
                visit(c);
                visit(e);
            }
            visit(alt);
        }
        Record(fs) | NamedRecord(_, fs) => {
            for (_, e) in fs {
                visit(e);
            }
        }
        Block(ds, tail) | Action(ds, tail) => {
            for d in ds {
                if let Stmt::Def { body, .. } | Stmt::Effect { body, .. } = d {
                    visit(body);
                }
            }
            if let Some(t) = tail {
                visit(t);
            }
        }
        _ => {}
    }
}
fn exact(t: Prim) -> bool {
    matches!(t, Prim::N | Prim::Z | Prim::Q | Prim::D)
}
fn num(t: Prim) -> bool {
    exact(t) || t == Prim::R
}
fn type_has_nominal(t: &Type) -> bool {
    match t {
        Type::Nominal(_) => true,
        Type::Seq(t) | Type::Table(t) | Type::Optional(t) | Type::Action(t) => type_has_nominal(t),
        Type::Fun(a, b) | Type::Dict(a, b) => type_has_nominal(a) || type_has_nominal(b),
        Type::Tuple(ts) => ts.iter().any(type_has_nominal),
        Type::Record(row) => row.fields.iter().any(|(_, t)| type_has_nominal(t)),
        _ => false,
    }
}
fn solve_numeric(a: Prim, b: Prim) -> Option<Prim> {
    use crate::builtins::{promote, Numeric as K};
    fn to(x: Prim) -> Option<K> {
        Some(match x {
            Prim::N => K::N,
            Prim::Z => K::Z,
            Prim::Q => K::Q,
            Prim::D => K::D,
            Prim::R => K::R,
            _ => return None,
        })
    }
    Some(match promote(to(a)?, to(b)?)? {
        K::N => Prim::N,
        K::Z => Prim::Z,
        K::Q => Prim::Q,
        K::D => Prim::D,
        K::R => Prim::R,
    })
}

#[derive(Clone)]
struct Signature {
    args: Vec<Type>,
    out: Type,
    explicit: HashMap<String, Type>,
    qualifiers: Vec<Constraint>,
}
struct Engine<'r> {
    cx: InferCtx,
    result: InferResult,
    resolver: &'r Resolution,
    schemas: HashMap<String, Row>,
    start: usize,
    literals: Vec<(Type, i128, usize, usize)>,
    active: Option<std::ops::Range<usize>>,
    qualifiers: &'r std::collections::BTreeMap<usize, Vec<(String, String)>>,
}
impl Engine<'_> {
    fn err(&self, text: impl Into<String>) -> InferError {
        InferError {
            kind: InferErrorKind::TypeError,
            start: self.start,
            end: self
                .active
                .as_ref()
                .filter(|s| s.start == self.start)
                .map_or(self.start, |s| s.end),
            message: text.into(),
        }
    }
    fn unsupported(&self, text: impl Into<String>) -> InferError {
        InferError {
            kind: InferErrorKind::Unsupported,
            start: self.start,
            end: self
                .active
                .as_ref()
                .filter(|s| s.start == self.start)
                .map_or(self.start, |s| s.end),
            message: text.into(),
        }
    }
    fn unify(&mut self, x: &Type, y: &Type) -> R<()> {
        // The generic unifier cannot resolve nominal schema names. Check any
        // pending class obligations that are about to take on a nominal type.
        let has_nominal =
            type_has_nominal(&self.cx.resolve(x)) || type_has_nominal(&self.cx.resolve(y));
        let pending = if has_nominal {
            self.cx.constraints()
        } else {
            Vec::new()
        };
        self.cx.unify_committing(x, y).map_err(|e| self.err(e))?;
        for c in pending {
            self.nominal_evidence(c.class, &c.ty, &mut HashSet::new())?;
        }
        Ok(())
    }
    fn req(&mut self, c: Class, t: &Type) -> R<()> {
        self.nominal_evidence(c, t, &mut HashSet::new())?;
        self.cx.require(c, t).map_err(|e| self.err(e))
    }
    fn nominal_evidence(&mut self, c: Class, t: &Type, seen: &mut HashSet<String>) -> R<()> {
        if !matches!(c, Class::Eq | Class::Hash) {
            return Ok(());
        }
        match self.cx.resolve(t) {
            Type::Nominal(n) if seen.insert(n.clone()) => {
                // Hash remains intentionally unavailable for nominal records.
                if c == Class::Hash {
                    return Err(self.err(format!("nominal record {n} does not satisfy hash")));
                }
                let row = self
                    .schemas
                    .get(&n)
                    .cloned()
                    .ok_or_else(|| self.err(format!("unknown schema {n}")))?;
                for (field, t) in row.fields {
                    self.nominal_evidence(c, &t, seen)?;
                    self.cx
                        .require(c, &t)
                        .map_err(|text| self.err(format!("field {n}.{field}: {text}")))?;
                }
            }
            Type::Seq(x) | Type::Table(x) | Type::Optional(x) => {
                self.nominal_evidence(c, &x, seen)?
            }
            Type::Dict(k, v) => {
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
    fn annotation(&mut self, t: &str, vars: &mut HashMap<String, Type>) -> R<Type> {
        // Annotations are source token strings; invoke the same maximal-munch lexer.
        let toks = crate::lexer::lex(t).map_err(|e| self.err(e.message))?;
        let mut index = 0;
        let v = self.type_arrow(&toks, &mut index, vars, &[])?;
        if index != toks.len() {
            return Err(self.err("trailing type annotation tokens"));
        }
        Ok(v)
    }
    fn type_arrow(
        &mut self,
        t: &[crate::lexer::Token],
        i: &mut usize,
        v: &mut HashMap<String, Type>,
        ends: &[&str],
    ) -> R<Type> {
        let next = |i: usize| t.get(i).map(|x| x.text.as_str()).unwrap_or("");
        let mut args = vec![self.type_atom(t, i, v)?];
        while !next(*i).is_empty() && !ends.contains(&next(*i)) {
            if next(*i) == "->" {
                *i += 1;
                return Ok(function(args.into_iter(), self.type_arrow(t, i, v, ends)?));
            }
            args.push(self.type_atom(t, i, v)?)
        }
        if args.len() == 1 {
            Ok(args.remove(0))
        } else {
            Err(self.err("multiple type components need an arrow"))
        }
    }
    fn type_atom(
        &mut self,
        t: &[crate::lexer::Token],
        i: &mut usize,
        v: &mut HashMap<String, Type>,
    ) -> R<Type> {
        let next = |i: usize| t.get(i).map(|x| x.text.as_str()).unwrap_or("");
        let expect = |i: &mut usize, x: &str| {
            if next(*i) == x {
                *i += 1;
                Ok(())
            } else {
                Err(format!("expected {x} in type annotation"))
            }
        };
        let tok = next(*i);
        if tok.is_empty() {
            return Err(self.err("expected a type"));
        }
        *i += 1;
        use Prim::*;
        Ok(match tok {
            "n" => p(N),
            "z" => p(Z),
            "q" => p(Q),
            "d" => p(D),
            "r" => p(R),
            "s" => p(S),
            "b" => p(B),
            "c" => p(C),
            "_" => self.cx.fresh(),
            "?" => o(self.type_atom(t, i, v)?),
            "!" => a(self.type_atom(t, i, v)?),
            "#" => Type::Table(crate::infer::b(self.type_atom(t, i, v)?)),
            "[" => {
                let x = self.type_arrow(t, i, v, &["]"])?;
                expect(i, "]").map_err(|e| self.err(e))?;
                s(x)
            }
            "#[" => {
                let mut xs = Vec::new();
                while next(*i) != "]" {
                    xs.push(self.type_atom(t, i, v)?);
                    if next(*i) == "," {
                        *i += 1;
                    }
                }
                *i += 1;
                Type::Table(crate::infer::b(Type::Tuple(xs)))
            }
            "(" => {
                if next(*i) == ")" {
                    *i += 1;
                    p(Unit)
                } else {
                    let mut xs = vec![self.type_arrow(t, i, v, &[")", ","])?];
                    while next(*i) == "," {
                        *i += 1;
                        xs.push(self.type_arrow(t, i, v, &[")", ","])?);
                    }
                    expect(i, ")").map_err(|e| self.err(e))?;
                    if xs.len() == 1 {
                        xs.remove(0)
                    } else {
                        Type::Tuple(xs)
                    }
                }
            }
            "{" => {
                let k = self.type_arrow(t, i, v, &[":"])?;
                expect(i, ":").map_err(|e| self.err(e))?;
                let x = self.type_arrow(t, i, v, &["}"])?;
                expect(i, "}").map_err(|e| self.err(e))?;
                Type::Dict(crate::infer::b(k), crate::infer::b(x))
            }
            ".{" => {
                let mut fields = Vec::new();
                while next(*i) != "}" {
                    let name = next(*i);
                    if name == "|" || name == ".." {
                        return Err(self.err("explicit open-row annotations are not supported; infer the row from field use"));
                    }
                    if name.is_empty() {
                        return Err(self.err("unclosed record type"));
                    }
                    *i += 1;
                    expect(i, ":").map_err(|e| self.err(e))?;
                    let ty = self.type_arrow(t, i, v, &[";", "}", "|", ".."])?;
                    if fields.iter().any(|(k, _)| k == name) {
                        return Err(self.err(format!("duplicate field {name}")));
                    }
                    fields.push((name.into(), ty));
                    if next(*i) == ";" {
                        *i += 1;
                    } else if !matches!(next(*i), "}" | "|" | "..") {
                        return Err(self.err("expected ; or } in record type"));
                    }
                }
                expect(i, "}").map_err(|e| self.err(e))?;
                Type::Record(Row { fields, tail: None })
            }
            x if x.starts_with('\'') => {
                v.entry(x.into()).or_insert_with(|| self.cx.fresh()).clone()
            }
            x if self.schemas.contains_key(x) => Type::Nominal(x.into()),
            _ => return Err(self.err(format!("unknown type {tok}"))),
        })
    }
    fn signature(
        &mut self,
        params: &[String],
        types: &[Option<String>],
        out: Option<&str>,
    ) -> R<Signature> {
        let mut vars = HashMap::new();
        let mut args = Vec::new();
        for (i, _) in params.iter().enumerate() {
            args.push(match types.get(i).and_then(Option::as_deref) {
                Some(t) => self.annotation(t, &mut vars)?,
                None => self.cx.fresh(),
            })
        }
        let out = if let Some(t) = out {
            self.annotation(t, &mut vars)?
        } else {
            self.cx.fresh()
        };
        Ok(Signature {
            args,
            out,
            explicit: vars,
            qualifiers: vec![],
        })
    }
    fn install_schema(&mut self, name: &str, fields: &[(String, String)]) -> R<()> {
        let mut row = Row {
            fields: vec![],
            tail: None,
        };
        for (name, t) in fields {
            let x = self.annotation(t, &mut HashMap::new())?;
            if row.fields.iter().any(|(k, _)| k == name) {
                return Err(self.err(format!("duplicate field {name}")));
            }
            row.fields.push((name.clone(), x));
        }
        self.schemas.insert(name.to_owned(), row);
        Ok(())
    }
    fn int(&mut self, n: i128, want: Option<&Type>) -> R<Type> {
        match want.map(|w| self.cx.resolve(w)) {
            Some(Type::Prim(Prim::N)) if n < 0 => {
                Err(self.err("negative integer literal cannot have type n"))
            }
            Some(Type::Prim(t)) if num(t) => Ok(p(t)),
            _ => {
                let t = self.cx.fresh();
                self.req(Class::Num, &t)?;
                let end = self
                    .active
                    .as_ref()
                    .filter(|s| s.start == self.start)
                    .map_or(self.start, |s| s.end);
                self.literals.push((t.clone(), n, self.start, end));
                Ok(t)
            }
        }
    }
    fn default_literals(&mut self, from: usize) -> R<()> {
        let entries = self.literals[from..].to_vec();
        for (t, n, start, end) in entries {
            match self.cx.resolve(&t) {
                Type::Var(_) => self.unify(&t, &p(Prim::Z))?,
                Type::Prim(Prim::N) if n < 0 => {
                    return Err(InferError {
                        kind: InferErrorKind::TypeError,
                        start,
                        end,
                        message: "negative integer literal cannot have type n".into(),
                    })
                }
                _ => {}
            }
        }
        Ok(())
    }
    fn lookup(&mut self, env: &Env, n: &str) -> R<Type> {
        let sc = env
            .get(n)
            .ok_or_else(|| self.err(format!("unknown value {n}")))?;
        let (t, cs) = self.cx.instantiate(sc);
        for c in cs {
            self.req(c.class, &c.ty)?;
        }
        Ok(t)
    }
    fn expr(&mut self, env: &Env, e: &Expr, want: Option<&Type>, effects: bool) -> R<Type> {
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
            Expr::Int(n) => self.int(*n, want)?,
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
            Expr::Name(n) => self.lookup(env, n)?,
            Expr::Ref(n) => {
                if env.contains_key(n) {
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
                Type::Tuple(
                    es.iter()
                        .enumerate()
                        .map(|(i, x)| self.expr(env, x, xs.map(|ts| &ts[i]), effects))
                        .collect::<crate::infer::R<Vec<_>>>()?,
                )
            }
            Expr::Array(es) => {
                let element = match want.map(|t| self.cx.resolve(t)) {
                    Some(Type::Seq(t)) => *t,
                    _ => self.cx.fresh(),
                };
                for e in es {
                    self.expr(env, e, Some(&element), effects)?;
                }
                s(element)
            }
            Expr::Record(fs) | Expr::NamedRecord(_, fs) => {
                let expected = match e {
                    Expr::NamedRecord(n, _) => Some(
                        self.schemas
                            .get(n)
                            .cloned()
                            .ok_or_else(|| self.err(format!("unknown schema {n}")))?,
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
                if let Expr::NamedRecord(n, _) = e {
                    let typ = Type::Record(expected.expect("nominal schema"));
                    self.unify(&Type::Record(Row { fields, tail: None }), &typ)?;
                    Type::Nominal(n.clone())
                } else {
                    Type::Record(Row { fields, tail: None })
                }
            }
            Expr::Cases(cases, alt) => {
                let t = want.cloned().unwrap_or_else(|| self.cx.fresh());
                for (cond, body) in cases {
                    self.expr(env, cond, Some(&p(B)), effects)?;
                    self.expr(env, body, Some(&t), effects)?;
                }
                self.expr(env, alt, Some(&t), effects)?;
                t
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
                self.expr(&nested, body, Some(&sig.out), false)?;
                t
            }
            Expr::Block(defs, tail) | Expr::Action(defs, tail) => {
                let action = matches!(e, Expr::Action(..));
                let desired = if action {
                    match want.map(|x| self.cx.resolve(x)) {
                        Some(Type::Action(t)) => Some(*t),
                        _ => None,
                    }
                } else {
                    want.cloned()
                };
                let mut nested = env.clone();
                self.local_defs(defs, &mut nested, action)?;
                let t = if let Some(ex) = tail {
                    self.expr(&nested, ex, desired.as_ref(), action)?
                } else {
                    p(Unit)
                };
                if action {
                    a(t)
                } else {
                    t
                }
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
                for (i, ex) in tail.iter().enumerate() {
                    nested.insert("$".into(), mono(t.clone()));
                    t = self.expr(
                        &nested,
                        ex,
                        if i + 1 == tail.len() { want } else { None },
                        effects,
                    )?;
                }
                t
            }
            Expr::Call(n, args) => {
                if env.contains_key(n) {
                    let callee = self.lookup(env, n)?;
                    self.apply(env, callee, args, want, effects)?
                } else {
                    self.builtin_call(env, e, n, args, want, effects)?
                }
            }
        };
        if let Some(w) = want {
            self.unify(&t, w)?;
        }
        self.result.expressions.insert(address(e), t.clone());
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
        for (i, ex) in args.iter().enumerate() {
            let arg = self.cx.fresh();
            let out = if i + 1 == args.len() {
                want.cloned().unwrap_or_else(|| self.cx.fresh())
            } else {
                self.cx.fresh()
            };
            self.unify(&callee, &f(arg.clone(), out.clone()))?;
            self.expr(env, ex, Some(&arg), effects)?;
            callee = out;
        }
        Ok(callee)
    }
    fn select(&mut self, sel: &str, src: &Type) -> R<Type> {
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
                .ok_or_else(|| self.err(format!("schema {n} has no field {sel}"))),
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
    fn require_exact_numeric(&mut self, t: &Type, n: &str) -> R<()> {
        let t = self.cx.resolve(t);
        match t {
            Type::Prim(p) if exact(p) => Ok(()),
            Type::Var(_)
                if self
                    .literals
                    .iter()
                    .rev()
                    .any(|(lit, _, _, _)| self.cx.resolve(lit) == t) =>
            {
                self.unify(&t, &p(Prim::Z))
            }
            Type::Var(_) => {
                Err(self.err(format!("ambiguous {n}: supply an exact numeric annotation")))
            }
            _ => Err(self.err(format!("{n} expects an exact numeric type, found {t}"))),
        }
    }
    fn numeric_binary(&mut self, n: &str, ta: &Type, tb: &Type, out: Option<&Type>) -> R<Type> {
        use Prim::*;
        let ta = self.cx.resolve(ta);
        let tb = self.cx.resolve(tb);
        if [&ta, &tb]
            .iter()
            .any(|t| matches!(t, Type::Seq(el) if matches!(el.as_ref(), Type::Seq(_))))
        {
            return Err(self.err("numeric lifting is limited to one sequence dimension"));
        }
        let out = out.map(|t| self.cx.resolve(t));
        if let Type::Seq(a) = &ta {
            let se = match &out {
                Some(Type::Seq(t)) => Some(t.as_ref()),
                _ => None,
            };
            let b = match &tb {
                Type::Seq(b) => b.as_ref(),
                _ => &tb,
            };
            return Ok(s(self.numeric_binary(n, a, b, se)?));
        }
        if let Type::Seq(b) = &tb {
            let se = match &out {
                Some(Type::Seq(t)) => Some(t.as_ref()),
                _ => None,
            };
            return Ok(s(self.numeric_binary(n, &ta, b, se)?));
        }
        if n == "^" {
            self.unify(&tb, &p(N))?;
            self.req(Class::Num, &ta)?;
            return Ok(ta);
        }
        self.req(Class::Num, &ta)?;
        self.req(Class::Num, &tb)?;
        let k = match (&ta, &tb) {
            (Type::Prim(a), Type::Prim(b)) => p(solve_numeric(*a, *b)
                .ok_or_else(|| self.err("r does not implicitly mix with exact numerical types"))?),
            (Type::Var(_), Type::Prim(_)) => {
                self.unify(&ta, &tb)?;
                tb.clone()
            }
            (Type::Prim(_), Type::Var(_)) => {
                self.unify(&tb, &ta)?;
                ta.clone()
            }
            (Type::Var(_), Type::Var(_)) => {
                self.unify(&ta, &tb)?;
                if let Some(t @ Type::Prim(_)) = out.as_ref() {
                    if n != "/" || t == &p(R) {
                        self.unify(&ta, t)?;
                    }
                }
                ta.clone()
            }
            _ => return Err(self.err("expected numerical operands")),
        };
        let k = self.cx.resolve(&k);
        if matches!(n, "//" | "%" | "..") {
            match k {
                Type::Prim(N | Z) => {}
                Type::Var(_) => self.unify(&k, &p(Z))?,
                _ => return Err(self.err(format!("{n} requires integral operands"))),
            }
        }
        Ok(if n == "/" {
            if k == p(R) {
                p(R)
            } else {
                self.require_exact_numeric(&k, "division")?;
                p(Q)
            }
        } else {
            k
        })
    }
    fn binary_rel(&mut self, n: &str, a: &Type, b: &Type) -> R<Type> {
        let at = self.cx.resolve(a);
        let bt = self.cx.resolve(b);
        if n != "==" {
            if [&at, &bt]
                .iter()
                .any(|t| matches!(t, Type::Seq(el) if matches!(el.as_ref(), Type::Seq(_))))
            {
                return Err(self.err("elementwise comparison is limited to one sequence dimension; use == for nested structural equality"));
            }
            match (&at, &bt) {
                (Type::Seq(a), Type::Seq(b)) => return Ok(s(self.binary_rel(n, a, b)?)),
                (Type::Seq(a), b) => return Ok(s(self.binary_rel(n, a, b)?)),
                (a, Type::Seq(b)) => return Ok(s(self.binary_rel(n, a, b)?)),
                _ => {}
            }
        }
        let k = if let (Type::Prim(a), Type::Prim(b)) = (&at, &bt) {
            if num(*a) && num(*b) {
                p(solve_numeric(*a, *b).ok_or_else(|| {
                    self.err("r does not implicitly mix with exact numerical types")
                })?)
            } else {
                self.unify(&at, &bt)?;
                at
            }
        } else {
            self.unify(&at, &bt)?;
            at
        };
        self.req(
            if matches!(n, "<" | "<=" | ">" | ">=") {
                Class::Ord
            } else {
                Class::Eq
            },
            &k,
        )?;
        Ok(p(Prim::B))
    }
    fn unary_fn(
        &mut self,
        env: &Env,
        ex: &Expr,
        arg: &Type,
        want: Option<&Type>,
        effect: bool,
    ) -> R<Type> {
        let out = want.cloned().unwrap_or_else(|| self.cx.fresh());
        self.expr(env, ex, Some(&f(arg.clone(), out.clone())), effect)?;
        Ok(out)
    }
    fn collection(&mut self, t: &Type) -> R<Type> {
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
    fn seq_element(&mut self, t: &Type) -> R<Type> {
        let out = self.cx.fresh();
        self.unify(t, &s(out.clone()))?;
        Ok(out)
    }
    fn row(&self, t: &Type) -> R<Row> {
        match self.cx.resolve(t) {
            Type::Record(r) => Ok(r),
            Type::Nominal(n) => self
                .schemas
                .get(&n)
                .cloned()
                .ok_or_else(|| self.err(format!("unknown schema {n}"))),
            x => Err(self.err(format!("expected a record, found {x}"))),
        }
    }
    fn builtin_ref(&mut self, n: &str, want: Option<&Type>) -> R<Type> {
        use Prim::*;
        if n.starts_with('.') && n != ".:" && n != ".." {
            let src = self.cx.fresh();
            let out = self.cx.fresh();
            let t = f(src.clone(), out.clone());
            if let Some(w) = want {
                self.unify(&t, w)?;
            }
            let actual = self.select(&n[1..], &src)?;
            self.unify(&out, &actual)?;
            return Ok(t);
        }
        if let Some(w) = want {
            let t = self.cx.resolve(w);
            if let Type::Fun(..) = t {
                let mut cursor = t.clone();
                let mut args = Vec::new();
                while let Type::Fun(a, b) = cursor {
                    args.push(*a);
                    cursor = *b;
                }
                let ret = self.builtin_types(n, &args, Some(&cursor))?;
                self.unify(&ret, &cursor)?;
                return Ok(w.clone());
            }
        }
        let x = self.cx.fresh();
        Ok(match n {
            "+" | "-" | "*" => {
                self.req(Class::Num, &x)?;
                f(x.clone(), f(x.clone(), x))
            }
            "=" | "/=" | "==" => {
                self.req(Class::Eq, &x)?;
                f(x.clone(), f(x, p(B)))
            }
            "<" | "<=" | ">" | ">=" => {
                self.req(Class::Ord, &x)?;
                f(x.clone(), f(x, p(B)))
            }
            "&&" | "||" => f(p(B), f(p(B), p(B))),
            "?+" => f(o(x), p(B)),
            "?!" => f(o(x.clone()), x),
            "io.print" | "io.println" => f(x, a(p(Unit))),
            "?1" => f(x.clone(), o(x)),
            "+/" | "*/" => {
                self.req(Class::Num, &x)?;
                f(s(x.clone()), x)
            }
            _ => {
                return Err(self.unsupported(format!(
                    "builtin reference &{n} requires a contextual function type"
                )))
            }
        })
    }
    fn builtin_call(
        &mut self,
        env: &Env,
        call: &Expr,
        n: &str,
        args: &[Expr],
        want: Option<&Type>,
        effect: bool,
    ) -> R<Type> {
        use Prim::*;
        let canonical = crate::builtins::lookup(n).map_or(n, |b| b.canonical);
        let n = canonical;
        let result = match n {
            "@" => {
                let arg = self.cx.fresh();
                let out = want.cloned().unwrap_or_else(|| self.cx.fresh());
                // Bind the operand first so selector references see tuple/row shape.
                self.expr(env, &args[1], Some(&arg), effect)?;
                self.expr(env, &args[0], Some(&f(arg, out.clone())), effect)?;
                out
            }
            "@@" => {
                let tuple = self.expr(env, &args[1], None, effect)?;
                let Type::Tuple(xs) = self.cx.resolve(&tuple) else {
                    return Err(self.err("@@ requires a tuple bundle"));
                };
                let out = want.cloned().unwrap_or_else(|| self.cx.fresh());
                self.expr(
                    env,
                    &args[0],
                    Some(&function(xs.into_iter(), out.clone())),
                    effect,
                )?;
                out
            }
            "@:" | "?:" | "=:" | ">:" | "<:" => {
                let src = self.expr(env, &args[1], None, effect)?;
                let res = self.cx.resolve(&src);
                let elt = match &res {
                    Type::Dict(_, t) if n == "@:" => t.as_ref().clone(),
                    _ => self.collection(&src)?,
                };
                match (&res, n) {
                    (Type::Dict(k, _), "@:") => {
                        let result = self.unary_fn(env, &args[0], &elt, None, effect)?;
                        Type::Dict(k.clone(), b(result))
                    }
                    (_, "@:") => {
                        let result = self.unary_fn(env, &args[0], &elt, None, effect)?;
                        s(result)
                    }
                    (_, "?:") => {
                        let mask = matches!(&args[0], Expr::Array(_))
                            || matches!(&args[0],Expr::Name(n) if env.get(n).is_some_and(|sc|matches!(self.cx.resolve(&sc.ty),Type::Seq(_))))
                            || matches!(&args[0],Expr::Call(n,_) if matches!(n.as_str(),"<"|"<="|">"|">="|"="|"/="|"&&"|"||")||n.starts_with('.'));
                        if mask {
                            self.expr(env, &args[0], Some(&s(p(B))), effect)?;
                        } else {
                            self.unary_fn(env, &args[0], &elt, Some(&p(B)), effect)?;
                        }
                        src
                    }
                    (_, "=:") => {
                        let k = self.unary_fn(env, &args[0], &elt, None, effect)?;
                        self.req(Class::Hash, &k)?;
                        Type::Dict(b(k), b(src))
                    }
                    _ => {
                        let k = self.unary_fn(env, &args[0], &elt, None, effect)?;
                        self.req(Class::Ord, &k)?;
                        src
                    }
                }
            }
            "df.agg" => {
                let src = self.expr(env, &args[2], None, effect)?;
                let Type::Table(row) = self.cx.resolve(&src) else {
                    return Err(self.err("df.agg expects a table"));
                };
                let key = self.unary_fn(env, &args[0], &row, None, effect)?;
                self.req(Class::Hash, &key)?;
                let summary = match want.map(|x| self.cx.resolve(x)) {
                    Some(Type::Table(row)) => *row,
                    _ => self.cx.fresh(),
                };
                self.expr(
                    env,
                    &args[1],
                    Some(&f(key, f(src, summary.clone()))),
                    effect,
                )?;
                self.row(&summary)?;
                Type::Table(b(summary))
            }
            "df.extend" => {
                let src = self.expr(env, &args[1], None, effect)?;
                let Type::Table(row) = self.cx.resolve(&src) else {
                    return Err(self.err("df.extend expects a table"));
                };
                let mut out = self.row(&row)?;
                let columns = self.expr(env, &args[0], None, effect)?;
                let columns = self.row(&columns)?;
                if columns.fields.is_empty() {
                    return Err(self.err("df.extend requires at least one new column"));
                }
                for (n, t) in columns.fields {
                    if out.fields.iter().any(|(k, _)| k == &n) {
                        return Err(
                            self.err(format!("df.extend cannot replace existing column {n}"))
                        );
                    }
                    out.fields.push((n, self.seq_element(&t)?));
                }
                Type::Table(b(Type::Record(out)))
            }
            "#[" => {
                let Expr::Array(rows) = &args[0] else {
                    return Err(self.err("expected table rows"));
                };
                let row = match want.map(|x| self.cx.resolve(x)) {
                    Some(Type::Table(row)) => *row,
                    _ => self.cx.fresh(),
                };
                for ex in rows {
                    self.expr(env, ex, Some(&row), effect)?;
                }
                self.result
                    .expressions
                    .insert(address(&args[0]), s(row.clone()));
                match self.cx.resolve(&row) {
                    Type::Record(_) | Type::Nominal(_) | Type::Tuple(_) => {}
                    _ => return Err(self.err("table rows need a tuple or record schema")),
                }
                Type::Table(b(row))
            }
            "#:" => {
                let target = match want.map(|x| self.cx.resolve(x)) {
                    Some(Type::Table(row)) => Some(*row),
                    _ => None,
                };
                let struct_want = match target.as_ref() {
                    Some(Type::Record(r)) if matches!(&args[0], Expr::Record(_)) => {
                        Some(Type::Record(Row {
                            fields: r
                                .fields
                                .iter()
                                .map(|(n, t)| (n.clone(), s(t.clone())))
                                .collect(),
                            tail: r.tail,
                        }))
                    }
                    _ => None,
                };
                let src = self.expr(env, &args[0], struct_want.as_ref(), effect)?;
                let r = match self.cx.resolve(&src) {
                    Type::Seq(t) => match self.cx.resolve(&t) {
                        Type::Record(_) | Type::Nominal(_) | Type::Tuple(_) => *t,
                        _ => return Err(self.err("#: expects tuple or record rows")),
                    },
                    Type::Nominal(_) | Type::Record(_) => {
                        let cols = self.row(&src)?;
                        if cols.fields.is_empty() {
                            return Err(self.err("#: requires at least one column"));
                        }
                        let mut fields = Vec::new();
                        for (name, t) in cols.fields {
                            fields.push((name, self.seq_element(&t)?));
                        }
                        Type::Record(Row { fields, tail: None })
                    }
                    _ => return Err(self.err("#: expects row sequence or record of columns")),
                };
                Type::Table(b(r))
            }
            "!" => {
                if !effect {
                    return Err(self.err("cannot execute an action in a pure context"));
                }
                let out = want.cloned().unwrap_or_else(|| self.cx.fresh());
                self.expr(env, &args[0], Some(&a(out.clone())), effect)?;
                out
            }
            "csv.read" => {
                self.expr(env, &args[0], Some(&p(S)), effect)?;
                match want.map(|t| self.cx.resolve(t)) {
                    Some(Type::Action(out)) if matches!(out.as_ref(), Type::Table(_)) => {
                        let Type::Table(row) = out.as_ref() else {
                            unreachable!()
                        };
                        self.csv_row(row)?;
                        a(*out)
                    }
                    _ => a(s(s(p(S)))),
                }
            }
            "csv.write" => {
                self.expr(env, &args[0], Some(&p(S)), effect)?;
                let tt = self.expr(env, &args[1], None, effect)?;
                let Type::Table(row) = self.cx.resolve(&tt) else {
                    return Err(self.err("csv.write expects a named or structural table"));
                };
                self.csv_row(&row)?;
                a(p(Unit))
            }
            "?0" => {
                let x = self.cx.fresh();
                o(x)
            }
            "?1" => {
                let x = match want.map(|t| self.cx.resolve(t)) {
                    Some(Type::Optional(x)) => *x,
                    _ => self.cx.fresh(),
                };
                self.expr(env, &args[0], Some(&x), effect)?;
                o(x)
            }
            "??" => {
                let t = want.cloned().unwrap_or_else(|| self.cx.fresh());
                self.expr(env, &args[1], Some(&o(t.clone())), effect)?;
                self.expr(env, &args[0], Some(&t), effect)?;
                t
            }
            ".:" => {
                let t = self.expr(env, &args[1], None, effect)?;
                match self.cx.resolve(&t) {
                    Type::Dict(k, v) => {
                        self.expr(env, &args[0], Some(&k), effect)?;
                        o(*v)
                    }
                    Type::Seq(t) | Type::Table(t) => {
                        self.expr(env, &args[0], Some(&p(N)), effect)?;
                        o(*t)
                    }
                    x => return Err(self.err(format!("cannot index {x}"))),
                }
            }
            "/:" | "\\:" => {
                let acc = self.expr(env, &args[1], if n == "/:" { want } else { None }, effect)?;
                let seq = self.expr(env, &args[2], None, effect)?;
                let item = self.seq_element(&seq)?;
                self.expr(
                    env,
                    &args[0],
                    Some(&f(acc.clone(), f(item, acc.clone()))),
                    effect,
                )?;
                if n == "/:" {
                    acc
                } else {
                    s(acc)
                }
            }
            _ => {
                let xs = args
                    .iter()
                    .enumerate()
                    .map(|(i, x)| {
                        let context = match (n, i) {
                            ("^", 1) | ("take" | "[:" | "drop", 0) | ("slice", 0 | 1) => Some(p(N)),
                            _ => None,
                        };
                        self.expr(env, x, context.as_ref(), effect)
                    })
                    .collect::<crate::infer::R<Vec<_>>>()?;
                self.builtin_types(n, &xs, want)?
            }
        };
        self.result.operations.insert(
            address(call),
            if matches!(n, "+" | "-" | "*" | "/" | "//" | "%" | "^" | "-:" | "|") {
                Operation::Numeric {
                    operator: n.to_owned(),
                    result: result.clone(),
                }
            } else {
                Operation::Builtin(n.to_owned())
            },
        );
        Ok(result)
    }
    fn csv_row(&mut self, t: &Type) -> R<()> {
        let r = self.row(t)?;
        if r.fields.is_empty() || r.tail.is_some() {
            return Err(self.err("CSV requires a closed nonempty row schema"));
        }
        for (name, t) in r.fields {
            let t = self.cx.resolve(&t);
            let t = if let Type::Optional(t) = t { *t } else { t };
            if !matches!(
                t,
                Type::Prim(Prim::N | Prim::Z | Prim::Q | Prim::R | Prim::B | Prim::S | Prim::C)
            ) {
                return Err(self.err(format!("unsupported CSV column type for {name}: {t}")));
            }
        }
        Ok(())
    }
    fn builtin_types(&mut self, n: &str, xs: &[Type], want: Option<&Type>) -> R<Type> {
        use Prim::*;
        let arity = crate::builtins::lookup(n)
            .map(|x| x.arity)
            .or_else(|| n.starts_with('.').then_some(1));
        if let Some(arity) = arity {
            if xs.len() != arity {
                return Err(self.err(format!("{n} expects {arity} operands")));
            }
        }
        Ok(match n {
            "+" | "-" | "*" | "/" | "//" | "%" | "^" => {
                self.numeric_binary(n, &xs[0], &xs[1], want)?
            }
            ".." => {
                if xs
                    .iter()
                    .any(|t| matches!(self.cx.resolve(t), Type::Seq(_)))
                {
                    return Err(self.err("range bounds must be scalar exact integers"));
                }
                let w = match want.map(|t| self.cx.resolve(t)) {
                    Some(Type::Seq(t)) => Some(*t),
                    _ => None,
                };
                s(self.numeric_binary(n, &xs[0], &xs[1], w.as_ref())?)
            }
            "<" | "<=" | ">" | ">=" | "=" | "/=" | "==" => self.binary_rel(n, &xs[0], &xs[1])?,
            "~" => {
                self.unify(&xs[0], &p(B))?;
                p(B)
            }
            "&&" | "||" => {
                fn bool_shape(t: &Type) -> Type {
                    if let Type::Seq(t) = t {
                        s(bool_shape(t))
                    } else {
                        p(Prim::B)
                    }
                }
                let a = self.cx.resolve(&xs[0]);
                let b = self.cx.resolve(&xs[1]);
                if [&a, &b]
                    .iter()
                    .any(|t| matches!(t, Type::Seq(el) if matches!(el.as_ref(), Type::Seq(_))))
                {
                    return Err(self.err("logical lifting is limited to one sequence dimension"));
                }
                let ash = bool_shape(&a);
                let bsh = bool_shape(&b);
                self.unify(&a, &ash)?;
                self.unify(&b, &bsh)?;
                fn depth(t: &Type) -> usize {
                    if let Type::Seq(t) = t {
                        1 + depth(t)
                    } else {
                        0
                    }
                }
                if depth(&ash) >= depth(&bsh) {
                    ash
                } else {
                    bsh
                }
            }
            "-:" | "|" | "^/" => {
                self.req(Class::Num, &xs[0])?;
                match (n, self.cx.resolve(&xs[0])) {
                    ("^/", _) => p(R),
                    ("-:", Type::Prim(N)) => p(Z),
                    ("-:", t @ Type::Var(_)) => {
                        self.require_exact_numeric(&t, "negation")?;
                        self.cx.resolve(&t)
                    }
                    (_, t) => t,
                }
            }
            ":n" | ":z" | ":q" | ":d" | ":r" => {
                self.req(Class::Num, &xs[0])?;
                if n == ":d" {
                    return Err(
                        self.err("decimal conversion is not executable in the reference subset")
                    );
                }
                p(match n {
                    ":n" => N,
                    ":z" => Z,
                    ":q" => Q,
                    ":d" => D,
                    _ => R,
                })
            }
            "#" => match self.cx.resolve(&xs[0]) {
                Type::Seq(_) | Type::Table(_) | Type::Dict(..) | Type::Prim(S) => p(N),
                Type::Var(_) => {
                    return Err(self.unsupported(
                        "cardinality on an unknown collection shape requires annotation",
                    ))
                }
                t => return Err(self.err(format!("cannot take cardinality of {t}"))),
            },
            "+/" | "*/" | "avg" | "med" | "var" | "sd" | ">/" | "</" | "&&/" | "||/" => {
                let elt = self.seq_element(&xs[0])?;
                if matches!(n, "&&/" | "||/") {
                    self.unify(&elt, &p(B))?;
                    p(B)
                } else {
                    self.req(Class::Num, &elt)?;
                    match n {
                        "avg" | "med" | "var" => {
                            self.require_exact_numeric(&elt, n)?;
                            o(p(Q))
                        }
                        "sd" => o(p(R)),
                        ">/" | "</" => o(elt),
                        _ => elt,
                    }
                }
            }
            "pct" => {
                self.require_exact_numeric(&xs[0], n)?;
                let t = self.seq_element(&xs[1])?;
                self.require_exact_numeric(&t, n)?;
                o(p(Q))
            }
            "?+" | "?!" => {
                let t = self.cx.fresh();
                self.unify(&xs[0], &o(t.clone()))?;
                if n == "?+" {
                    p(B)
                } else {
                    t
                }
            }
            "io.print" | "io.println" => a(p(Unit)),
            "take" | "[:" | "drop" | "slice" | "rev" => {
                let (ns, last) = xs.split_at(xs.len() - 1);
                for i in ns {
                    self.unify(i, &p(N))?;
                }
                match self.cx.resolve(&last[0]) {
                    t @ (Type::Seq(_) | Type::Table(_) | Type::Prim(S)) => t,
                    Type::Var(_) => {
                        return Err(self.unsupported(format!("{n} needs a known collection shape")))
                    }
                    t => {
                        return Err(
                            self.err(format!("{n} expects sequence, table, or string; found {t}"))
                        )
                    }
                }
            }
            "flat" | "tr" => {
                let el = self.cx.fresh();
                self.unify(&xs[0], &s(s(el.clone())))?;
                if n == "flat" {
                    s(el)
                } else {
                    s(s(el))
                }
            }
            "=." => {
                let el = self.seq_element(&xs[0])?;
                self.req(Class::Eq, &el)?;
                s(el)
            }
            ":#" => match self.cx.resolve(&xs[0]) {
                Type::Table(row) => s(*row),
                x => return Err(self.err(format!(":# expects a table, found {x}"))),
            },
            "+:" => {
                self.unify(&xs[0], &xs[1])?;
                self.collection(&xs[0])?;
                xs[0].clone()
            }
            "&:" => {
                let a = self.seq_element(&xs[0])?;
                let b = self.seq_element(&xs[1])?;
                s(Type::Tuple(vec![a, b]))
            }
            "*:" => match (self.cx.resolve(&xs[0]), self.cx.resolve(&xs[1])) {
                (Type::Table(a), Type::Table(b1)) => Type::Table(b(Type::Tuple(vec![*a, *b1]))),
                _ => return Err(self.err("*: expects two tables")),
            },
            n if n.starts_with('.') && n != ".:" => self.select(&n[1..], &xs[0])?,
            n => return Err(self.unsupported(format!("{n} is not covered by Draft 0.4 inference"))),
        })
    }
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
        if params.is_empty() {
            self.expr(env, body, Some(&sig.out), effect)
                .map_err(|mut e| {
                    e.message = format!("{name}: {}", e.message);
                    e
                })?;
            return Ok(());
        }
        let mut nested = env.clone();
        let ids = self.resolver.parameters(stmt);
        for (i, (name, t)) in params.iter().zip(&sig.args).enumerate() {
            nested.insert(name.clone(), mono(t.clone()));
            if let Some(id) = ids.get(i) {
                self.result.bindings.insert(*id, mono(t.clone()));
            }
        }
        self.expr(&nested, body, Some(&sig.out), effect && params.is_empty())
            .map_err(|mut e| {
                e.message = format!("{name}: {}", e.message);
                e
            })?;
        Ok(())
    }
    fn validate_signature(&self, sig: &Signature) -> R<()> {
        if sig.explicit.is_empty() {
            return Ok(());
        }
        let mut variables = HashSet::new();
        for (name, t) in &sig.explicit {
            let resolved = self.cx.resolve(t);
            if !matches!(resolved, Type::Var(_)) || !variables.insert(resolved) {
                return Err(self.err(format!("signature variable {name} was specialized; an annotated type variable must remain distinct and general")));
            }
        }
        for c in self.cx.constraints() {
            let t = self.cx.resolve(&c.ty);
            if variables.contains(&t)
                && !sig
                    .qualifiers
                    .iter()
                    .any(|q| q.class == c.class && self.cx.resolve(&q.ty) == t)
            {
                return Err(self.err(format!("annotated signature lacks {:?} {t}", c.class)));
            }
        }
        Ok(())
    }
    fn def_signature(&mut self, stmt: &Stmt) -> R<Signature> {
        let Stmt::Def {
            params,
            param_types,
            result_type,
            start,
            ..
        } = stmt
        else {
            unreachable!()
        };
        self.start = *start;
        let mut sig = self.signature(params, param_types, result_type.as_deref())?;
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
                sig.qualifiers.push(Constraint { class, ty });
            }
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
    fn group(&mut self, defs: Vec<&Stmt>, env: &mut Env, effect: bool) -> R<()> {
        // Ordinary immutable values do not add a recursive header. Infer them
        // against the existing environment instead of cloning every prior value.
        if let [stmt @ Stmt::Def {
            name,
            params,
            body,
            start,
            ..
        }] = defs.as_slice()
        {
            if params.is_empty() {
                let sig = self.def_signature(stmt)?;
                let pos = self.literals.len();
                self.body(env, stmt, &sig, effect)?;
                self.default_literals(pos)?;
                self.start = *start;
                self.validate_signature(&sig)?;
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
                Stmt::Schema {
                    name,
                    fields,
                    start,
                } => {
                    self.start = *start;
                    self.install_schema(name, fields)?;
                    if let Some(id) = self.resolver.declaration(stmt) {
                        self.result
                            .bindings
                            .insert(id, mono(Type::Nominal(name.clone())));
                    }
                }
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
                    if !effect {
                        return Err(self.err("expression statement in pure block"));
                    }
                    let t = self.expr(env, body, None, effect)?;
                    if let Some(n) = name {
                        self.bind(env, stmt, n, mono(t));
                    }
                }
            }
        }
        for (stmt, sig) in &sigs {
            self.body(env, stmt, sig, effect)?;
        }
        self.default_literals(literals)?;
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
            self.validate_signature(&sig)?;
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
    fn local_defs(&mut self, defs: &[Stmt], env: &mut Env, effect: bool) -> R<()> {
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
                Stmt::Schema {
                    name,
                    fields,
                    start,
                } => {
                    self.start = *start;
                    self.install_schema(name, fields)?;
                }
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
        let mut pending: Vec<(SymbolId, &Stmt, Signature)> = Vec::new();
        let mut checked = HashSet::new();
        for stmt in defs {
            match stmt {
                Stmt::Def { params, .. } if !params.is_empty() => {
                    let id = self.resolver.declaration(stmt).expect("local symbol");
                    let sig = signatures.remove(&id).expect("local header signature");
                    let pos = self.literals.len();
                    self.body(env, stmt, &sig, effect)?;
                    self.default_literals(pos)?;
                    checked.insert(id);
                    pending.push((id, stmt, sig));
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
                        for (_, stmt, sig) in done {
                            let Stmt::Def { name, start, .. } = stmt else {
                                unreachable!()
                            };
                            self.start = *start;
                            self.validate_signature(&sig)?;
                            let t = function(sig.args.into_iter(), sig.out);
                            let sc = self.scheme(&visible, &t, true);
                            self.bind(env, stmt, name, sc);
                        }
                    }
                }
                Stmt::Schema { .. } => {}
                _ => self.group(vec![stmt], env, effect)?,
            }
        }
        Ok(())
    }
}
pub fn infer(p: &Program, resolver: &Resolution) -> R<InferResult> {
    let mut e = Engine {
        cx: InferCtx::new(),
        result: InferResult::default(),
        resolver,
        schemas: HashMap::new(),
        start: 0,
        literals: vec![],
        active: None,
        qualifiers: &p.constraints,
    };
    let mut env = Env::new();
    // Header names make forward nominal schemas visible without pretending their fields exist.
    for stmt in &p.defs {
        if let Stmt::Schema { name, .. } = stmt {
            e.schemas.insert(
                name.clone(),
                Row {
                    fields: vec![],
                    tail: None,
                },
            );
        }
    }
    for stmt in &p.defs {
        if let Stmt::Schema {
            name,
            fields,
            start,
        } = stmt
        {
            e.start = *start;
            e.install_schema(name, fields)?;
        }
    }
    for group in &resolver.groups {
        e.group(group.iter().map(|i| &p.defs[*i]).collect(), &mut env, false)?;
    }
    if let Some(entry) = &p.entry {
        e.start = p.entry_start.unwrap_or(0);
        let pos = e.literals.len();
        e.expr(&env, entry, None, false)?;
        e.default_literals(pos)?;
    }
    for typ in e.result.expressions.values_mut() {
        *typ = e.cx.resolve(typ);
    }
    for scheme in e.result.bindings.values_mut() {
        scheme.ty = e.cx.resolve(&scheme.ty);
        for c in &mut scheme.constraints {
            c.ty = e.cx.resolve(&c.ty);
        }
    }
    for op in e.result.operations.values_mut() {
        if let Operation::Numeric { result, .. } = op {
            *result = e.cx.resolve(result);
        }
    }
    for (name, row) in e.schemas {
        let Type::Record(row) = e.cx.resolve(&Type::Record(row)) else {
            unreachable!()
        };
        e.result.schemas.insert(name, row);
    }
    Ok(e.result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn check(source: &str) -> R<InferResult> {
        let p = crate::parser::parse(source).unwrap();
        let names = crate::resolve::resolve(&p).unwrap();
        infer(&p, &names)
    }
    #[test]
    fn independent_generic_instantiations_and_recursive_scc() {
        check(
            r#"id(x) := x; a := id 1; b := id "x";
          even(x:n):b := ? (= x 0) 1b (odd (- x 1));
          odd(x:n):b := ? (= x 0) 0b (even (- x 1));
          c := even 2;"#,
        )
        .unwrap();
        assert_eq!(
            check("loop(x) := loop [x];").unwrap_err().kind,
            InferErrorKind::TypeError
        );
    }
    #[test]
    fn higher_order_and_open_rows() {
        check(
            r#"
          apply(g,x) := @ g x;
          id(v) := v;
          value(row) := .val row;
          a := apply &id 3;
          b := value .{ label := "a"; val := 1; };
          c := value .{ val := 1.25; flag := 1b; };
        "#,
        )
        .unwrap();
        assert_eq!(
            check("value(r) := .val r; a := value .{ nope := 0; };")
                .unwrap_err()
                .kind,
            InferErrorKind::TypeError
        );
    }
    #[test]
    fn annotated_signature_constraints_and_nominal_evidence() {
        check("f(x:['a]):b | eq 'a := == x x; >> (f);").unwrap();
        let e = check("f(x:['a]):b := == x x; >> (f);").unwrap_err();
        assert_eq!(e.kind, InferErrorKind::TypeError);
        assert!(e.message.contains("lacks"));
        let e=check("schema :: { callback:z->z; }; id(x) := x; same(x) := == x x; data := schema { callback:=&id; }; b := same data;").unwrap_err();
        assert_eq!(e.kind, InferErrorKind::TypeError);
        assert!(e.message.contains("callback"), "{e:?}");
        check("schema :: { count:z; }; same(x) := == x x; data := schema { count:=2; }; b := same data;").unwrap();
    }
    #[test]
    fn value_restriction_and_empty_ambiguity() {
        assert!(check(r#"id(x) := x; x := ? 1b &id &id; a := @ x 1; b := @ x "s";"#).is_err());
        assert!(check("empty := [];").is_err());
        check("empty:[z] := [];").unwrap();
    }
    #[test]
    fn local_functions_generalize_after_acyclic_and_recursive_groups() {
        check(
            r#"run(u:()):() := {
            id(x) := x;
            a := id 1;
            b := id "one";
            first(x) := second x;
            second(y) := y;
            c := first 1;
            d := first "two";
            ()
        };"#,
        )
        .unwrap();
    }
    #[test]
    fn diagnostics_have_the_expression_span_when_resolved() {
        let source = "good := 1;\ninvalid := * 2 0b;";
        let e = check(source).unwrap_err();
        assert_eq!(e.start, source.find('*').unwrap());
        assert_eq!(e.end, source.len() - 1);
    }
}
