//! Reference evaluator. Exact arithmetic is checked and bounded by i128.
use crate::{
    parser::{Expr, Program, Stmt},
    Error,
};
use std::{cmp::Ordering, collections::HashMap, io::Read, rc::Rc};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum Kind {
    N,
    Z,
    Q,
}
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct Num {
    n: i128,
    d: i128,
    kind: Kind,
}
type Rt<T> = Result<T, String>;
fn overflow() -> String {
    "exact arithmetic exceeds the prototype's i128 bound".into()
}
fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}
impl Num {
    fn int(n: i128, kind: Kind) -> Self {
        Self { n, d: 1, kind }
    }
    fn new(n: i128, d: i128, kind: Kind) -> Rt<Self> {
        if d == 0 {
            return Err("division by zero".into());
        }
        if n == 0 {
            return Ok(Self { n: 0, d: 1, kind });
        }
        let g = gcd(n.unsigned_abs(), d.unsigned_abs());
        let (n, d) = if g == 1u128 << 127 {
            (1, 1)
        } else {
            let g = g as i128;
            (n / g, d / g)
        };
        if d < 0 {
            Ok(Self {
                n: n.checked_neg().ok_or_else(overflow)?,
                d: d.checked_neg().ok_or_else(overflow)?,
                kind,
            })
        } else {
            Ok(Self { n, d, kind })
        }
    }
    fn lit(s: &str) -> Rt<Self> {
        let frac = s.contains('.');
        let (s, kind) = match s.as_bytes().last() {
            Some(b'n') => (&s[..s.len() - 1], Kind::N),
            Some(b'z') => (&s[..s.len() - 1], Kind::Z),
            Some(b'q') => (&s[..s.len() - 1], Kind::Q),
            Some(b'd' | b'r') => {
                return Err(
                    "decimal d and binary64 r are not supported by this prototype; use exact q"
                        .into(),
                )
            }
            _ => (
                s,
                if frac {
                    Kind::Q
                } else if s.starts_with('-') {
                    Kind::Z
                } else {
                    Kind::N
                },
            ),
        };
        if let Some((a, b)) = s.split_once('.') {
            if kind != Kind::Q {
                return Err("fractional natural or integer literal".into());
            }
            let b = b.trim_end_matches('0');
            if b.is_empty() {
                return Ok(Self::int(a.parse().map_err(|_| overflow())?, kind));
            }
            let n = format!("{a}{b}").parse::<i128>().map_err(|_| overflow())?;
            let d = 10_i128
                .checked_pow(b.len().try_into().map_err(|_| overflow())?)
                .ok_or_else(overflow)?;
            Self::new(n, d, kind)
        } else {
            let n = s.parse().map_err(|_| overflow())?;
            if kind == Kind::N && n < 0 {
                return Err("natural number cannot be negative".into());
            }
            Ok(Self::int(n, kind))
        }
    }
    fn integral(self) -> Rt<i128> {
        if self.d != 1 {
            Err("expected an integral value".into())
        } else {
            Ok(self.n)
        }
    }
    fn cmp(self, b: Self) -> Ordering {
        fn pos(mut an: u128, mut ad: u128, mut bn: u128, mut bd: u128) -> Ordering {
            let mut rev = false;
            loop {
                let c = (an / ad).cmp(&(bn / bd));
                let (ar, br) = (an % ad, bn % bd);
                let c = if c == Ordering::Equal && (ar == 0 || br == 0) {
                    ar.cmp(&br)
                } else {
                    c
                };
                if c != Ordering::Equal || ar == 0 {
                    return if rev { c.reverse() } else { c };
                }
                (an, ad, bn, bd) = (ad, ar, bd, br);
                rev = !rev;
            }
        }
        match (self.n < 0, b.n < 0) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (a, _) => {
                let c = pos(
                    self.n.unsigned_abs(),
                    self.d as u128,
                    b.n.unsigned_abs(),
                    b.d as u128,
                );
                if a {
                    c.reverse()
                } else {
                    c
                }
            }
        }
    }
    fn kind(a: Kind, b: Kind) -> Kind {
        if a == Kind::Q || b == Kind::Q {
            Kind::Q
        } else if a == Kind::Z || b == Kind::Z {
            Kind::Z
        } else {
            Kind::N
        }
    }
    fn calc(self, op: &str, b: Self) -> Rt<Self> {
        let k = Self::kind(self.kind, b.kind);
        let mul = |a: i128, b: i128| a.checked_mul(b).ok_or_else(overflow);
        match op {
            "+" | "-" => {
                let g = gcd(self.d as u128, b.d as u128) as i128;
                let a = mul(self.n, b.d / g)?;
                let c = mul(b.n, self.d / g)?;
                let n = if op == "+" {
                    a.checked_add(c)
                } else {
                    a.checked_sub(c)
                }
                .ok_or_else(overflow)?;
                if k == Kind::N && n < 0 {
                    return Err("natural subtraction underflow".into());
                }
                Self::new(n, mul(self.d / g, b.d)?, k)
            }
            "*" => {
                let g = gcd(self.n.unsigned_abs(), b.d as u128) as i128;
                let h = gcd(b.n.unsigned_abs(), self.d as u128) as i128;
                Self::new(mul(self.n / g, b.n / h)?, mul(self.d / h, b.d / g)?, k)
            }
            "/" => {
                if b.n == 0 {
                    return Err("division by zero".into());
                }
                let g = gcd(self.n.unsigned_abs(), b.n.unsigned_abs());
                let (a, c) = if g == 1u128 << 127 {
                    (1, 1)
                } else {
                    let g = g as i128;
                    (self.n / g, b.n / g)
                };
                let h = gcd(self.d as u128, b.d as u128) as i128;
                Self::new(mul(a, b.d / h)?, mul(self.d / h, c)?, Kind::Q)
            }
            "//" | "%" => {
                let (a, b) = (self.integral()?, b.integral()?);
                if b == 0 {
                    return Err("division by zero".into());
                }
                if k == Kind::Q {
                    return Err("integral operator requires n or z operands".into());
                }
                let mut q = a.checked_div(b).ok_or_else(overflow)?;
                let r = a.checked_rem(b).ok_or_else(overflow)?;
                if r != 0 && (r < 0) != (b < 0) {
                    q = q.checked_sub(1).ok_or_else(overflow)?;
                }
                let result = if op == "//" {
                    q
                } else if r != 0 && (r < 0) != (b < 0) {
                    r.checked_add(b).ok_or_else(overflow)?
                } else {
                    r
                };
                Ok(Self::int(result, k))
            }
            "^" => {
                let exp = b.integral()?;
                if exp < 0 || b.kind != Kind::N {
                    return Err("power requires a natural exponent".into());
                }
                let mut a = self;
                let mut out = Self::int(1, a.kind);
                let mut e = exp as u128;
                while e != 0 {
                    if e & 1 == 1 {
                        out = out.calc("*", a)?;
                    }
                    e >>= 1;
                    if e != 0 {
                        a = a.calc("*", a)?;
                    }
                }
                Ok(out)
            }
            _ => Err(format!("unsupported numeric operator {op}")),
        }
    }
}

#[derive(Clone)]
enum V {
    Num(Num),
    Str(String),
    Char(char),
    Bool(bool),
    Unit,
    Tuple(Vec<V>),
    Array(Vec<V>),
    Table(Vec<V>),
    Record(Vec<(String, V)>),
    Dict(Vec<(V, V)>),
    Opt(Option<Box<V>>),
    Fn(Fun),
    Action(Action),
    Print(Box<V>),
    ReadCsv(String),
}
#[derive(Clone)]
struct Fun {
    code: Code,
    supplied: Vec<V>,
}
#[derive(Clone)]
enum Code {
    User {
        params: Vec<String>,
        types: Vec<Option<String>>,
        result: Option<String>,
        body: Rc<Expr>,
        env: usize,
    },
    Builtin(String, usize),
}
#[derive(Clone)]
struct Action {
    steps: Rc<Vec<Stmt>>,
    tail: Option<Rc<Expr>>,
    env: usize,
}
#[derive(Clone)]
enum Binding {
    Ready(V),
    Pending {
        body: Rc<Expr>,
        ty: Option<String>,
        env: usize,
    },
    Busy,
    NotReady,
}
#[derive(Default)]
struct Frame {
    parent: Option<usize>,
    vals: HashMap<String, Binding>,
}
struct Engine {
    frames: Vec<Frame>,
    stdout: String,
    fuel: u64,
    depth: u16,
    effects: u16,
}
pub struct Output {
    pub stdout: String,
    pub status: u8,
}

fn repr(v: &V) -> String {
    let grouped = |xs: &[V], a: &str, b: &str, sep: &str| {
        format!(
            "{a}{}{b}",
            xs.iter().map(repr).collect::<Vec<_>>().join(sep)
        )
    };
    match v {
        V::Num(x) if x.d == 1 => x.n.to_string(),
        V::Num(x) => format!("{}/{}", x.n, x.d),
        V::Bool(b) => if *b { "1b" } else { "0b" }.into(),
        V::Str(s) => s.clone(),
        V::Char(c) => c.to_string(),
        V::Unit => "()".into(),
        V::Tuple(xs) => grouped(xs, "(", ")", ", "),
        V::Array(xs) => grouped(xs, "[", "]", " "),
        V::Table(xs) => grouped(xs, "#[", "]", " "),
        V::Record(xs) => format!(
            "{{{}}}",
            xs.iter()
                .map(|(k, v)| format!("{k} := {}", repr(v)))
                .collect::<Vec<_>>()
                .join("; ")
        ),
        V::Dict(xs) => format!(
            "#{{{}}}",
            xs.iter()
                .map(|(k, v)| format!("{} : {}", repr(k), repr(v)))
                .collect::<Vec<_>>()
                .join("; ")
        ),
        V::Opt(None) => "?0".into(),
        V::Opt(Some(v)) => format!("?1 {}", repr(v)),
        V::Fn(_) => "<function>".into(),
        V::Action(_) | V::Print(_) | V::ReadCsv(_) => "<action>".into(),
    }
}
fn equal(a: &V, b: &V) -> bool {
    match (a, b) {
        (V::Num(a), V::Num(b)) => a.cmp(*b) == Ordering::Equal,
        (V::Bool(a), V::Bool(b)) => a == b,
        (V::Str(a), V::Str(b)) => a == b,
        (V::Char(a), V::Char(b)) => a == b,
        (V::Unit, V::Unit) | (V::Opt(None), V::Opt(None)) => true,
        (V::Opt(Some(a)), V::Opt(Some(b))) => equal(a, b),
        (V::Tuple(a), V::Tuple(b)) | (V::Array(a), V::Array(b)) | (V::Table(a), V::Table(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| equal(a, b))
        }
        (V::Record(a), V::Record(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, v)| b.iter().any(|(l, w)| k == l && equal(v, w)))
        }
        (V::Dict(a), V::Dict(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, v)| b.iter().any(|(l, w)| equal(k, l) && equal(v, w)))
        }
        _ => false,
    }
}
fn order(a: &V, b: &V) -> Rt<Ordering> {
    match (a, b) {
        (V::Num(a), V::Num(b)) => Ok(a.cmp(*b)),
        (V::Str(a), V::Str(b)) => Ok(a.cmp(b)),
        (V::Char(a), V::Char(b)) => Ok(a.cmp(b)),
        _ => Err("values are not comparable in this prototype".into()),
    }
}
fn num(v: V) -> Rt<Num> {
    if let V::Num(v) = v {
        Ok(v)
    } else {
        Err("expected a number".into())
    }
}
fn natural(v: V) -> Rt<usize> {
    let v = num(v)?.integral()?;
    v.try_into()
        .map_err(|_| "expected a nonnegative machine-sized integer".into())
}
fn boolean(v: V) -> Rt<bool> {
    if let V::Bool(v) = v {
        Ok(v)
    } else {
        Err("expected a scalar Boolean".into())
    }
}
fn convert(v: V, t: &str) -> Rt<V> {
    if let V::Array(xs) = v {
        return Ok(V::Array(
            xs.into_iter().map(|x| convert(x, t)).collect::<Rt<_>>()?,
        ));
    }
    let mut x = num(v)?;
    x.kind = match t {
        "n" => {
            if x.integral()? < 0 {
                return Err("cannot convert negative value to n".into());
            }
            Kind::N
        }
        "z" => {
            x.integral()?;
            Kind::Z
        }
        "q" => Kind::Q,
        _ => return Err(format!("numeric conversion to {t} is not implemented")),
    };
    Ok(V::Num(x))
}
fn typed(v: V, t: Option<&str>) -> Rt<V> {
    match (v, t.map(str::trim)) {
        (v @ V::Num(_), Some(t @ ("n" | "z" | "q"))) => convert(v, t),
        (V::Num(_), Some("d" | "r")) => Err("d and r are not implemented; use exact q".into()),
        (V::Array(xs), Some(t)) if t.starts_with('[') && t.ends_with(']') => Ok(V::Array(
            xs.into_iter()
                .map(|x| typed(x, Some(&t[1..t.len() - 1])))
                .collect::<Rt<_>>()?,
        )),
        (V::Opt(Some(x)), Some(t)) if t.starts_with('?') => {
            Ok(V::Opt(Some(Box::new(typed(*x, Some(&t[1..]))?))))
        }
        (v, _) => Ok(v),
    }
}
fn arity(s: &str) -> Option<usize> {
    Some(match s {
        "+" | "-" | "*" | "/" | "//" | "%" | "^" | "=" | "/=" | "==" | "<" | "<=" | ">" | ">="
        | "&&" | "||" | ".." | "@" | "@@" | "&:" | "@:" | "??" | "?:" | "=:" | ">:" | "<:"
        | "+:" | "*:" | "[:" | ".:" | "pct" => 2,
        "/:" | "\\:" => 3,
        "?0" => 0,
        "?1" | "?+" | "?!" | "+/" | "*/" | "&&/" | "||/" | ">/" | "</" | "#" | "avg"
        | "io.print" | "io.println" | "csv.read" | "#[" | "#:" | ":#" | "=." | "~" | "-:" | "|"
        | "!" => 1,
        s if s.starts_with('.') && s.len() > 1 => 1,
        s if s.starts_with(':') && s.len() == 2 => 1,
        _ => return None,
    })
}
fn known(s: &str) -> V {
    V::Fn(Fun {
        code: Code::Builtin(s.into(), arity(s).unwrap_or(0)),
        supplied: Vec::new(),
    })
}
fn lift2(a: V, b: V, f: &impl Fn(V, V) -> Rt<V>) -> Rt<V> {
    Ok(match (a, b) {
        (V::Array(xs), V::Array(ys)) => {
            if xs.len() != ys.len() {
                return Err("shape mismatch: sequence lengths differ".into());
            }
            V::Array(
                xs.into_iter()
                    .zip(ys)
                    .map(|(a, b)| f(a, b))
                    .collect::<Rt<_>>()?,
            )
        }
        (V::Array(xs), b) => V::Array(xs.into_iter().map(|a| f(a, b.clone())).collect::<Rt<_>>()?),
        (a, V::Array(xs)) => V::Array(xs.into_iter().map(|b| f(a.clone(), b)).collect::<Rt<_>>()?),
        (a, b) => f(a, b)?,
    })
}

impl Engine {
    fn frame(&mut self, parent: Option<usize>) -> usize {
        let n = self.frames.len();
        self.frames.push(Frame {
            parent,
            vals: HashMap::new(),
        });
        n
    }
    fn put(&mut self, env: usize, name: String, b: Binding) {
        self.frames[env].vals.insert(name, b);
    }
    fn resolve(&mut self, mut env: usize, s: &str) -> Rt<Option<V>> {
        loop {
            if let Some(b) = self.frames[env].vals.get(s).cloned() {
                return match b {
                    Binding::Ready(v) => Ok(Some(v)),
                    Binding::Pending {
                        body,
                        ty,
                        env: use_env,
                    } => {
                        self.put(env, s.into(), Binding::Busy);
                        let v = self.eval_expected(&body, use_env, ty.as_deref())?;
                        self.put(env, s.into(), Binding::Ready(v.clone()));
                        Ok(Some(v))
                    }
                    Binding::Busy => Err(format!("cyclic value dependency involving {s}")),
                    Binding::NotReady => Err(format!("local value {s} used before initialization")),
                };
            }
            match self.frames[env].parent {
                Some(p) => env = p,
                None => return Ok(None),
            }
        }
    }
    fn eval(&mut self, e: &Expr, env: usize) -> Rt<V> {
        self.eval_expected(e, env, None)
    }
    fn eval_expected(&mut self, e: &Expr, env: usize, expected: Option<&str>) -> Rt<V> {
        if self.fuel == 0 {
            return Err("prototype execution limit exhausted".into());
        }
        self.fuel -= 1;
        if self.depth >= 128 {
            return Err("prototype nesting or recursion limit (128) exceeded".into());
        }
        self.depth += 1;
        let r = self
            .eval_inner(e, env, expected)
            .and_then(|v| typed(v, expected));
        self.depth -= 1;
        r
    }
    fn eval_inner(&mut self, e: &Expr, env: usize, expected: Option<&str>) -> Rt<V> {
        match e {
            Expr::Int(x) => Ok(V::Num(Num::int(*x, if *x < 0 { Kind::Z } else { Kind::N }))),
            Expr::Num(s) => Ok(V::Num(Num::lit(s)?)),
            Expr::Str(s) => Ok(V::Str(s.clone())),
            Expr::Char(c) => Ok(V::Char(*c)),
            Expr::Bool(b) => Ok(V::Bool(*b)),
            Expr::Unit => Ok(V::Unit),
            Expr::Tuple(es) => Ok(V::Tuple(self.eval_list(es, env)?)),
            Expr::Array(es) => {
                let t = expected
                    .filter(|t| t.starts_with('[') && t.ends_with(']'))
                    .map(|t| &t[1..t.len() - 1]);
                Ok(V::Array(
                    es.iter()
                        .map(|x| self.eval_expected(x, env, t))
                        .collect::<Rt<_>>()?,
                ))
            }
            Expr::Record(fields) | Expr::NamedRecord(_, fields) => Ok(V::Record(
                fields
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), self.eval(v, env)?)))
                    .collect::<Rt<_>>()?,
            )),
            Expr::Name(n) => match self.resolve(env, n)? {
                Some(v) => Ok(v),
                None if n == "?0" => Ok(V::Opt(None)),
                _ => Err(format!("unbound value {n}")),
            },
            Expr::Ref(n) => match self.resolve(env, n)? {
                Some(v @ V::Fn(_)) => Ok(v),
                Some(_) => Err(format!("{n} is not a referenceable callable")),
                None => arity(n)
                    .map(|_| known(n))
                    .ok_or_else(|| format!("unknown callable {n}")),
            },
            Expr::Lambda(params, b) => Ok(V::Fn(Fun {
                code: Code::User {
                    params: params.clone(),
                    types: vec![None; params.len()],
                    result: None,
                    body: Rc::new((**b).clone()),
                    env,
                },
                supplied: Vec::new(),
            })),
            Expr::Cases(cases, otherwise) => {
                for (c, v) in cases {
                    if boolean(self.eval(c, env)?)? {
                        return self.eval_expected(v, env, expected);
                    }
                }
                self.eval_expected(otherwise, env, expected)
            }
            Expr::Action(ss, tail) => Ok(V::Action(Action {
                steps: Rc::new(ss.clone()),
                tail: tail.as_deref().cloned().map(Rc::new),
                env,
            })),
            Expr::Block(ss, tail) => self.block(ss, tail.as_deref(), env, expected, false),
            Expr::Call(s, es) => self.call(s, es, env, expected),
        }
    }
    fn eval_list(&mut self, es: &[Expr], env: usize) -> Rt<Vec<V>> {
        es.iter().map(|x| self.eval(x, env)).collect()
    }
    fn available_fun(&self, s: &Stmt, env: usize) -> Option<(String, V)> {
        let Stmt::Def {
            name,
            params,
            param_types,
            result_type,
            body,
            ..
        } = s
        else {
            return None;
        };
        if params.is_empty() {
            return None;
        }
        Some((
            name.clone(),
            V::Fn(Fun {
                code: Code::User {
                    params: params.clone(),
                    types: param_types.clone(),
                    result: result_type.clone(),
                    body: Rc::new(body.clone()),
                    env,
                },
                supplied: Vec::new(),
            }),
        ))
    }
    fn block(
        &mut self,
        steps: &[Stmt],
        tail: Option<&Expr>,
        parent: usize,
        expected: Option<&str>,
        effect: bool,
    ) -> Rt<V> {
        let scope = self.frame(Some(parent));
        let mut env = scope;
        let mut plan = Vec::with_capacity(steps.len());
        // Allocate each declaration's lexical frame first. Hoisted functions can
        // call each other, but a local value is initialized only on its own step.
        for s in steps {
            let before = env;
            match s {
                Stmt::Def { name, params, .. } if params.is_empty() => {
                    env = self.frame(Some(env));
                    self.put(env, name.clone(), Binding::NotReady);
                }
                Stmt::Effect {
                    name: Some(name), ..
                } => {
                    env = self.frame(Some(env));
                    self.put(env, name.clone(), Binding::NotReady);
                }
                Stmt::Def { .. } => {
                    let (name, v) = self.available_fun(s, env).expect("nonempty formal list");
                    self.put(scope, name, Binding::Ready(v));
                }
                _ => {}
            }
            plan.push((before, env));
        }
        for (s, (before, after)) in steps.iter().zip(plan) {
            match s {
                Stmt::Def {
                    name,
                    params,
                    result_type,
                    body,
                    ..
                } if params.is_empty() => {
                    let v = self.eval_expected(body, before, result_type.as_deref())?;
                    self.put(after, name.clone(), Binding::Ready(v));
                }
                Stmt::Effect { name, body, .. } => {
                    if !effect {
                        return Err("effect statement in pure scope".into());
                    }
                    let v = self.eval(body, before)?;
                    if let Some(n) = name {
                        self.put(after, n.clone(), Binding::Ready(v));
                    }
                }
                _ => {}
            }
        }
        match tail {
            Some(t) => self.eval_expected(t, env, expected),
            None => Ok(V::Unit),
        }
    }
    fn execute(&mut self, v: V) -> Rt<V> {
        let a = match v {
            V::Print(v) => {
                self.stdout.push_str(&repr(&v));
                self.stdout.push('\n');
                return Ok(V::Unit);
            }
            V::ReadCsv(path) => return read_csv(&path),
            V::Action(a) => a,
            _ => return Err("expected an action after !".into()),
        };
        self.effects += 1;
        let r = self.block(&a.steps, a.tail.as_deref(), a.env, None, true);
        self.effects -= 1;
        r
    }
    fn apply(&mut self, f: V, args: Vec<V>) -> Rt<V> {
        let V::Fn(mut f) = f else {
            return Err("expected a callable function value".into());
        };
        f.supplied.extend(args);
        let a = match &f.code {
            Code::User { params, .. } => params.len(),
            Code::Builtin(_, a) => *a,
        };
        if f.supplied.len() < a {
            return Ok(V::Fn(f));
        }
        if f.supplied.len() > a {
            return Err("too many operands for function value".into());
        }
        match f.code {
            Code::Builtin(s, _) => self.builtin(&s, f.supplied),
            Code::User {
                params,
                types,
                result,
                body,
                env,
            } => {
                let local = self.frame(Some(env));
                for ((n, v), i) in params.into_iter().zip(f.supplied).zip(0..) {
                    self.put(
                        local,
                        n,
                        Binding::Ready(typed(v, types.get(i).and_then(Option::as_deref))?),
                    );
                }
                self.eval_expected(&body, local, result.as_deref())
            }
        }
    }
}

fn seq(v: V) -> Rt<(Vec<V>, bool)> {
    match v {
        V::Array(xs) => Ok((xs, false)),
        V::Table(xs) => Ok((xs, true)),
        _ => Err("expected a sequence or table".into()),
    }
}
fn array(v: V) -> Rt<Vec<V>> {
    if let V::Array(xs) = v {
        Ok(xs)
    } else {
        Err("expected a sequence".into())
    }
}
fn wrap(xs: Vec<V>, table: bool) -> V {
    if table {
        V::Table(xs)
    } else {
        V::Array(xs)
    }
}
fn optional(v: Option<V>) -> V {
    V::Opt(v.map(Box::new))
}
fn is_row(v: &V) -> bool {
    matches!(v, V::Tuple(_) | V::Record(_))
}
fn select(name: &str, v: V) -> Rt<V> {
    match v {
        V::Tuple(xs) => {
            let i = name
                .parse::<usize>()
                .map_err(|_| format!("expected a tuple selector, not .{name}"))?;
            if i == 0 {
                return Err("tuple selectors start at one".into());
            }
            xs.into_iter()
                .nth(i - 1)
                .ok_or_else(|| format!("tuple has no field .{name}"))
        }
        V::Record(xs) => xs
            .into_iter()
            .find_map(|(k, v)| if k == name { Some(v) } else { None })
            .ok_or_else(|| format!("record has no field .{name}")),
        V::Table(xs) | V::Array(xs) => Ok(V::Array(
            xs.into_iter().map(|x| select(name, x)).collect::<Rt<_>>()?,
        )),
        _ => Err(format!(
            "selector .{name} requires a tuple, record, or structured collection"
        )),
    }
}
fn broadcast(s: &str, a: V, b: V) -> Rt<V> {
    lift2(a, b, &|a, b| {
        Ok(match s {
            "+" | "-" | "*" | "/" | "//" | "%" | "^" => V::Num(num(a)?.calc(s, num(b)?)?),
            "=" => V::Bool(equal(&a, &b)),
            "/=" => V::Bool(!equal(&a, &b)),
            "<" => V::Bool(order(&a, &b)?.is_lt()),
            "<=" => V::Bool(!order(&a, &b)?.is_gt()),
            ">" => V::Bool(order(&a, &b)?.is_gt()),
            ">=" => V::Bool(!order(&a, &b)?.is_lt()),
            "&&" => V::Bool(boolean(a)? & boolean(b)?),
            "||" => V::Bool(boolean(a)? | boolean(b)?),
            _ => return Err(format!("unknown binary operator {s}")),
        })
    })
}
fn reduced(s: &str, v: V) -> Rt<V> {
    let xs = array(v)?;
    match s {
        "+/" | "*/" | "avg" => {
            if s == "avg" && xs.is_empty() {
                return Ok(optional(None));
            }
            let count = xs.len() as i128;
            let prod = s == "*/";
            let mut a = Num::int(i128::from(prod), Kind::N);
            for x in xs {
                a = a.calc(if prod { "*" } else { "+" }, num(x)?)?
            }
            if s == "avg" {
                Ok(optional(Some(V::Num(
                    a.calc("/", Num::int(count, Kind::N))?,
                ))))
            } else {
                Ok(V::Num(a))
            }
        }
        "&&/" | "||/" => {
            let all = s == "&&/";
            let mut a = all;
            for x in xs {
                let b = boolean(x)?;
                a = if all { a && b } else { a || b };
            }
            Ok(V::Bool(a))
        }
        ">/" | "</" => {
            let mut it = xs.into_iter();
            let Some(mut a) = it.next() else {
                return Ok(optional(None));
            };
            for b in it {
                let c = order(&a, &b)?;
                if if s == ">/" { c.is_lt() } else { c.is_gt() } {
                    a = b
                }
            }
            Ok(optional(Some(a)))
        }
        _ => Err(format!("unsupported reduction {s}")),
    }
}

impl Engine {
    fn call(&mut self, s: &str, es: &[Expr], env: usize, expected: Option<&str>) -> Rt<V> {
        if s == "??" {
            if es.len() != 2 {
                return Err("?? takes two operands".into());
            }
            return match self.eval(&es[1], env)? {
                V::Opt(None) => self.eval_expected(&es[0], env, expected),
                V::Opt(Some(v)) => Ok(*v),
                _ => Err("?? expects an optional second operand".into()),
            };
        }
        if let Some(f) = self.resolve(env, s)? {
            // Adopt declared parameter types for unsuffixed numeric arguments.
            let ts = if let V::Fn(Fun {
                code: Code::User { types, .. },
                supplied,
                ..
            }) = &f
            {
                Some(&types[supplied.len()..])
            } else {
                None
            };
            let args = es
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    self.eval_expected(e, env, ts.and_then(|a| a.get(i)).and_then(Option::as_deref))
                })
                .collect::<Rt<_>>()?;
            return self.apply(f, args);
        }
        let ty = if matches!(s, "+" | "-" | "*" | "//" | "%") {
            expected.filter(|t| matches!(*t, "n" | "z" | "q"))
        } else {
            None
        };
        let args = es
            .iter()
            .map(|e| {
                if matches!(e, Expr::Int(_)) {
                    self.eval_expected(e, env, ty)
                } else {
                    self.eval(e, env)
                }
            })
            .collect::<Rt<_>>()?;
        self.builtin(s, args)
    }
    fn builtin(&mut self, s: &str, args: Vec<V>) -> Rt<V> {
        if arity(s) != Some(args.len()) {
            return Err(format!(
                "unsupported callable or invalid operand count: {s}"
            ));
        }
        if self.fuel == 0 {
            return Err("prototype execution limit exhausted".into());
        }
        self.fuel -= 1;
        let mut it = args.into_iter();
        let mut next = || it.next().expect("checked arity");
        match s {
            "+" | "-" | "*" | "/" | "//" | "%" | "^" | "=" | "/=" | "<" | "<=" | ">" | ">="
            | "&&" | "||" => broadcast(s, next(), next()),
            "==" => Ok(V::Bool(equal(&next(), &next()))),
            "~" => Ok(V::Bool(!boolean(next())?)),
            "|" => {
                let x = num(next())?;
                Ok(V::Num(Num::new(
                    x.n.checked_abs().ok_or_else(overflow)?,
                    x.d,
                    x.kind,
                )?))
            }
            "-:" => {
                let x = num(next())?;
                Ok(V::Num(Num::new(
                    x.n.checked_neg().ok_or_else(overflow)?,
                    x.d,
                    if x.kind == Kind::N { Kind::Z } else { x.kind },
                )?))
            }
            "@" => self.apply(next(), vec![next()]),
            "@@" => {
                let f = next();
                let V::Tuple(args) = next() else {
                    return Err("@@ expects a tuple bundle as its second operand".into());
                };
                self.apply(f, args)
            }
            "!" => {
                if self.effects == 0 {
                    return Err("execution is only permitted inside an effect block".into());
                }
                self.execute(next())
            }
            "io.print" | "io.println" => Ok(V::Print(Box::new(next()))),
            "csv.read" => match next() {
                V::Str(s) => Ok(V::ReadCsv(s)),
                _ => Err("csv.read requires a string path".into()),
            },
            "?0" => Ok(optional(None)),
            "?1" => Ok(optional(Some(next()))),
            "?+" => match next() {
                V::Opt(x) => Ok(V::Bool(x.is_some())),
                _ => Err("?+ expects an optional operand".into()),
            },
            "?!" => match next() {
                V::Opt(Some(x)) => Ok(*x),
                V::Opt(None) => Err("cannot unwrap ?0".into()),
                _ => Err("?! expects an optional operand".into()),
            },
            ".." => {
                let (a, b) = (num(next())?, num(next())?);
                let k = Num::kind(a.kind, b.kind);
                if k == Kind::Q {
                    return Err("range bounds must be n or z".into());
                }
                let (a, b) = (a.integral()?, b.integral()?);
                if b < a {
                    return Ok(V::Array(Vec::new()));
                }
                let count = b.checked_sub(a).ok_or_else(overflow)?;
                if count >= 100_000 {
                    return Err("prototype range limit (100000 items) exceeded".into());
                }
                Ok(V::Array((a..=b).map(|n| V::Num(Num::int(n, k))).collect()))
            }
            "+/" | "*/" | "&&/" | "||/" | ">/" | "</" | "avg" => reduced(s, next()),
            "pct" => {
                let p = num(next())?;
                if p.n < 0 || p.cmp(Num::int(100, Kind::N)).is_gt() {
                    return Err("percentile must be between 0 and 100".into());
                }
                let mut a = array(next())?
                    .into_iter()
                    .map(num)
                    .collect::<Rt<Vec<_>>>()?;
                if a.is_empty() {
                    return Ok(optional(None));
                }
                a.sort_by(|x, y| x.cmp(*y));
                let h = p
                    .calc("*", Num::int((a.len() - 1) as i128, Kind::N))?
                    .calc("/", Num::int(100, Kind::N))?;
                let i = (h.n / h.d) as usize;
                let x = Num::new(a[i].n, a[i].d, Kind::Q)?;
                let y = if h.n % h.d == 0 {
                    x
                } else {
                    let y = Num::new(a[i + 1].n, a[i + 1].d, Kind::Q)?;
                    x.calc(
                        "+",
                        y.calc("-", x)?
                            .calc("*", Num::new(h.n % h.d, h.d, Kind::Q)?)?,
                    )?
                };
                Ok(optional(Some(V::Num(y))))
            }
            "#" => {
                let n = match next() {
                    V::Array(a) | V::Table(a) => a.len(),
                    V::Dict(d) => d.len(),
                    V::Str(a) => a.chars().count(),
                    _ => return Err("# expects a collection or string".into()),
                };
                Ok(V::Num(Num::int(n as i128, Kind::N)))
            }
            "#[" | "#:" => {
                let a = array(next())?;
                if a.iter().any(|x| !is_row(x)) {
                    return Err("table rows must be tuples or records".into());
                }
                Ok(V::Table(a))
            }
            ":#" => match next() {
                V::Table(a) => Ok(V::Array(a)),
                _ => Err(":# expects a table".into()),
            },
            "@:" => {
                let (f, a) = (next(), next());
                match a {
                    V::Dict(xs) => Ok(V::Dict(
                        xs.into_iter()
                            .map(|(k, v)| Ok((k, self.apply(f.clone(), vec![v])?)))
                            .collect::<Rt<_>>()?,
                    )),
                    v => {
                        let (xs, _) = seq(v)?;
                        Ok(V::Array(
                            xs.into_iter()
                                .map(|v| self.apply(f.clone(), vec![v]))
                                .collect::<Rt<_>>()?,
                        ))
                    }
                }
            }
            "?:" => {
                let (test, xs) = (next(), next());
                let (xs, t) = seq(xs)?;
                let mut out = Vec::new();
                if let V::Array(bs) = test {
                    if bs.len() != xs.len() {
                        return Err("mask length differs from collection length".into());
                    }
                    for (b, x) in bs.into_iter().zip(xs) {
                        if boolean(b)? {
                            out.push(x)
                        }
                    }
                } else {
                    for x in xs {
                        if boolean(self.apply(test.clone(), vec![x.clone()])?)? {
                            out.push(x)
                        }
                    }
                }
                Ok(wrap(out, t))
            }
            "=:" => {
                let (f, xs) = (next(), next());
                let (xs, t) = seq(xs)?;
                let mut groups: Vec<(V, Vec<V>)> = Vec::new();
                if xs.len() > 8192 {
                    return Err("prototype grouping limit (8192 rows) exceeded".into());
                }
                for v in xs {
                    let k = self.apply(f.clone(), vec![v.clone()])?;
                    if let Some((_, g)) = groups.iter_mut().find(|(x, _)| equal(x, &k)) {
                        g.push(v)
                    } else {
                        groups.push((k, vec![v]))
                    }
                }
                Ok(V::Dict(
                    groups.into_iter().map(|(k, g)| (k, wrap(g, t))).collect(),
                ))
            }
            ">:" | "<:" => {
                let (f, a) = (next(), next());
                let (xs, t) = seq(a)?;
                let mut xs = xs
                    .into_iter()
                    .map(|v| Ok((self.apply(f.clone(), vec![v.clone()])?, v)))
                    .collect::<Rt<Vec<_>>>()?;
                if let Some((k, _)) = xs.first() {
                    for (k2, _) in &xs {
                        order(k, k2)?;
                    }
                }
                xs.sort_by(|(a, _), (b, _)| {
                    let c = order(a, b).unwrap_or(Ordering::Equal);
                    if s == ">:" {
                        c
                    } else {
                        c.reverse()
                    }
                });
                Ok(wrap(xs.into_iter().map(|(_, v)| v).collect(), t))
            }
            "+:" => {
                let (mut a, t) = seq(next())?;
                let (b, u) = seq(next())?;
                if t != u {
                    return Err("concatenation requires matching collection kinds".into());
                }
                a.extend(b);
                Ok(wrap(a, t))
            }
            "*:" | "&:" => {
                let (a, t) = seq(next())?;
                let (b, u) = seq(next())?;
                if t != u || s == "&:" && t {
                    return Err("incompatible collection kinds".into());
                }
                if s == "&:" {
                    if a.len() != b.len() {
                        return Err("zip requires equal lengths".into());
                    }
                    return Ok(V::Array(
                        a.into_iter()
                            .zip(b)
                            .map(|(x, y)| V::Tuple(vec![x, y]))
                            .collect(),
                    ));
                }
                if a.len().saturating_mul(b.len()) > 100_000 {
                    return Err("prototype cross-product limit exceeded".into());
                }
                Ok(wrap(
                    a.iter()
                        .flat_map(|x| b.iter().map(move |y| V::Tuple(vec![x.clone(), y.clone()])))
                        .collect(),
                    t,
                ))
            }
            "=." => {
                let xs = array(next())?;
                if xs.len() > 8192 {
                    return Err("prototype distinct limit (8192 items) exceeded".into());
                }
                let mut ys = Vec::new();
                for x in xs {
                    if !ys.iter().any(|y| equal(y, &x)) {
                        ys.push(x)
                    }
                }
                Ok(V::Array(ys))
            }
            ".:" => {
                let (key, a) = (next(), next());
                let x = match a {
                    V::Dict(xs) => {
                        xs.into_iter()
                            .find_map(|(k, v)| if equal(&k, &key) { Some(v) } else { None })
                    }
                    v => {
                        let i = natural(key)?;
                        let (xs, _) = seq(v)?;
                        if i == 0 {
                            None
                        } else {
                            xs.into_iter().nth(i - 1)
                        }
                    }
                };
                Ok(optional(x))
            }
            "[:" => {
                let n = natural(next())?;
                let (mut xs, t) = seq(next())?;
                xs.truncate(n);
                Ok(wrap(xs, t))
            }
            "/:" | "\\:" => {
                let (f, mut a, xs) = (next(), next(), array(next())?);
                let scan = s == "\\:";
                let mut out = if scan { vec![a.clone()] } else { Vec::new() };
                for v in xs {
                    let partial = self.apply(f.clone(), vec![a])?;
                    a = self.apply(partial, vec![v])?;
                    if scan {
                        out.push(a.clone());
                    }
                }
                Ok(if scan { V::Array(out) } else { a })
            }
            s if s.starts_with('.') => select(&s[1..], next()),
            s if s.starts_with(':') => convert(next(), &s[1..]),
            _ => Err(format!("unsupported callable {s}")),
        }
    }
}

fn native_program(program: &Program) -> Rt<Output> {
    let mut e = Engine {
        frames: vec![Frame::default()],
        stdout: String::new(),
        fuel: 1_000_000,
        depth: 0,
        effects: 0,
    };
    for s in &program.defs {
        match s {
            Stmt::Def {
                name,
                params,
                result_type,
                body,
                ..
            } if params.is_empty() => {
                e.put(
                    0,
                    name.clone(),
                    Binding::Pending {
                        body: Rc::new(body.clone()),
                        ty: result_type.clone(),
                        env: 0,
                    },
                );
            }
            Stmt::Def { .. } => {
                let (n, v) = e.available_fun(s, 0).expect("nonempty formal list");
                e.put(0, n, Binding::Ready(v));
            }
            Stmt::Schema { .. } => {}
            Stmt::Effect { .. } => {
                return Err("effect statement outside the anonymous entry".into())
            }
        }
    }
    for s in &program.defs {
        if let Stmt::Def { name, params, .. } = s {
            if params.is_empty() {
                e.resolve(0, name)?;
            }
        }
    }
    if let Some(entry) = &program.entry {
        let act = e.eval(entry, 0)?;
        let v = e.execute(act)?;
        let status = match v {
            V::Unit => 0,
            V::Num(n) if n.kind == Kind::N => u8::try_from(n.integral()?)
                .map_err(|_| "entry exit status must be from 0 to 255")?,
            _ => return Err("entry must return unit or a natural exit status".into()),
        };
        Ok(Output {
            stdout: e.stdout,
            status,
        })
    } else {
        Ok(Output {
            stdout: e.stdout,
            status: 0,
        })
    }
}

/// Evaluate a parsed (and normally checked) program. Explicit CSV actions may read local files.
/// Output is returned to the caller, not written directly to stdout.
pub fn run(program: &Program) -> Result<Output, Error> {
    native_program(program).map_err(|message| Error::new(0, message))
}

const CSV_MAX_BYTES: u64 = 1_048_576;
const CSV_MAX_CELLS: usize = 100_000;
fn read_csv(path: &str) -> Rt<V> {
    let err = |e: std::io::Error| format!("cannot read CSV {path:?}: {e}");
    let m = std::fs::metadata(path).map_err(err)?;
    if !m.is_file() {
        return Err(format!("CSV path {path:?} is not a regular file"));
    }
    if m.len() > CSV_MAX_BYTES {
        return Err(format!("CSV input exceeds {CSV_MAX_BYTES} bytes"));
    }
    let f = std::fs::File::open(path).map_err(err)?;
    if !f.metadata().map_err(err)?.is_file() {
        return Err(format!("CSV path {path:?} is not a regular file"));
    }
    let mut bytes = Vec::new();
    f.take(CSV_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(err)?;
    if bytes.len() as u64 > CSV_MAX_BYTES {
        return Err(format!("CSV input exceeds {CSV_MAX_BYTES} bytes"));
    }
    let s = std::str::from_utf8(&bytes).map_err(|_| format!("CSV {path:?} is not valid UTF-8"))?;
    let grid = parse_csv(s).map_err(|e| format!("CSV {path:?}: {e}"))?;
    Ok(V::Array(
        grid.into_iter()
            .map(|row| V::Array(row.into_iter().map(V::Str).collect()))
            .collect(),
    ))
}
fn parse_csv(s: &str) -> Rt<Vec<Vec<String>>> {
    #[derive(PartialEq, Eq)]
    enum State {
        Start,
        Text,
        Quoted,
        Closed,
    }
    let s = s.strip_prefix('\u{feff}').unwrap_or(s);
    let mut state = State::Start;
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut open = false;
    let mut count = 0usize;
    let mut chars = s.chars().peekable();
    let mut add_field = |row: &mut Vec<String>, field: &mut String| -> Rt<()> {
        count += 1;
        if count > CSV_MAX_CELLS {
            return Err(format!("CSV input exceeds {CSV_MAX_CELLS} cells"));
        }
        row.push(std::mem::take(field));
        Ok(())
    };
    fn add_row(rows: &mut Vec<Vec<String>>, row: &mut Vec<String>) -> Rt<()> {
        if let Some(first) = rows.first() {
            if row.len() != first.len() {
                return Err(format!(
                    "record {} has {} fields; expected {}",
                    rows.len() + 1,
                    row.len(),
                    first.len()
                ));
            }
        }
        rows.push(std::mem::take(row));
        Ok(())
    }
    while let Some(c) = chars.next() {
        open = true;
        if state == State::Quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    state = State::Closed;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            ',' => {
                add_field(&mut row, &mut field)?;
                state = State::Start;
            }
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                add_field(&mut row, &mut field)?;
                add_row(&mut rows, &mut row)?;
                state = State::Start;
                open = false;
            }
            '"' if state == State::Start => state = State::Quoted,
            '"' => {
                return Err(format!(
                    "unexpected quote in record {}, field {}",
                    rows.len() + 1,
                    row.len() + 1
                ))
            }
            _ if state == State::Closed => {
                return Err(format!(
                    "unexpected content after closing quote in record {}, field {}",
                    rows.len() + 1,
                    row.len() + 1
                ))
            }
            _ => {
                field.push(c);
                state = State::Text;
            }
        }
    }
    if state == State::Quoted {
        return Err(format!(
            "unclosed quoted field in record {}, field {}",
            rows.len() + 1,
            row.len() + 1
        ));
    }
    if open {
        add_field(&mut row, &mut field)?;
        add_row(&mut rows, &mut row)?;
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_exact_core() {
        fn q(s: &str) -> Num {
            Num::lit(s).unwrap()
        }
        let a = q("0.125");
        assert_eq!((a.n, a.d), (1, 8));
        assert_eq!(
            q("1")
                .calc("/", q("3"))
                .unwrap()
                .calc("+", q("0.5"))
                .unwrap(),
            Num::new(5, 6, Kind::Q).unwrap()
        );
        assert!(q("1").calc("-", q("2")).is_err());
        assert_eq!(q("-5").calc("//", q("2")).unwrap().n, -3);
        assert_eq!(q("-5").calc("%", q("2")).unwrap().n, 1);
        assert_eq!(q("5").calc("%", q("-2")).unwrap().n, -1);
        assert_eq!(
            Num::new(i128::MAX, i128::MAX - 1, Kind::Q)
                .unwrap()
                .cmp(Num::new(i128::MAX - 1, i128::MAX - 2, Kind::Q).unwrap()),
            Ordering::Less
        );
        assert!(Num::int(i128::MAX, Kind::Z).calc("+", q("1")).is_err());
    }

    #[test]
    fn csv_parser_is_lossless_and_strict() {
        assert_eq!(
            parse_csv("\u{feff}site,value\r\n\"north, west\",\"a\"\"b\r\nc\"\r\nSouth,\r\n")
                .unwrap(),
            vec![
                vec!["site", "value"],
                vec!["north, west", "a\"b\r\nc"],
                vec!["South", ""]
            ]
        );
        assert!(parse_csv("").unwrap().is_empty());
        assert_eq!(parse_csv(",").unwrap(), vec![vec!["", ""]]);
        assert!(parse_csv("x,y\r\nz\r\n").unwrap_err().contains("record 2"));
        assert!(parse_csv("\"x\"z\n")
            .unwrap_err()
            .contains("after closing quote"));
        assert!(parse_csv("\"x").unwrap_err().contains("unclosed"));
        assert!(parse_csv("x\"y\n")
            .unwrap_err()
            .contains("unexpected quote"));
    }

    #[test]
    fn evaluation_uses_lazy_branches_and_lexical_scope() {
        let p = crate::parser::parse(
            r#"
            << "io" : out;
            x := 1;
            local(n:n):n := {
                r := f n;
                f(t:n):n := + t x;
                r
            };
            !{
                f := \(x)-> + x 10;
                ! out.print (? 1b (local 4) (/ 1 0));
                ! out.print (@ f 7);
                ! out.print (@@ &+ (3,4));
            };
        "#,
        )
        .unwrap();
        let o = run(&p).unwrap();
        assert_eq!(o.stdout, "5\n17\n7\n");
        assert_eq!(o.status, 0);
    }
}
