//! Static semantics of the Flux reference subset.
use crate::{
    parser::{Expr, Program, Stmt},
    Error,
};
use std::{
    collections::{HashMap, HashSet},
    fmt,
};

type R<T> = Result<T, String>;
#[derive(Clone, Debug, PartialEq, Eq)]
enum T {
    N,
    Z,
    Q,
    Approx,
    B,
    S,
    C,
    Unit,
    Seq(Box<T>),
    Tuple(Vec<T>),
    Opt(Box<T>),
    Action(Box<T>),
    Fun(Box<T>, Box<T>),
    Table(Box<T>),
    Dict(Box<T>, Box<T>),
    Record(String),
    Shape(Vec<(String, T)>),
}
use T::*;
fn bx(t: T) -> Box<T> {
    Box::new(t)
}
impl fmt::Display for T {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            N => write!(f, "n"),
            Z => write!(f, "z"),
            Q => write!(f, "q"),
            Approx => write!(f, "r"),
            B => write!(f, "b"),
            S => write!(f, "s"),
            C => write!(f, "c"),
            Unit => write!(f, "()"),
            Seq(x) => write!(f, "[{x}]"),
            Opt(x) => write!(f, "?{x}"),
            Action(x) => write!(f, "!{x}"),
            Table(x) => write!(f, "#{x}"),
            Fun(a, b) => write!(f, "({a}->{b})"),
            Dict(k, v) => write!(f, "{{{k}:{v}}}"),
            Record(n) => write!(f, "{n}"),
            Shape(fields) => write!(
                f,
                ".{{ {} }}",
                fields
                    .iter()
                    .map(|(n, t)| format!("{n}:{t}"))
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
            Tuple(xs) => write!(
                f,
                "({})",
                xs.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        }
    }
}
#[derive(Clone)]
enum Binding {
    Value(T),
    Callable(Vec<T>, Option<T>),
}
#[derive(Clone, Default)]
struct Env {
    values: HashMap<String, Binding>,
    rows: HashMap<String, Vec<(String, T)>>,
}
fn mismatch(a: &T, b: &T) -> String {
    format!("expected {a}, found {b}")
}
fn numeric(t: &T) -> bool {
    matches!(t, N | Z | Q)
}
fn integral(t: &T) -> bool {
    matches!(t, N | Z)
}
fn join(a: T, b: T) -> R<T> {
    if a == b {
        return Ok(a);
    }
    if numeric(&a) && numeric(&b) {
        return Ok(if a == Q || b == Q { Q } else { Z });
    }
    Err(mismatch(&a, &b))
}
fn fun(ts: Vec<T>, ret: T) -> T {
    ts.into_iter()
        .rev()
        .fold(ret, |out, arg| Fun(bx(arg), bx(out)))
}
fn conforms(expected: &T, got: &T) -> bool {
    expected == got
}

fn annotation(env: &Env, text: &str) -> R<T> {
    struct TypeParser<'a> {
        tokens: Vec<crate::lexer::Token>,
        i: usize,
        env: &'a Env,
    }
    impl TypeParser<'_> {
        fn peek(&self) -> &str {
            self.tokens.get(self.i).map_or("", |t| t.text.as_str())
        }
        fn eat(&mut self, k: &str) -> bool {
            if self.peek() == k {
                self.i += 1;
                true
            } else {
                false
            }
        }
        fn end(&mut self, k: &str) -> R<()> {
            if self.eat(k) {
                Ok(())
            } else {
                Err(format!("expected {k} in type annotation"))
            }
        }
        fn atom(&mut self) -> R<T> {
            let tok = self.peek().to_string();
            if tok.is_empty() {
                return Err("expected a type".into());
            }
            self.i += 1;
            match tok.as_str() {
                "n" => Ok(N),
                "z" => Ok(Z),
                "q" => Ok(Q),
                "b" => Ok(B),
                "s" => Ok(S),
                "c" => Ok(C),
                "r" => Ok(Approx),
                "d" => Err("type d is not supported in this prototype; use exact q".into()),
                "?" => Ok(Opt(bx(self.atom()?))),
                "!" => Ok(Action(bx(self.atom()?))),
                "#" => {
                    let row = self.atom()?;
                    if !matches!(row, Tuple(_) | Record(_) | Shape(_)) {
                        return Err(format!("table type needs a row schema, found {row}"));
                    }
                    Ok(Table(bx(row)))
                }
                "[" => {
                    let t = self.ty(&["]"])?;
                    self.end("]")?;
                    Ok(Seq(bx(t)))
                }
                "#[" => {
                    let mut fields = Vec::new();
                    while !self.eat("]") {
                        fields.push(self.atom()?);
                        self.eat(",");
                    }
                    if fields.is_empty() {
                        return Err("table type needs a row schema".into());
                    }
                    Ok(Table(bx(Tuple(fields))))
                }
                "(" => {
                    if self.eat(")") {
                        return Ok(Unit);
                    }
                    let t = self.ty(&[",", ")"])?;
                    if self.eat(")") {
                        return Ok(t);
                    }
                    let mut ts = vec![t];
                    while self.eat(",") {
                        ts.push(self.ty(&[",", ")"])?);
                    }
                    self.end(")")?;
                    Ok(Tuple(ts))
                }
                ".{" => {
                    let mut ts = Vec::new();
                    while !self.eat("}") {
                        let k = self.peek().to_string();
                        if k.is_empty()
                            || !k.as_bytes().first().is_some_and(|b| b.is_ascii_lowercase())
                        {
                            return Err("expected field name in structural record type".into());
                        }
                        self.i += 1;
                        self.end(":")?;
                        ts.push((k, self.ty(&[";", "}"])?));
                        if self.eat("}") {
                            break;
                        }
                        self.end(";")?;
                    }
                    shape_type(ts)
                }
                "{" => {
                    let k = self.ty(&[":"])?;
                    self.end(":")?;
                    let v = self.ty(&["}"])?;
                    self.end("}")?;
                    Ok(Dict(bx(k), bx(v)))
                }
                _ if self.env.rows.contains_key(&tok) => Ok(Record(tok)),
                _ if tok.starts_with('\'') => {
                    Err("polymorphic type variables are not supported in this prototype".into())
                }
                _ => Err(format!("unknown or unsupported type {tok}")),
            }
        }
        fn ty(&mut self, ends: &[&str]) -> R<T> {
            let mut ts = vec![self.atom()?];
            while !self.peek().is_empty() && !ends.contains(&self.peek()) {
                if self.eat("->") {
                    return Ok(fun(ts, self.ty(ends)?));
                }
                ts.push(self.atom()?);
            }
            if ts.len() == 1 {
                Ok(ts.remove(0))
            } else {
                Err("multiple type atoms need an arrow".into())
            }
        }
    }
    TypeParser {
        tokens: crate::lexer::lex(text).map_err(|e| e.to_string())?,
        i: 0,
        env,
    }
    .ty(&[])
}

fn prep(defs: &[Stmt], env: &mut Env) -> Result<(), Error> {
    let mut local = HashSet::new();
    for stmt in defs {
        match stmt {
            Stmt::Def { name, start, .. } | Stmt::Schema { name, start, .. } => {
                if !local.insert(name) {
                    return Err(Error::new(*start, format!("duplicate binding {name}")));
                }
            }
            Stmt::Effect {
                name: Some(name),
                start,
                ..
            } => {
                if !local.insert(name) {
                    return Err(Error::new(*start, format!("duplicate binding {name}")));
                }
            }
            Stmt::Effect { name: None, .. } => {}
        }
    }
    for stmt in defs {
        if let Stmt::Schema {
            name,
            fields,
            start,
        } = stmt
        {
            if matches!(name.as_str(), "n" | "z" | "q" | "d" | "r" | "b" | "s" | "c") {
                return Err(Error::new(
                    *start,
                    format!("{name} is reserved in the type namespace"),
                ));
            }
            if env.rows.contains_key(name) {
                return Err(Error::new(
                    *start,
                    format!("schema shadowing is not supported: {name}"),
                ));
            }
            let mut names = HashSet::new();
            let mut ts = Vec::new();
            for (field, ty) in fields {
                if !names.insert(field) {
                    return Err(Error::new(
                        *start,
                        format!("duplicate field {field} in schema {name}"),
                    ));
                }
                ts.push((
                    field.clone(),
                    annotation(env, ty).map_err(|e| Error::new(*start, e))?,
                ));
            }
            env.rows.insert(name.clone(), ts);
            env.values.remove(name);
        }
    }
    for stmt in defs {
        if let Stmt::Def {
            name,
            params,
            param_types,
            result_type,
            start,
            ..
        } = stmt
        {
            if params.is_empty() {
                continue;
            }
            let mut seen = HashSet::new();
            for p in params {
                if !seen.insert(p) {
                    return Err(Error::new(*start, format!("duplicate parameter {p}")));
                }
            }
            let ts = params
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let typ = param_types
                        .get(i)
                        .and_then(Option::as_deref)
                        .ok_or_else(|| {
                            format!(
                                "parameter {p} of {name} needs a type annotation in this prototype"
                            )
                        })?;
                    annotation(env, typ)
                })
                .collect::<R<Vec<_>>>()
                .map_err(|e| Error::new(*start, e))?;
            let rt = result_type
                .as_deref()
                .map(|s| annotation(env, s))
                .transpose()
                .map_err(|e| Error::new(*start, e))?;
            env.values.insert(name.clone(), Binding::Callable(ts, rt));
        }
    }
    Ok(())
}

fn check_def(def: &Stmt, env: &mut Env, effect: bool) -> Result<(), Error> {
    match def {
        Stmt::Schema { .. } => {}
        Stmt::Effect { name, body, start } => {
            if !effect {
                return Err(Error::new(
                    *start,
                    "effect statements require an effect context",
                ));
            }
            let t = expr(env, body, None, effect).map_err(|e| Error::new(*start, e))?;
            if let Some(n) = name {
                env.values.insert(n.clone(), Binding::Value(t));
            }
        }
        Stmt::Def {
            name,
            params,
            result_type,
            body,
            start,
            ..
        } => {
            let err = |e: String| Error::new(*start, format!("{name}: {e}"));
            if params.is_empty() {
                let ann = result_type
                    .as_deref()
                    .map(|s| annotation(env, s))
                    .transpose()
                    .map_err(err)?;
                let t = expr(env, body, ann.as_ref(), effect).map_err(err)?;
                env.values.insert(name.clone(), Binding::Value(t));
            } else {
                let Some(Binding::Callable(ts, rt)) = env.values.get(name).cloned() else {
                    return Err(err("missing callable signature".into()));
                };
                let mut private = env.clone();
                for (p, t) in params.iter().zip(ts.iter()) {
                    private.values.insert(p.clone(), Binding::Value(t.clone()));
                }
                let t = expr(&private, body, rt.as_ref(), false).map_err(err)?;
                env.values
                    .insert(name.clone(), Binding::Callable(ts, Some(t)));
            }
        }
    }
    Ok(())
}

fn defs_check(defs: &[Stmt], env: &mut Env, effect: bool) -> Result<(), Error> {
    prep(defs, env)?;
    for d in defs {
        check_def(d, env, effect)?;
    }
    Ok(())
}

fn top_defs_check(defs: &[Stmt], env: &mut Env) -> Result<(), Error> {
    prep(defs, env)?;
    // Values are initialized by dependency at top-level. Try the declarations
    // again only when another declaration's type becomes known.
    let mut pending: Vec<_> = defs.iter().collect();
    while !pending.is_empty() {
        let mut remain = Vec::new();
        let mut first_error = None;
        for d in pending.iter().copied() {
            if let Err(e) = check_def(d, env, false) {
                if first_error.is_none() {
                    first_error = Some(e);
                }
                remain.push(d);
            }
        }
        if remain.len() == pending.len() {
            return Err(first_error.expect("nonempty stalled declarations"));
        }
        pending = remain;
    }
    Ok(())
}

fn contextual_num(n: i128, want: Option<&T>) -> T {
    match want {
        Some(Q) => Q,
        Some(Z) => Z,
        Some(N) if n >= 0 => N,
        _ => Z,
    }
}
fn lit(s: &str, want: Option<&T>) -> R<T> {
    let signed = s.starts_with('-');
    let frac = s.contains('.');
    match s.as_bytes().last() {
        Some(b'd') => Err("decimal d is not supported; use exact q".into()),
        Some(b'r') => match s[..s.len() - 1].parse::<f64>() {
            Ok(v) if v.is_finite() => Ok(Approx),
            _ => Err("real literal must be finite".into()),
        },
        Some(b'n') if signed || frac => {
            Err("expected a nonnegative integral natural literal".into())
        }
        Some(b'z') if frac => Err("expected an integral z literal".into()),
        Some(b'n') => Ok(N),
        Some(b'z') => Ok(Z),
        Some(b'q') => Ok(Q),
        _ if frac => Ok(Q),
        _ if signed => Ok(contextual_num(-1, want)),
        _ => Ok(contextual_num(0, want)),
    }
}
fn shape_type(fields: Vec<(String, T)>) -> R<T> {
    let mut names = HashSet::new();
    for (n, _) in &fields {
        if !names.insert(n) {
            return Err(format!("duplicate field {n}"));
        }
    }
    Ok(Shape(fields))
}
fn record_fields(env: &Env, t: &T) -> R<Vec<(String, T)>> {
    match t {
        Shape(fs) => Ok(fs.clone()),
        Record(n) => env
            .rows
            .get(n)
            .cloned()
            .ok_or_else(|| format!("unknown schema {n}")),
        _ => Err(format!("expected a record, found {t}")),
    }
}
fn column_shape(env: &Env, columns: &T) -> R<Vec<(String, T)>> {
    let cols = record_fields(env, columns)?;
    if cols.is_empty() {
        return Err("record-of-columns requires at least one column".into());
    }
    cols.into_iter()
        .map(|(n, t)| match t {
            Seq(element) => Ok((n, *element)),
            _ => Err(format!("column {n} must be a sequence, found {t}")),
        })
        .collect()
}
fn apply_two(env: &Env, f: &Expr, a: &T, b: &T, effect: bool) -> R<T> {
    match f {
        Expr::TypedLambda(ps, ts, body) => {
            lambda(env, ps, ts, body, None, Some(&[a.clone(), b.clone()]))
        }
        Expr::Lambda(ps, body) => lambda(env, ps, &[], body, None, Some(&[a.clone(), b.clone()])),
        _ => {
            let f = expr(env, f, None, effect)?;
            let Fun(p, r) = f else {
                return Err("df.agg summary must be a two-argument function".into());
            };
            if &*p != a {
                return Err(mismatch(a, &p));
            }
            let Fun(p, r) = *r else {
                return Err("df.agg summary must accept the group as its second parameter".into());
            };
            if &*p != b {
                return Err(mismatch(b, &p));
            }
            Ok(*r)
        }
    }
}
fn lambda(
    env: &Env,
    params: &[String],
    annotations: &[Option<String>],
    body: &Expr,
    want: Option<&T>,
    supplied: Option<&[T]>,
) -> R<T> {
    let mut cursor = want;
    let mut nested = env.clone();
    let mut seen = HashSet::new();
    let mut ts = Vec::new();
    if supplied.is_some_and(|ts| ts.len() != params.len()) {
        return Err("function has wrong lambda arity".into());
    }
    for (i, p) in params.iter().enumerate() {
        if !seen.insert(p) {
            return Err(format!("duplicate lambda parameter {p}"));
        }
        let contextual = match cursor {
            Some(Fun(arg, result)) => {
                cursor = Some(result);
                Some(arg.as_ref())
            }
            Some(_) => return Err("too many lambda parameters for contextual function type".into()),
            None => None,
        };
        let expected = supplied.map(|ts| &ts[i]).or(contextual);
        let given = annotations
            .get(i)
            .and_then(Option::as_deref)
            .map(|s| annotation(env, s))
            .transpose()?;
        let t = match (given, expected) {
            (Some(t), Some(e)) if &t != e => return Err(mismatch(e, &t)),
            (Some(t), _) => t,
            (None, Some(e)) => e.clone(),
            _ => {
                return Err(format!(
                    "cannot infer lambda parameter {p}; add its type annotation"
                ))
            }
        };
        nested.values.insert(p.clone(), Binding::Value(t.clone()));
        ts.push(t);
    }
    let out = expr(&nested, body, cursor, false)?;
    Ok(if supplied.is_some() {
        out
    } else {
        fun(ts, out)
    })
}
fn expr(env: &Env, e: &Expr, want: Option<&T>, effect: bool) -> R<T> {
    let t = match e {
        Expr::Int(n) => contextual_num(*n, want),
        Expr::Num(s) => lit(s, want)?,
        Expr::Str(_) => S,
        Expr::Bool(_) => B,
        Expr::Char(_) => C,
        Expr::Unit => Unit,
        Expr::Name(n) => match env.values.get(n) {
            Some(Binding::Value(t)) => t.clone(),
            Some(Binding::Callable(..)) => {
                return Err(format!(
                    "{n} is a callable; use &{n} to obtain a function value"
                ))
            }
            _ => return Err(format!("unknown value {n}")),
        },
        Expr::Ref(n) => return reference(env, n, want),
        Expr::Call(n, args) => call(env, n, args, want, effect)?,
        Expr::Cases(cases, last) => {
            // With no annotation, take a candidate from the first branch,
            // then still validate all branches, including the lazy ones.
            let mut all = cases
                .iter()
                .map(|(cond, out)| {
                    expr(env, cond, Some(&B), effect)?;
                    Ok(out)
                })
                .collect::<R<Vec<_>>>()?;
            all.push(last.as_ref());
            let t = match want {
                Some(t) => t.clone(),
                None => all
                    .iter()
                    .find_map(|e| expr(env, e, None, effect).ok())
                    .ok_or_else(|| {
                        "cannot infer conditional result; supply a result annotation".to_string()
                    })?,
            };
            for e in all {
                expr(env, e, Some(&t), effect)?;
            }
            t
        }
        Expr::Tuple(xs) => {
            let expected = match want {
                Some(Tuple(ts)) if xs.len() == ts.len() => Some(ts),
                _ => None,
            };
            Tuple(
                xs.iter()
                    .enumerate()
                    .map(|(i, x)| expr(env, x, expected.map(|ts| &ts[i]), effect))
                    .collect::<R<Vec<_>>>()?,
            )
        }
        Expr::Array(xs) => {
            let expected = match want {
                Some(Seq(t)) => Some(t.as_ref()),
                _ => None,
            };
            Seq(bx(list(env, xs, expected, effect)?))
        }
        Expr::Block(ds, x) | Expr::Action(ds, x) => {
            let action = matches!(e, Expr::Action(..));
            let (out, permitted) = if action {
                (
                    match want {
                        Some(Action(t)) => Some(t.as_ref()),
                        _ => None,
                    },
                    true,
                )
            } else {
                (want, false)
            };
            let mut nested = env.clone();
            defs_check(ds, &mut nested, permitted).map_err(|e| e.to_string())?;
            let t = if let Some(x) = x {
                expr(&nested, x, out, permitted)?
            } else {
                Unit
            };
            if action {
                Action(bx(t))
            } else {
                t
            }
        }
        Expr::Pipeline(stages) => {
            let Some((first, rest)) = stages.split_first() else {
                return Err("pipeline needs a source expression".into());
            };
            let mut ty = expr(
                env,
                first,
                if rest.is_empty() { want } else { None },
                effect,
            )?;
            let mut nested = env.clone();
            for (i, stage) in rest.iter().enumerate() {
                nested.values.insert("$".to_string(), Binding::Value(ty));
                ty = expr(
                    &nested,
                    stage,
                    if i + 1 == rest.len() { want } else { None },
                    effect,
                )
                .map_err(|e| format!("pipeline stage {}: {e}", i + 1))?;
            }
            ty
        }
        Expr::TypedLambda(ps, annotations, b) => lambda(env, ps, annotations, b, want, None)?,
        Expr::Lambda(ps, b) => {
            let mut cursor = want;
            let mut nested = env.clone();
            let mut seen = HashSet::new();
            let mut ts = Vec::new();
            for p in ps {
                if !seen.insert(p) {
                    return Err(format!("duplicate lambda parameter {p}"));
                }
                let Some(Fun(arg, out)) = cursor else {
                    return Err(
                        "cannot infer lambda parameters without a contextual function type".into(),
                    );
                };
                nested
                    .values
                    .insert(p.clone(), Binding::Value(arg.as_ref().clone()));
                ts.push(arg.as_ref().clone());
                cursor = Some(out);
            }
            fun(ts, expr(&nested, b, cursor, false)?)
        }
        Expr::NamedRecord(n, fields) => {
            if env.values.contains_key(n) {
                return Err(format!("schema {n} is shadowed by a value"));
            }
            let schema = env
                .rows
                .get(n)
                .ok_or_else(|| format!("unknown schema {n}"))?;
            let mut seen = HashSet::new();
            for (name, value) in fields {
                if !seen.insert(name) {
                    return Err(format!("duplicate field {name}"));
                }
                let (_, ty) = schema
                    .iter()
                    .find(|(f, _)| f == name)
                    .ok_or_else(|| format!("unknown field {name} on {n}"))?;
                expr(env, value, Some(ty), effect)?;
            }
            for (name, _) in schema {
                if !seen.contains(name) {
                    return Err(format!("missing field {name} on {n}"));
                }
            }
            Record(n.clone())
        }
        Expr::Record(fields) => {
            let expected = match want {
                Some(Shape(fields)) => Some(fields.as_slice()),
                _ => None,
            };
            let mut actual = Vec::new();
            for (name, value) in fields {
                let context =
                    expected.and_then(|ts| ts.iter().find(|(k, _)| k == name).map(|(_, t)| t));
                actual.push((name.clone(), expr(env, value, context, effect)?));
            }
            shape_type(actual)?
        }
    };
    if let Some(expected) = want {
        if !conforms(expected, &t) {
            return Err(mismatch(expected, &t));
        }
    }
    Ok(t)
}

fn list(env: &Env, xs: &[Expr], expected: Option<&T>, effect: bool) -> R<T> {
    let inferred = if let Some(t) = expected {
        t.clone()
    } else {
        let x = xs
            .first()
            .ok_or_else(|| "empty collection needs a type annotation".to_string())?;
        expr(env, x, None, effect)?
    };
    for x in xs {
        expr(env, x, Some(&inferred), effect)?;
    }
    Ok(inferred)
}

fn column(env: &Env, selector: &str, t: &T) -> R<T> {
    match t {
        Table(row) | Seq(row) => Ok(Seq(bx(column(env, selector, row)?))),
        Tuple(ts) => {
            let i = selector
                .parse::<usize>()
                .map_err(|_| format!("tuple selector .{selector} must be numeric"))?;
            ts.get(
                i.checked_sub(1)
                    .ok_or_else(|| "tuple selectors start at .1".to_string())?,
            )
            .cloned()
            .ok_or_else(|| format!("tuple has no position .{i}"))
        }
        Shape(fields) => fields
            .iter()
            .find(|(f, _)| f == selector)
            .map(|(_, t)| t.clone())
            .ok_or_else(|| format!("record has no field {selector}")),
        Record(n) => env
            .rows
            .get(n)
            .and_then(|ts| ts.iter().find(|(f, _)| f == selector))
            .map(|(_, t)| t.clone())
            .ok_or_else(|| format!("schema {n} has no field {selector}")),
        _ => Err(format!("cannot select .{selector} from {t}")),
    }
}

fn reference(env: &Env, n: &str, want: Option<&T>) -> R<T> {
    let t = match env.values.get(n) {
        Some(Binding::Callable(ts, Some(out))) => fun(ts.clone(), out.clone()),
        Some(Binding::Callable(..)) => {
            return Err(format!(
                "reference to {n} needs an explicit result type here"
            ))
        }
        Some(Binding::Value(_)) => {
            return Err(format!(
                "{n} names a value, not a referenceable declaration"
            ))
        }
        None => {
            let Some(ty) = want else {
                return Err(format!(
                    "cannot infer reference &{n}; supply a function type"
                ));
            };
            let mut args = Vec::new();
            let mut r = ty;
            while let Fun(a, b) = r {
                args.push(a.as_ref().clone());
                r = b.as_ref();
            }
            let actual = builtin(env, n, &args, Some(r), false)?;
            if &actual != r {
                return Err(mismatch(r, &actual));
            }
            ty.clone()
        }
    };
    if let Some(w) = want {
        if &t != w {
            return Err(mismatch(w, &t));
        }
    }
    Ok(t)
}

fn arity(n: &str, got: usize, want: usize) -> R<()> {
    if got == want {
        Ok(())
    } else {
        Err(format!("{n} expects {want} operand(s), found {got}"))
    }
}
fn invoke(env: &Env, f: &Expr, arg: &T, want: Option<&T>, effect: bool) -> R<T> {
    match f {
        Expr::Ref(n) if !env.values.contains_key(n) => {
            builtin(env, n, std::slice::from_ref(arg), want, effect)
        }
        Expr::TypedLambda(ps, ts, body) if ps.len() == 1 => {
            let inferred = want.map(|t| fun(vec![arg.clone()], t.clone()));
            let t = lambda(
                env,
                ps,
                ts,
                body,
                inferred.as_ref(),
                Some(std::slice::from_ref(arg)),
            )?;
            if let Some(w) = want {
                if !conforms(w, &t) {
                    return Err(mismatch(w, &t));
                }
            }
            Ok(t)
        }
        Expr::Lambda(ps, body) if ps.len() == 1 => {
            let mut nested = env.clone();
            nested
                .values
                .insert(ps[0].clone(), Binding::Value(arg.clone()));
            expr(&nested, body, want, false)
        }
        _ => {
            let t = expr(env, f, None, effect)?;
            let Fun(p, r) = t else {
                return Err(format!("expected a function value, found {t}"));
            };
            if p.as_ref() != arg {
                return Err(mismatch(&p, arg));
            }
            if let Some(w) = want {
                if w != r.as_ref() {
                    return Err(mismatch(w, &r));
                }
            }
            Ok(*r)
        }
    }
}

fn call(env: &Env, n: &str, args: &[Expr], want: Option<&T>, effect: bool) -> R<T> {
    if let Some(b) = env.values.get(n) {
        let Binding::Callable(ps, r) = b else {
            return Err(format!("{n} is a value; apply function values with @"));
        };
        arity(n, args.len(), ps.len())?;
        for (x, p) in args.iter().zip(ps) {
            expr(env, x, Some(p), effect)?;
        }
        return r.clone().ok_or_else(|| {
            format!("{n} needs an explicit result annotation for recursive or forward calls")
        });
    }
    match n {
        "df.extend" | "table.extend" => {
            arity(n, args.len(), 2)?;
            let source = expr(env, &args[1], None, effect)?;
            let Table(row) = source else {
                return Err("df.extend expects a table as its second operand".into());
            };
            let mut fields = record_fields(env, &row)?;
            let columns = expr(env, &args[0], None, effect)?;
            for (name, t) in column_shape(env, &columns)? {
                if fields.iter().any(|(existing, _)| existing == &name) {
                    return Err(format!("df.extend cannot replace existing column {name}"));
                }
                fields.push((name, t));
            }
            return Ok(Table(bx(shape_type(fields)?)));
        }
        "df.agg" | "table.agg" => {
            arity(n, args.len(), 3)?;
            let src = expr(env, &args[2], None, effect)?;
            let Table(row) = &src else {
                return Err("df.agg expects a table as its third operand".into());
            };
            let key = invoke(env, &args[0], row, None, effect)?;
            if !comparable(&key) {
                return Err(format!("invalid grouping key {key}"));
            }
            let summary = apply_two(env, &args[1], &key, &src, effect)?;
            record_fields(env, &summary)?;
            return Ok(Table(bx(summary)));
        }
        "#:" => {
            arity(n, args.len(), 1)?;
            if let (Expr::Record(cols), Some(Table(row))) = (&args[0], want) {
                if let Shape(fs) = row.as_ref() {
                    let field_types =
                        shape_type(fs.iter().cloned().map(|(n, t)| (n, Seq(bx(t)))).collect())?;
                    expr(env, &Expr::Record(cols.clone()), Some(&field_types), effect)?;
                    return Ok(Table(row.clone()));
                }
            }
            let t = expr(
                env,
                &args[0],
                match want {
                    Some(Table(row)) if matches!(&args[0], Expr::Array(_)) => {
                        Some(Seq(row.clone()))
                    }
                    _ => None,
                }
                .as_ref(),
                effect,
            )?;
            return builtin(env, n, &[t], want, effect);
        }
        ".:" => {
            arity(n, args.len(), 2)?;
            let context = expr(env, &args[1], None, effect)?;
            let key_type = match &context {
                Dict(k, _) => Some(k.as_ref()),
                Seq(_) | Table(_) => Some(&N),
                _ => None,
            };
            let key = expr(env, &args[0], key_type, effect)?;
            return builtin(env, n, &[key, context], want, effect);
        }
        "#[" => {
            arity(n, args.len(), 1)?;
            let Expr::Array(rows) = &args[0] else {
                return Err("table literal expected rows".into());
            };
            let t = list(
                env,
                rows,
                match want {
                    Some(Table(t)) => Some(t.as_ref()),
                    _ => None,
                },
                effect,
            )?;
            if !matches!(t, Tuple(_) | Record(_) | Shape(_)) {
                return Err("table rows must be tuples or records".into());
            }
            return Ok(Table(bx(t)));
        }
        "?0" => {
            arity(n, args.len(), 0)?;
            if let Some(Opt(t)) = want {
                return Ok(Opt(t.clone()));
            }
            return Err("?0 needs an optional type annotation".into());
        }
        "?1" => {
            arity(n, args.len(), 1)?;
            let w = match want {
                Some(Opt(t)) => Some(t.as_ref()),
                _ => None,
            };
            return Ok(Opt(bx(expr(env, &args[0], w, effect)?)));
        }
        "??" => {
            arity(n, args.len(), 2)?;
            let fallback = expr(env, &args[0], want, effect)?;
            let optional = expr(env, &args[1], Some(&Opt(bx(fallback.clone()))), effect);
            if let Err(initial) = optional {
                if let Ok(Opt(t)) = expr(env, &args[1], None, effect) {
                    expr(env, &args[0], Some(&t), effect)?;
                    return Ok(*t);
                }
                return Err(initial);
            }
            return Ok(fallback);
        }
        "@" => {
            arity(n, args.len(), 2)?;
            if !matches!(&args[0], Expr::Lambda(..))
                && !matches!(&args[0],Expr::Ref(k) if !env.values.contains_key(k))
            {
                let t = expr(env, &args[0], None, effect)?;
                let Fun(p, r) = t else {
                    return Err(format!("expected a function value, found {t}"));
                };
                expr(env, &args[1], Some(&p), effect)?;
                return Ok(*r);
            }
            let arg = expr(env, &args[1], None, effect)?;
            return invoke(env, &args[0], &arg, want, effect);
        }
        "@@" => {
            arity(n, args.len(), 2)?;
            if let Expr::Lambda(ps, body) = &args[0] {
                let t = expr(env, &args[1], None, effect)?;
                let Tuple(ts) = t else {
                    return Err("@@ expects a tuple bundle".into());
                };
                if ts.len() != ps.len() {
                    return Err("@@ tuple length does not match lambda arity".into());
                }
                let mut nested = env.clone();
                let mut seen = HashSet::new();
                for (p, t) in ps.iter().zip(ts) {
                    if !seen.insert(p) {
                        return Err(format!("duplicate lambda parameter {p}"));
                    }
                    nested.values.insert(p.clone(), Binding::Value(t));
                }
                return expr(&nested, body, want, false);
            }
            if let Expr::Ref(f) = &args[0] {
                if !env.values.contains_key(f) {
                    let t = expr(env, &args[1], None, effect)?;
                    let Tuple(ts) = t else {
                        return Err("@@ expects a tuple bundle".into());
                    };
                    return builtin(env, f, &ts, want, effect);
                }
            }
            let mut f = expr(env, &args[0], None, effect)?;
            if let Expr::Tuple(es) = &args[1] {
                for e in es {
                    let Fun(p, r) = f else {
                        return Err("@@ received more operands than the function accepts".into());
                    };
                    expr(env, e, Some(&p), effect)?;
                    f = *r;
                }
                return Ok(f);
            }
            let t = expr(env, &args[1], None, effect)?;
            let Tuple(ts) = t else {
                return Err("@@ expects a tuple bundle".into());
            };
            for actual in ts {
                let Fun(p, r) = f else {
                    return Err("@@ received more operands than the function accepts".into());
                };
                if p.as_ref() != &actual {
                    return Err(mismatch(&p, &actual));
                }
                f = *r;
            }
            return Ok(f);
        }
        "@:" | "?:" | "=:" | ">:" | "<:" => {
            arity(n, args.len(), 2)?;
            let src = expr(env, &args[1], None, effect)?;
            let item = match &src {
                Seq(a) | Table(a) => a,
                Dict(_, v) if n == "@:" => v,
                _ => return Err(format!("{n} expects a compatible collection, found {src}")),
            };
            if n == "?:" && expr(env, &args[0], Some(&Seq(bx(B))), effect).is_ok() {
                return Ok(src);
            }
            let w = if n == "?:" {
                Some(&B)
            } else {
                match want {
                    Some(Seq(t)) | Some(Dict(_, t)) if n == "@:" => Some(t.as_ref()),
                    _ => None,
                }
            };
            let ret = invoke(env, &args[0], item, w, effect)?;
            return match n {
                "?:" => Ok(src),
                "@:" => Ok(match src {
                    Dict(k, _) => Dict(k, bx(ret)),
                    _ => Seq(bx(ret)),
                }),
                "=:" if comparable(&ret) => Ok(Dict(bx(ret), bx(src))),
                ">:" | "<:" if numeric(&ret) || matches!(ret, S | C | Approx) => Ok(src),
                _ => Err(format!("invalid key type {ret} for {n}")),
            };
        }
        "/:" | "\\:" => {
            arity(n, args.len(), 3)?;
            let acc = expr(env, &args[1], if n == "/:" { want } else { None }, effect)?;
            let src = expr(env, &args[2], None, effect)?;
            let Seq(item) = src else {
                return Err(format!("{n} expects a sequence"));
            };
            let f = fun(vec![acc.clone(), item.as_ref().clone()], acc.clone());
            expr(env, &args[0], Some(&f), effect)?;
            return Ok(if n == "/:" { acc } else { Seq(bx(acc)) });
        }
        "!" => {
            arity(n, args.len(), 1)?;
            if !effect {
                return Err(
                    "cannot execute an action in a pure context; use an action block".into(),
                );
            }
            let action_context = want.map(|t| Action(bx(t.clone())));
            let t = expr(env, &args[0], action_context.as_ref(), effect)?;
            if let Action(t) = t {
                return Ok(*t);
            }
            return Err(format!("! expects an action, found {t}"));
        }
        _ => {}
    }
    let preferred_numeric = if matches!(
        n,
        "+" | "-" | "*" | "/" | "//" | "%" | ".." | ">" | ">=" | "<" | "<=" | "=" | "/="
    ) && args.len() == 2
    {
        args.iter().find_map(|x| {
            if matches!(x, Expr::Int(_)) {
                return None;
            }
            let v = expr(env, x, None, effect).ok()?;
            if numeric(&v) {
                Some(v)
            } else if let Seq(t) = v {
                numeric(&t).then_some(*t)
            } else {
                None
            }
        })
    } else {
        None
    };
    let result_collection = match want {
        Some(Seq(t)) if n == ".." => Some(t.as_ref()),
        _ => None,
    };
    let reduction = want.filter(|t| numeric(t)).map(|t| Seq(bx(t.clone())));
    let shape = |i| match n {
        "+" | "-" | "*" if matches!(want, Some(N | Z | Q)) => want,
        "//" | "%" if matches!(want, Some(N | Z)) => want,
        "^" if i == 1 => Some(&N),
        "^" if matches!(want, Some(N | Z | Q)) => want,
        "[:" | "take" | "drop" if i == 0 => Some(&N),
        "slice" if i < 2 => Some(&N),
        "+/" | "*/" if reduction.is_some() => reduction.as_ref(),
        ".." if result_collection.is_some() => result_collection,
        _ if matches!(args.get(i), Some(Expr::Int(_))) && preferred_numeric.is_some() => {
            preferred_numeric.as_ref()
        }
        "~" => Some(&B),
        _ => None,
    };
    let ty = args
        .iter()
        .enumerate()
        .map(|(i, x)| {
            expr(env, x, shape(i), effect).or_else(|original| {
                if shape(i).is_some()
                    && matches!(n, "+" | "-" | "*" | "//" | "%" | "^" | "+/" | "*/" | "..")
                    && !(n == "^" && i == 1)
                {
                    expr(env, x, None, effect).map_err(|_| original)
                } else {
                    Err(original)
                }
            })
        })
        .collect::<R<Vec<_>>>()?;
    builtin(env, n, &ty, want, effect)
}

fn lift(a: &T, b: &T, f: &impl Fn(&T, &T) -> R<T>) -> R<T> {
    match (a, b) {
        (Seq(a), Seq(b)) => Ok(Seq(bx(lift(a, b, f)?))),
        (Seq(a), b) => Ok(Seq(bx(lift(a, b, f)?))),
        (a, Seq(b)) => Ok(Seq(bx(lift(a, b, f)?))),
        (a, b) => f(a, b),
    }
}
fn comparable(t: &T) -> bool {
    match t {
        Fun(..) | Action(_) => false,
        Seq(t) | Opt(t) | Table(t) => comparable(t),
        Dict(k, v) => comparable(k) && comparable(v),
        Tuple(ts) => ts.iter().all(comparable),
        _ => true,
    }
}
fn csv_schema(env: &Env, row: &T) -> R<()> {
    let fields = record_fields(env, row)?;
    if fields.is_empty() {
        return Err("CSV requires at least one column".into());
    }
    fn scalar(t: &T) -> bool {
        matches!(t, N | Z | Q | Approx | B | S | C)
    }
    for (name, t) in fields {
        if !scalar(&t) && !matches!(&t,Opt(x) if scalar(x)) {
            return Err(format!("CSV column {name} has unsupported type {t}"));
        }
    }
    Ok(())
}
fn builtin(env: &Env, n: &str, ts: &[T], _want: Option<&T>, _effect: bool) -> R<T> {
    let one = || {
        arity(n, ts.len(), 1)?;
        Ok::<&T, String>(&ts[0])
    };
    let two = || {
        arity(n, ts.len(), 2)?;
        Ok::<(&T, &T), String>((&ts[0], &ts[1]))
    };
    let number = |t: &T| {
        if numeric(t) {
            Ok(())
        } else {
            Err(format!("{n} expects numeric operands, found {t}"))
        }
    };
    match n {
        "+" | "-" | "*" | "/" | "//" | "%" | "^" => {
            let (a, b) = two()?;
            lift(a, b, &|a, b| {
                number(a)?;
                number(b)?;
                if matches!(n, "//" | "%") && !(integral(a) && integral(b)) {
                    return Err(format!("{n} expects integral operands, found {a} and {b}"));
                }
                if n == "^" {
                    if b != &N {
                        return Err(format!("exponent must have type n, found {b}"));
                    }
                    return Ok(a.clone());
                }
                if n == "/" {
                    return Ok(Q);
                }
                join(a.clone(), b.clone())
            })
        }
        ".." => {
            let (a, b) = two()?;
            if !integral(a) || !integral(b) {
                return Err("range endpoints must be integral".into());
            }
            Ok(Seq(bx(join(a.clone(), b.clone())?)))
        }
        "<" | "<=" | ">" | ">=" | "=" | "!=" | "/=" | "==" => {
            let (a, b) = two()?;
            let cmp = |a: &T, b: &T| {
                let j = join(a.clone(), b.clone())?;
                if !comparable(&j) {
                    return Err(format!("{n} cannot compare {j}"));
                }
                if matches!(n, "<" | "<=" | ">" | ">=")
                    && !(numeric(&j) || matches!(j, S | C | Approx))
                {
                    return Err(format!("{n} cannot order {j}"));
                }
                Ok(B)
            };
            if n == "==" {
                cmp(a, b)
            } else {
                lift(a, b, &cmp)
            }
        }
        "-:" | "~-" | "|" => {
            let t = one()?;
            number(t)?;
            Ok(if n != "|" && t == &N { Z } else { t.clone() })
        }
        "~" => {
            let t = one()?;
            if t == &B {
                Ok(B)
            } else {
                Err(mismatch(&B, t))
            }
        }
        "&&" | "||" => {
            let (a, b) = two()?;
            lift(a, b, &|a, b| {
                if a == &B && b == &B {
                    Ok(B)
                } else {
                    Err(format!("{n} requires Boolean operands"))
                }
            })
        }
        "+/" | "*/" | "avg" | ">/" | "</" | "&&/" | "||/" => {
            let Seq(t) = one()? else {
                return Err(format!("{n} expects a sequence"));
            };
            if matches!(n, "&&/" | "||/") {
                if t.as_ref() != &B {
                    return Err(format!("{n} requires a Boolean sequence"));
                }
                return Ok(B);
            }
            number(t)?;
            Ok(match n {
                "avg" => Opt(bx(Q)),
                ">/" | "</" => Opt(t.clone()),
                _ => t.as_ref().clone(),
            })
        }
        "#" => match one()? {
            Seq(_) | Table(_) | Dict(..) | S => Ok(N),
            t => Err(format!("# expects a collection, found {t}")),
        },
        "#:" => match one()? {
            Seq(t) if matches!(t.as_ref(), Tuple(_) | Record(_) | Shape(_)) => Ok(Table(t.clone())),
            t @ (Shape(_) | Record(_)) => Ok(Table(bx(shape_type(column_shape(env, t)?)?))),
            _ => Err("#: expects a sequence of rows or a record of columns".into()),
        },
        ":#" => match one()? {
            Table(t) => Ok(Seq(t.clone())),
            _ => Err(":# expects a table".into()),
        },
        "+:" => {
            let (a, b) = two()?;
            if a == b && matches!(a, Table(_) | Seq(_)) {
                Ok(a.clone())
            } else {
                Err("+: requires matching collection types".into())
            }
        }
        "&:" | "*:" => {
            let (a, b) = two()?;
            match (n, a, b) {
                ("&:", Seq(a), Seq(b)) => {
                    Ok(Seq(bx(Tuple(vec![a.as_ref().clone(), b.as_ref().clone()]))))
                }
                ("*:", Table(a), Table(b)) => Ok(Table(bx(Tuple(vec![
                    a.as_ref().clone(),
                    b.as_ref().clone(),
                ])))),
                _ => Err(format!("{n} requires two matching collection kinds")),
            }
        }
        "=." => {
            let t = one()?;
            if matches!(t, Seq(_)) && comparable(t) {
                Ok(t.clone())
            } else {
                Err(format!("{n} expects comparable rows or elements"))
            }
        }
        "[:" => {
            let (a, b) = two()?;
            if a == &N && matches!(b, Seq(_) | Table(_) | S) {
                Ok(b.clone())
            } else {
                Err("take expects a natural count and a sequence, table, or string".into())
            }
        }
        ".:" => {
            let (a, b) = two()?;
            match b {
                Seq(t) | Table(t) if a == &N => Ok(Opt(t.clone())),
                Dict(k, v) if k.as_ref() == a => Ok(Opt(v.clone())),
                _ => Err(format!("cannot index {b} with {a}")),
            }
        }
        "take" | "drop" | "slice" => {
            arity(n, ts.len(), if n == "slice" { 3 } else { 2 })?;
            let (inds, tail) = ts.split_at(ts.len() - 1);
            if inds.iter().all(|t| t == &N) && matches!(&tail[0], Seq(_) | Table(_) | S) {
                Ok(tail[0].clone())
            } else {
                Err(format!(
                    "{n} expects natural indices and a sequence, table, or string"
                ))
            }
        }
        "rev" => match one()? {
            t @ (Seq(_) | Table(_) | S) => Ok(t.clone()),
            _ => Err("rev expects a sequence, table, or string".into()),
        },
        "flat" => match one()? {
            Seq(t) if matches!(t.as_ref(), Seq(_)) => Ok(t.as_ref().clone()),
            _ => Err("flat expects a sequence of sequences".into()),
        },
        "tr" => match one()? {
            t @ Seq(rows) if matches!(rows.as_ref(), Seq(_)) => Ok(t.clone()),
            _ => Err("tr expects a rectangular sequence of sequences".into()),
        },
        "sd" | "statistics.sd" | "stats.sd" => match one()? {
            Seq(t) if numeric(t) || t.as_ref() == &Approx => Ok(Opt(bx(Approx))),
            _ => Err(format!("{n} expects an exact numeric sequence")),
        },
        "^/" => {
            let x = one()?;
            if x != &Approx {
                number(x)?;
            }
            Ok(Approx)
        }
        "med" | "var" | "statistics.med" | "statistics.var" | "stats.med" | "stats.var"
        | "statistics.avg" | "statistics.mean" | "stats.avg" | "stats.mean" => match one()? {
            Seq(t) if numeric(t) => Ok(Opt(bx(Q))),
            _ => Err(format!("{n} expects a numerical sequence")),
        },
        "pct" | "statistics.pct" | "stats.pct" => {
            let (a, b) = two()?;
            if numeric(a) && matches!(b,Seq(t) if numeric(t)) {
                Ok(Opt(bx(Q)))
            } else {
                Err("pct expects a numerical percentile and a numerical sequence".into())
            }
        }
        "?+" | "?!" => match one()? {
            Opt(t) => Ok(if n == "?+" { B } else { t.as_ref().clone() }),
            t => Err(format!("{n} expects an optional, found {t}")),
        },
        ":n" | ":z" | ":q" => {
            number(one()?)?;
            Ok(match n {
                ":n" => N,
                ":z" => Z,
                _ => Q,
            })
        }
        "io.print" | "io.println" | "!>" => {
            one()?;
            Ok(Action(bx(Unit)))
        }
        "csv.read" => {
            let t = one()?;
            if t != &S {
                return Err(mismatch(&S, t));
            }
            if let Some(Action(out)) = _want {
                if let Table(row) = out.as_ref() {
                    csv_schema(env, row)?;
                    return Ok(Action(out.clone()));
                }
            }
            Ok(Action(bx(Seq(bx(Seq(bx(S)))))))
        }
        "csv.write" => {
            let (path, t) = two()?;
            if path != &S {
                return Err(mismatch(&S, path));
            }
            let Table(row) = t else {
                return Err("csv.write expects a named or structural table".into());
            };
            csv_schema(env, row)?;
            Ok(Action(bx(Unit)))
        }
        s if s.starts_with('.') && s.len() > 1 && s != ".:" => column(env, &s[1..], one()?),
        _ => Err(format!(
            "unknown or unsupported callable {n} in the Flux prototype"
        )),
    }
}

fn program_env(p: &Program) -> Result<Env, Error> {
    let mut env = Env::default();
    for import in &p.imports {
        if !matches!(
            import.as_str(),
            "io" | "csv" | "df" | "table" | "stats" | "statistics"
        ) {
            return Err(Error::new(
                0,
                format!(
                    "import {import} is not supported; available modules: io, csv, df, statistics"
                ),
            ));
        }
    }
    top_defs_check(&p.defs, &mut env)?;
    Ok(env)
}

pub fn inspect(p: &Program, expression: &Expr) -> Result<String, Error> {
    let env = program_env(p)?;
    expr(&env, expression, None, false)
        .map(|t| t.to_string())
        .map_err(|e| Error::new(0, e))
}
pub fn inspect_binding(p: &Program, name: &str) -> Result<String, Error> {
    let env = program_env(p)?;
    match env.values.get(name) {
        Some(Binding::Value(t)) => Ok(t.to_string()),
        Some(Binding::Callable(ts, Some(t))) => Ok(fun(ts.clone(), t.clone()).to_string()),
        _ => Err(Error::new(0, format!("unknown binding {name}"))),
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inspection {
    pub type_name: String,
    pub schema: Option<String>,
}
fn describe(env: &Env, t: &T) -> Inspection {
    let schema = match t {
        Table(row) => match record_fields(env, row) {
            Ok(fields) => Some(format!(
                ".{{ {} }}",
                fields
                    .iter()
                    .map(|(n, t)| format!("{n}:{t}"))
                    .collect::<Vec<_>>()
                    .join("; ")
            )),
            Err(_) => Some(row.to_string()),
        },
        _ => None,
    };
    Inspection {
        type_name: t.to_string(),
        schema,
    }
}
pub fn inspect_entry(p: &Program) -> Result<Inspection, Error> {
    let env = program_env(p)?;
    let e = p
        .entry
        .as_ref()
        .ok_or_else(|| Error::new(0, "missing expression to inspect"))?;
    let t = expr(&env, e, None, true).map_err(|e| Error::new(p.entry_start.unwrap_or(0), e))?;
    Ok(describe(
        &env,
        match &t {
            Action(inner) => inner,
            _ => &t,
        },
    ))
}
pub fn check(p: &Program) -> Result<(), Error> {
    let env = program_env(p)?;
    if let Some(e) = &p.entry {
        let t = expr(&env, e, None, true).map_err(|e| Error::new(p.entry_start.unwrap_or(0), e))?;
        if matches!(&t,Action(ret) if ret.as_ref()==&Z)
            && expr(&env, e, Some(&Action(bx(N))), true).is_ok()
        {
            return Ok(());
        }

        let returned = match &t {
            Action(r) => r.as_ref(),
            _ => &t,
        };
        if !matches!(returned, Unit | N) {
            return Err(Error::new(
                p.entry_start.unwrap_or(0),
                format!("entry must yield () or n, found {returned}"),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn verify(s: &str) -> Result<(), Error> {
        check(&parse(s)?)
    }
    fn check_error(s: &str, needle: &str) {
        let p = parse(s).unwrap();
        let error = check(&p).unwrap_err().to_string();
        assert!(error.contains(needle), "expected {needle:?} in {error:?}");
    }
    #[test]
    fn bad_entry_reports_the_entry_start() {
        let source = "x := 3;

!{ 0b };";
        let p = parse(source).unwrap();
        assert_eq!(check(&p).unwrap_err().start, source.find("!{").unwrap());
        let source = "x := 3;

!{ ! x };";
        let p = parse(source).unwrap();
        assert_eq!(
            inspect_entry(&p).unwrap_err().start,
            source.find("!{").unwrap()
        );
    }
    #[test]
    fn return_and_argument_types() {
        verify("sq(x:q):q := * x x; y := sq 4;").unwrap();
        check_error("sq(x:q):q := * x x; y := sq 0b;", "expected q, found b");
        check_error("f(x:n):n := 0b;", "expected n, found b");
    }
    #[test]
    fn lexical_shadowing_and_scope() {
        verify("f(x:n):n := x; g(f:n):n := { a := + f 1; a };").unwrap();
        assert!(verify("f(x:n):n := { a := b; b := x; a };").is_err());
        verify("a := + b 1; b := 2;").unwrap();
    }
    #[test]
    fn rejects_bad_condition_side_even_if_never_run() {
        check_error("x := ? 1b 2 0b;", "expected z, found b");
        check_error("x := ? 3 2 1;", "expected b, found z");
    }
    #[test]
    fn collection_and_record_validation() {
        verify("row :: { city:s; cost:q; }; data: #row := #[row { city:=\"X\"; cost:=2; }]; x:?q := avg .cost data;").unwrap();
        check_error("x := [1 0b];", "expected z, found b");
        check_error(
            "row :: { city:s; cost:q; }; x := row { city:=\"X\"; };",
            "missing field cost",
        );
        check_error("x := .3 (1,2);", "no position .3");
        verify("data: #[s ?q] := #[(\"X\",?0) (\"Y\",?1 2)];").unwrap();
    }
    #[test]
    fn checks_effects_and_entry_result() {
        verify("!{ 0 };").unwrap();
        check_error("!{ 0b };", "entry must yield () or n");
        check_error("!{ ! 2 };", "expects an action");
    }
    #[test]
    fn recursion_and_function_values() {
        verify("fact(x:n):n := ? (= x 0) 1 (* x (fact (- x 1))); f := &fact; v := @ f 4;").unwrap();
        check_error("f := 2; x := @ f 4;", "expected a function value");
        verify("xs:[n] := @: (\\(v)-> + v 1) [1n 2n 3n];").unwrap();
    }
    #[test]
    fn grouped_stats_with_mask_and_csv_effect_type() {
        verify(
            r#"
            << "io" : o;
            << "csv" : c;
            !{
                raw:[[s]] := ! c.read "example.csv";
                t := #[("x",5) ("y",2) ("x",7)];
                good := ?: (&& (> .2 t 0) (< .2 t 7)) t;
                g := =: &.1 good;
                m:{s:?q} := @: (\(group)-> avg .2 group) g;
                ! o.print m;
            };
        "#,
        )
        .unwrap();
        check_error(r#"<< "csv" : c; !{ ! c.read 17; };"#, "expected s, found z");
    }

    #[test]
    fn validates_tuple_bundle_operands() {
        verify("f(a,b:q):q := + a b; x := @@ &f (2,3);").unwrap();
        check_error(
            "f(a,b:q):q := + a b; x := @@ &f (2,0b);",
            "expected q, found b",
        );
    }

    #[test]
    fn structural_tables_extend_and_pipeline_inspect() {
        let p = parse(
            r#"
            << "table" : df;
            t := #: .{ name := ["x" "y"]; gross := [2 3]; };
            v := df.extend .{ net := - .gross t 1; } t;
            !{ |{ v; take 1 $; $ } };
        "#,
        )
        .unwrap();
        let inspected = inspect_entry(&p).unwrap();
        assert_eq!(inspected.type_name, "#.{ name:s; gross:z; net:z }");
        assert_eq!(
            inspected.schema.as_deref(),
            Some(".{ name:s; gross:z; net:z }")
        );
        check_error(
            r#" << "df"; t := #: .{ amount := [1]; }; v := df.extend .{ amount := [2]; } t; "#,
            "cannot replace existing column amount",
        );
        check_error("t := #: .{ amount := 1; };", "must be a sequence");
        check_error("t := .missing .{ amount := 1; };", "no field missing");
    }
    #[test]
    fn typed_lambdas_and_grouped_schema() {
        let p = parse(
            r#"
            << "df";
            t := #: .{ station := ["north" "south"]; value := [2 5]; };
            g := df.agg &.station (\(k,rows)-> .{ station := k; average := avg .value rows; }) t;
            lambda := \(x:q) -> / 1 x;
            value:q := @ lambda 2;
            !{ g };
        "#,
        )
        .unwrap();
        assert_eq!(
            inspect_entry(&p).unwrap().schema.as_deref(),
            Some(".{ station:s; average:?q }")
        );
    }
    #[test]
    fn contextual_naturals_signed_default_and_typed_csv() {
        verify(
            r#"
            << "csv";
            obs :: { station:s; value:q; valid:b; };
            !{ t:#obs := ! csv.read "input.csv"; u := slice 1 2 t; ! csv.write "out.csv" u; };
        "#,
        )
        .unwrap();
        verify(
            r#"
            << "csv";
            !{ t:#.{ station:s; value:?q; } := ! csv.read "input.csv";
                ! csv.write "out.csv" t; };
        "#,
        )
        .unwrap();
        let p = parse("x := - 3 5; y:n := 2; c := 0.125; !{ () };").unwrap();
        assert_eq!(inspect_binding(&p, "x").unwrap(), "z");
        assert_eq!(inspect_binding(&p, "y").unwrap(), "n");
        assert_eq!(inspect_binding(&p, "c").unwrap(), "q");
    }

    #[test]
    fn string_collection_operations_type_as_strings() {
        verify(
            r#"
            word := "a\u{00e9}b";
            a:s := take 2 word;
            b:s := drop 1 word;
            c:s := slice 2 1 word;
            d:s := rev word;
            e:s := [: 2 word;
        "#,
        )
        .unwrap();
        check_error(r#"a := drop -1 "abc";"#, "expected n, found z");
        check_error(r#"a := rev 1;"#, "rev expects a sequence, table, or string");
    }

    #[test]
    fn rejects_unsupported_types() {
        check_error("x:d := 1;", "not supported");
        verify("x:r := 1.0r;").unwrap();
        check_error(
            "id(x:'a):'a := x;",
            "polymorphic type variables are not supported",
        );
    }
}
