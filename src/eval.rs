//! Reference evaluator. Exact arithmetic is checked and bounded by i128.
use crate::{
    parser::{Expr, Program, Stmt, TypeAnnotation},
    Error,
};
use std::{cmp::Ordering, collections::HashMap, rc::Rc};

fn annotations(ts: &[Option<TypeAnnotation>]) -> Vec<Option<String>> {
    ts.iter()
        .map(|t| t.as_ref().map(|t| t.source().to_owned()))
        .collect()
}
fn schema_annotations(fields: &[(String, TypeAnnotation)]) -> Vec<(String, String)> {
    fields
        .iter()
        .map(|(n, t)| (n.clone(), t.source().to_owned()))
        .collect()
}
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
            _ => (s, if frac { Kind::Q } else { Kind::Z }),
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
    fn real(self) -> f64 {
        self.n as f64 / self.d as f64
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
    Real(f64),
    Str(String),
    Char(char),
    Bool(bool),
    Unit,
    Tuple(Vec<V>),
    Array(Vec<V>),
    Table(Vec<V>, Vec<String>),
    Record(Vec<(String, V)>),
    Dict(Vec<(V, V)>),
    Opt(Option<Box<V>>),
    Fn(Fun),
    Action(Action),
    Print(Box<V>),
    ReadCsv(String, Option<Vec<(String, String)>>),
    WriteCsv(String, Box<V>),
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
    schemas: HashMap<String, Vec<(String, String)>>,
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
        V::Real(x) => x.to_string(),
        V::Num(x) if x.d == 1 => x.n.to_string(),
        V::Num(x) => format!("{}/{}", x.n, x.d),
        V::Bool(b) => if *b { "1b" } else { "0b" }.into(),
        V::Str(s) => s.clone(),
        V::Char(c) => c.to_string(),
        V::Unit => "()".into(),
        V::Tuple(xs) => grouped(xs, "(", ")", ", "),
        V::Array(xs) => grouped(xs, "[", "]", " "),
        V::Table(xs, _) => grouped(xs, "#[", "]", " "),
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
        V::Action(_) | V::Print(_) | V::ReadCsv(..) | V::WriteCsv(..) => "<action>".into(),
    }
}
fn equal(a: &V, b: &V) -> bool {
    match (a, b) {
        (V::Num(a), V::Num(b)) => a.cmp(*b) == Ordering::Equal,
        (V::Real(a), V::Real(b)) => a == b,
        (V::Bool(a), V::Bool(b)) => a == b,
        (V::Str(a), V::Str(b)) => a == b,
        (V::Char(a), V::Char(b)) => a == b,
        (V::Unit, V::Unit) | (V::Opt(None), V::Opt(None)) => true,
        (V::Opt(Some(a)), V::Opt(Some(b))) => equal(a, b),
        (V::Tuple(a), V::Tuple(b)) | (V::Array(a), V::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| equal(a, b))
        }
        (V::Table(a, ca), V::Table(b, cb)) => {
            ca == cb && a.len() == b.len() && a.iter().zip(b).all(|(a, b)| equal(a, b))
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
// Only variants with a lossless canonical key use the index. None of the
// omitted runtime shapes can be equal to one of these scalar variants.
#[derive(PartialEq, Eq, Hash)]
enum ScalarKey {
    Exact(i128, i128),
    Real(u64),
    Str(String),
    Char(char),
    Bool(bool),
    Unit,
    Absent,
}

impl ScalarKey {
    fn from_value(v: &V) -> Option<Self> {
        Some(match v {
            // Normalized numerator/denominator; Kind is intentionally ignored
            // because runtime equality compares n, z, and q by exact value.
            V::Num(n) => Self::Exact(n.n, n.d),
            V::Real(r) if r.is_finite() => Self::Real(if *r == 0.0 { 0 } else { r.to_bits() }),
            V::Str(s) => Self::Str(s.clone()),
            V::Char(c) => Self::Char(*c),
            V::Bool(b) => Self::Bool(*b),
            V::Unit => Self::Unit,
            V::Opt(None) => Self::Absent,
            _ => return None,
        })
    }
}

fn structural_fields(t: &str) -> Option<Vec<(String, String)>> {
    let ts = crate::lexer::lex(t).ok()?;
    let mut tokens = ts.as_slice();
    while matches!((tokens.first(),tokens.last()),(Some(a),Some(b)) if a.text=="("&&b.text==")") {
        tokens = &tokens[1..tokens.len() - 1];
    }
    let [first, inner @ .., last] = tokens else {
        return None;
    };
    if first.text != ".{" || last.text != "}" {
        return None;
    }
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    for next in 0..=inner.len() {
        let text = inner.get(next).map(|t| t.text.as_str());
        if text.is_none() || (text == Some(";") && depth == 0) {
            if next > start {
                let [name, colon, ty @ ..] = &inner[start..next] else {
                    return None;
                };
                if colon.text != ":" || ty.is_empty() {
                    return None;
                }
                out.push((
                    name.text.clone(),
                    t[ty.first()?.start..ty.last()?.end].to_owned(),
                ));
            }
            start = next + 1;
        } else {
            match text? {
                "(" | "[" | ".{" | "{" | "#[" | "#{" => depth += 1,
                ")" | "]" | "}" => depth = depth.checked_sub(1)?,
                _ => {}
            }
        }
    }
    (depth == 0).then_some(out)
}

fn order(a: &V, b: &V) -> Rt<Ordering> {
    match (a, b) {
        (V::Num(a), V::Num(b)) => Ok(a.cmp(*b)),
        (V::Real(a), V::Real(b)) => a.partial_cmp(b).ok_or_else(|| "nonfinite real".into()),
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
fn finite(x: f64) -> Rt<V> {
    if x.is_finite() {
        Ok(V::Real(x))
    } else {
        Err("nonfinite real result".into())
    }
}
fn as_real(v: V) -> Rt<f64> {
    match v {
        V::Real(x) => Ok(x),
        V::Num(x) => Ok(x.real()),
        _ => Err("expected a numeric value".into()),
    }
}
// Collection counts clip to the collection length. A natural wider than
// usize is necessarily past the end, so saturating before the clip is exact.
fn natural(v: V) -> Rt<usize> {
    let n = num(v)?.integral()?;
    if n < 0 {
        return Err("expected a nonnegative integer".into());
    }
    Ok(usize::try_from(n).unwrap_or(usize::MAX))
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
    if t == "r" {
        return finite(as_real(v)?);
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
        (v @ (V::Num(_) | V::Real(_)), Some("r")) => convert(v, "r"),
        (V::Num(_), Some("d")) => Err("d is not implemented; use exact q".into()),
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
fn canonical_builtin(s: &str) -> &str {
    match s {
        "table.agg" => "df.agg",
        "table.extend" => "df.extend",
        s => match s
            .strip_prefix("statistics.")
            .or_else(|| s.strip_prefix("stats."))
        {
            Some("mean") => "avg",
            Some(n @ ("avg" | "pct" | "sd" | "med" | "var")) => n,
            _ => s,
        },
    }
}
fn arity(s: &str) -> Option<usize> {
    Some(match canonical_builtin(s) {
        "+" | "-" | "*" | "/" | "//" | "%" | "^" | "=" | "/=" | "==" | "<" | "<=" | ">" | ">="
        | "&&" | "||" | ".." | "@" | "@@" | "&:" | "@:" | "??" | "?:" | "=:" | ">:" | "<:"
        | "+:" | "*:" | "[:" | ".:" | "pct" | "take" | "drop" | "df.extend" | "csv.write" => 2,
        "/:" | "\\:" | "slice" | "df.agg" => 3,
        "?0" => 0,
        "?1" | "?+" | "?!" | "+/" | "*/" | "&&/" | "||/" | ">/" | "</" | "#" | "avg"
        | "io.print" | "io.println" | "csv.read" | "#[" | "#:" | ":#" | "=." | "~" | "-:" | "|"
        | "!" | "rev" | "flat" | "tr" | "med" | "var" | "sd" | "^/" => 1,
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
            schemas: HashMap::new(),
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
            .and_then(|v| self.contextual(v, expected, env))
            .and_then(|v| typed(v, expected));
        self.depth -= 1;
        r
    }
    fn table_schema(&self, mut env: usize, t: &str) -> Option<Vec<(String, String)>> {
        let n = t.trim().strip_prefix('#')?.trim();
        if n.starts_with(".{") || n.starts_with('(') {
            return structural_fields(n);
        }
        loop {
            if let Some(fields) = self.frames[env].schemas.get(n) {
                return Some(fields.clone());
            }
            env = self.frames[env].parent?;
        }
    }
    fn contextual(&self, v: V, expected: Option<&str>, env: usize) -> Rt<V> {
        let Some(t) = expected else { return Ok(v) };
        match v {
            V::ReadCsv(path, None) => {
                let schema = t
                    .trim()
                    .strip_prefix('!')
                    .and_then(|inner| self.table_schema(env, inner));
                Ok(V::ReadCsv(path, schema))
            }
            V::Table(rows, old) => {
                let Some(schema) = self.table_schema(env, t) else {
                    return Ok(V::Table(rows, old));
                };
                let headers = schema.iter().map(|(name, _)| name.clone()).collect();
                let rows = rows
                    .into_iter()
                    .map(|v| match v {
                        V::Record(fs) => Ok(V::Record(
                            fs.into_iter()
                                .map(|(k, v)| {
                                    let t = schema
                                        .iter()
                                        .find(|(name, _)| *name == k)
                                        .map(|(_, t)| t.as_str());
                                    Ok((k, typed(v, t)?))
                                })
                                .collect::<Rt<_>>()?,
                        )),
                        v => Ok(v),
                    })
                    .collect::<Rt<_>>()?;
                Ok(V::Table(rows, headers))
            }
            v => Ok(v),
        }
    }
    fn eval_inner(&mut self, e: &Expr, env: usize, expected: Option<&str>) -> Rt<V> {
        match e {
            Expr::Int(x) => Ok(V::Num(Num::int(*x, Kind::Z))),
            Expr::Num(s) => {
                if let Some(raw) = s.strip_suffix('r') {
                    finite(raw.parse::<f64>().map_err(|_| "invalid real literal")?)
                } else {
                    Ok(V::Num(Num::lit(s)?))
                }
            }
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
            Expr::TypedLambda(params, types, b) => Ok(V::Fn(Fun {
                code: Code::User {
                    params: params.clone(),
                    types: annotations(types),
                    result: None,
                    body: Rc::new((**b).clone()),
                    env,
                },
                supplied: Vec::new(),
            })),
            Expr::Pipeline(stages) => {
                let mut it = stages.iter().peekable();
                let Some(first) = it.next() else {
                    return Err("empty pipeline".into());
                };
                let mut value = self.eval_expected(
                    first,
                    env,
                    if it.peek().is_none() { expected } else { None },
                )?;
                while let Some(stage) = it.next() {
                    let local = self.frame(Some(env));
                    self.put(local, "$".into(), Binding::Ready(value));
                    value = self.eval_expected(
                        stage,
                        local,
                        if it.peek().is_none() { expected } else { None },
                    )?;
                }
                Ok(value)
            }
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
            Expr::Match(..) => Err("match expressions require typed HIR".into()),
            Expr::Fallible(..) | Expr::Raise(..) | Expr::Propagate(..) | Expr::Recover(..) => {
                Err("typed error handling requires typed HIR".into())
            }
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
                    types: annotations(param_types),
                    result: result_type.as_ref().map(|t| t.source().to_owned()),
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
        for s in steps {
            if let Stmt::Schema { name, fields, .. } = s {
                self.frames[scope]
                    .schemas
                    .insert(name.clone(), schema_annotations(fields));
            }
        }
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
    fn fun_fields(&self, summary: &V) -> Vec<String> {
        fn function(
            engine: &Engine,
            v: &V,
            active: &mut std::collections::HashSet<*const Expr>,
            limit: usize,
        ) -> Vec<String> {
            let V::Fn(Fun {
                code: Code::User {
                    result, body, env, ..
                },
                ..
            }) = v
            else {
                return Vec::new();
            };
            if limit == 0 || !active.insert(Rc::as_ptr(body)) {
                return Vec::new();
            }
            let out = if let Some(schema) = result
                .as_ref()
                .and_then(|t| engine.table_schema(*env, &format!("#{t}")))
            {
                schema.into_iter().map(|(k, _)| k).collect()
            } else {
                from_body(engine, body, Some(*env), active, limit - 1)
            };
            active.remove(&Rc::as_ptr(body));
            out
        }
        fn from_body(
            engine: &Engine,
            e: &Expr,
            env: Option<usize>,
            active: &mut std::collections::HashSet<*const Expr>,
            limit: usize,
        ) -> Vec<String> {
            if limit == 0 {
                return Vec::new();
            }
            match e {
                Expr::Record(fs) | Expr::NamedRecord(_, fs) => {
                    fs.iter().map(|(k, _)| k.clone()).collect()
                }
                // A local block may shadow a helper; still recover a literal
                // record tail but do not consult the outer value bindings.
                Expr::Block(ss, Some(t)) => {
                    from_body(engine, t, env.filter(|_| ss.is_empty()), active, limit - 1)
                }
                Expr::Lambda(_, t) | Expr::TypedLambda(_, _, t) => {
                    from_body(engine, t, env, active, limit - 1)
                }
                Expr::Cases(_, t) => from_body(engine, t, env, active, limit - 1),
                Expr::Call(name, _) => {
                    let mut frame = env;
                    while let Some(i) = frame {
                        if let Some(b) = engine.frames[i].vals.get(name) {
                            return if let Binding::Ready(v) = b {
                                function(engine, v, active, limit - 1)
                            } else {
                                Vec::new()
                            };
                        }
                        frame = engine.frames[i].parent;
                    }
                    Vec::new()
                }
                _ => Vec::new(),
            }
        }
        function(self, summary, &mut std::collections::HashSet::new(), 64)
    }
    fn execute(&mut self, v: V) -> Rt<V> {
        let a = match v {
            V::Print(v) => {
                self.stdout.push_str(&repr(&v));
                self.stdout.push('\n');
                return Ok(V::Unit);
            }
            V::ReadCsv(path, schema) => return read_csv(&path, schema.as_deref()),
            V::WriteCsv(path, t) => {
                write_csv(&path, *t)?;
                return Ok(V::Unit);
            }
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

#[derive(Clone, Eq, PartialEq)]
enum Shape {
    Array,
    Table(Vec<String>),
}
fn seq(v: V) -> Rt<(Vec<V>, Shape)> {
    match v {
        V::Array(xs) => Ok((xs, Shape::Array)),
        V::Table(xs, cols) => Ok((xs, Shape::Table(cols))),
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
fn wrap(xs: Vec<V>, shape: Shape) -> V {
    match shape {
        Shape::Array => V::Array(xs),
        Shape::Table(cols) => V::Table(xs, cols),
    }
}
fn table(v: V) -> Rt<(Vec<V>, Vec<String>)> {
    match v {
        V::Table(xs, cols) => Ok((xs, cols)),
        _ => Err("expected a table".into()),
    }
}
fn record(v: V) -> Rt<Vec<(String, V)>> {
    match v {
        V::Record(fs) => Ok(fs),
        _ => Err("expected a record".into()),
    }
}
fn fields(xs: &[V]) -> Vec<String> {
    match xs.first() {
        Some(V::Record(fs)) => fs.iter().map(|(k, _)| k.clone()).collect(),
        _ => Vec::new(),
    }
}
type NamedColumns = (Vec<String>, Vec<Vec<V>>, Option<usize>);
fn columns(v: V) -> Rt<NamedColumns> {
    let (mut names, mut cols, mut len) = (Vec::new(), Vec::new(), None);
    for (name, col) in record(v)? {
        let items = array(col)?;
        if names.contains(&name) {
            return Err(format!("duplicate column {name}"));
        }
        if len.is_some_and(|n| n != items.len()) {
            return Err("record columns have unequal lengths".into());
        }
        len = Some(items.len());
        names.push(name);
        cols.push(items);
    }
    Ok((names, cols, len))
}
fn from_columns(v: V) -> Rt<V> {
    let (names, cols, count) = columns(v)?;
    let mut rows = (0..count.unwrap_or(0))
        .map(|_| Vec::new())
        .collect::<Vec<_>>();
    for (name, col) in names.iter().zip(cols) {
        for (r, v) in rows.iter_mut().zip(col) {
            r.push((name.clone(), v));
        }
    }
    Ok(V::Table(rows.into_iter().map(V::Record).collect(), names))
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
        V::Table(xs, _) | V::Array(xs) => Ok(V::Array(
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
        if s == "!" && es.len() == 1 {
            if self.effects == 0 {
                return Err("execution is only permitted inside an effect block".into());
            }
            let action_ty = expected.map(|t| format!("! {t}"));
            let a = self.eval_expected(&es[0], env, action_ty.as_deref())?;
            return self.execute(a);
        }
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
        let mut args: Vec<V> = es
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let ty = if matches!(
                    (s, i),
                    ("^", 1) | ("[:", 0) | ("take", 0) | ("drop", 0) | ("slice", 0 | 1) | (".:", 0)
                ) {
                    Some("n")
                } else {
                    ty
                };
                if matches!(e, Expr::Int(_)) {
                    self.eval_expected(e, env, ty)
                } else {
                    self.eval(e, env)
                }
            })
            .collect::<Rt<_>>()?;
        if args.len() == 2
            && matches!(
                s,
                "+" | "-" | "*" | "/" | "//" | "%" | ".." | "=" | "/=" | "<" | "<=" | ">" | ">="
            )
        {
            let hint = |v: &V| match v {
                V::Num(n) => Some(n.kind),
                V::Array(xs) => match xs.first() {
                    Some(V::Num(n)) => Some(n.kind),
                    _ => None,
                },
                _ => None,
            };
            for i in 0..2 {
                if let Expr::Int(n) = es[i] {
                    let peer = hint(&args[1 - i]);
                    let typ = match peer {
                        Some(Kind::N) if n >= 0 => Some("n"),
                        Some(Kind::Q) if !matches!(s, "//" | "%" | "..") => Some("q"),
                        _ => ty,
                    };
                    if let Some(t) = typ {
                        args[i] = typed(args[i].clone(), Some(t))?;
                    }
                }
            }
        }
        let out = self.builtin(s, args)?;
        self.contextual(out, expected, env)
    }
    fn builtin(&mut self, s: &str, args: Vec<V>) -> Rt<V> {
        let s = canonical_builtin(s);
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
                V::Str(s) => Ok(V::ReadCsv(s, None)),
                _ => Err("csv.read requires a string path".into()),
            },
            "csv.write" => match next() {
                V::Str(s) => Ok(V::WriteCsv(s, Box::new(next()))),
                _ => Err("csv.write requires a string path".into()),
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
                    V::Array(a) | V::Table(a, _) => a.len(),
                    V::Dict(d) => d.len(),
                    V::Str(a) => a.chars().count(),
                    _ => return Err("# expects a collection or string".into()),
                };
                Ok(V::Num(Num::int(n as i128, Kind::N)))
            }
            "#[" | "#:" => {
                let v = next();
                if s == "#:" && matches!(v, V::Record(_)) {
                    return from_columns(v);
                }
                let a = array(v)?;
                if a.iter().any(|x| !is_row(x)) {
                    return Err("table rows must be tuples or records".into());
                }
                let cols = fields(&a);
                Ok(V::Table(a, cols))
            }
            ":#" => match next() {
                V::Table(a, _) => Ok(V::Array(a)),
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
                let mut scalar_groups = HashMap::new();
                for v in xs {
                    let k = self.apply(f.clone(), vec![v.clone()])?;
                    let index = if let Some(h) = ScalarKey::from_value(&k) {
                        let n = groups.len();
                        *scalar_groups.entry(h).or_insert(n)
                    } else {
                        groups
                            .iter()
                            .position(|(x, _)| equal(x, &k))
                            .unwrap_or(groups.len())
                    };
                    if let Some((_, g)) = groups.get_mut(index) {
                        g.push(v);
                    } else {
                        groups.push((k, vec![v]));
                    }
                }
                Ok(V::Dict(
                    groups
                        .into_iter()
                        .map(|(k, g)| (k, wrap(g, t.clone())))
                        .collect(),
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
                if (matches!(t, Shape::Array) != matches!(u, Shape::Array))
                    || s == "&:" && !matches!(t, Shape::Array)
                {
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
                    if matches!(t, Shape::Array) {
                        Shape::Array
                    } else {
                        Shape::Table(Vec::new())
                    },
                ))
            }
            "=." => {
                let xs = array(next())?;
                if xs.len() > 8192 {
                    return Err("prototype distinct limit (8192 items) exceeded".into());
                }
                let mut ys = Vec::new();
                let mut scalars = std::collections::HashSet::new();
                for x in xs {
                    let novel = if let Some(h) = ScalarKey::from_value(&x) {
                        scalars.insert(h)
                    } else {
                        !ys.iter().any(|y| equal(y, &x))
                    };
                    if novel {
                        ys.push(x);
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
            "df.extend" => {
                let (names, cols, count) = columns(next())?;
                let (mut rows, mut existing) = table(next())?;
                if count.is_some_and(|n| n != rows.len()) {
                    return Err("extension column length does not match table height".into());
                }
                for name in &names {
                    if existing.contains(name) {
                        return Err(format!("df.extend cannot duplicate existing field {name}"));
                    }
                }
                if rows.iter().any(|v| !matches!(v, V::Record(_))) {
                    return Err("df.extend requires a record table".into());
                }
                for (name, col) in names.iter().zip(cols) {
                    for (row, v) in rows.iter_mut().zip(col) {
                        if let V::Record(fs) = row {
                            fs.push((name.clone(), v));
                        }
                    }
                }
                existing.extend(names);
                Ok(V::Table(rows, existing))
            }
            "df.agg" => {
                let (key, summary, t) = (next(), next(), next());
                let V::Dict(groups) = self.builtin("=:", vec![key, t])? else {
                    unreachable!()
                };
                let empty_fields = self.fun_fields(&summary);
                let mut rows = Vec::new();
                for (key, g) in groups {
                    let partial = self.apply(summary.clone(), vec![key])?;
                    let value = self.apply(partial, vec![g])?;
                    if !matches!(value, V::Record(_)) {
                        return Err("df.agg summarizer must return a record".into());
                    }
                    rows.push(value);
                }
                let names = if rows.is_empty() {
                    empty_fields
                } else {
                    fields(&rows)
                };
                Ok(V::Table(rows, names))
            }
            "med" => self.builtin("pct", vec![V::Num(Num::int(50, Kind::Z)), next()]),
            "^/" => {
                let x = as_real(next())?;
                if x < 0.0 {
                    return Err("square root of a negative number".into());
                }
                finite(x.sqrt())
            }
            "sd" => {
                let xs = array(next())?;
                if xs.is_empty() {
                    return Ok(optional(None));
                }
                let mut mean = 0.0;
                let mut m2 = 0.0;
                let n = xs.len() as f64;
                for (i, x) in xs.into_iter().enumerate() {
                    let x = as_real(x)?;
                    let diff = x - mean;
                    mean += diff / ((i + 1) as f64);
                    m2 += diff * (x - mean);
                }
                if !m2.is_finite() {
                    return Err("real standard deviation overflow".into());
                }
                Ok(optional(Some(finite((m2.max(0.0) / n).sqrt())?)))
            }
            "var" => {
                let xs = array(next())?;
                if xs.is_empty() {
                    return Ok(optional(None));
                }
                let count = Num::int(xs.len() as i128, Kind::Z);
                let mut sum = Num::int(0, Kind::Q);
                let mut nums = Vec::with_capacity(xs.len());
                for v in xs {
                    let n = num(v)?;
                    sum = sum.calc("+", n)?;
                    nums.push(n);
                }
                let mean = sum.calc("/", count)?;
                let mut squares = Num::int(0, Kind::Q);
                for n in nums {
                    let t = n.calc("-", mean)?;
                    squares = squares.calc("+", t.calc("*", t)?)?;
                }
                Ok(optional(Some(V::Num(squares.calc("/", count)?))))
            }
            "flat" => {
                let mut out = Vec::new();
                for v in array(next())? {
                    out.extend(array(v)?);
                }
                Ok(V::Array(out))
            }
            "tr" => {
                let a = array(next())?;
                let mut rows = a.into_iter().map(array).collect::<Rt<Vec<_>>>()?;
                let cols = rows.first().map_or(0, Vec::len);
                if rows.iter().any(|r| r.len() != cols) {
                    return Err("transpose requires a rectangular sequence".into());
                }
                let mut transposed = (0..cols)
                    .map(|_| Vec::with_capacity(rows.len()))
                    .collect::<Vec<_>>();
                for row in rows.drain(..) {
                    for (out, v) in transposed.iter_mut().zip(row) {
                        out.push(v);
                    }
                }
                Ok(V::Array(transposed.into_iter().map(V::Array).collect()))
            }
            "rev" => match next() {
                V::Str(s) => Ok(V::Str(s.chars().rev().collect())),
                v => {
                    let (mut a, t) = seq(v)?;
                    a.reverse();
                    Ok(wrap(a, t))
                }
            },
            "drop" | "slice" | "take" | "[:" => {
                let n = natural(next())?;
                let count = if s == "slice" { natural(next())? } else { 0 };
                let transform = |len: usize| -> Rt<(usize, usize)> {
                    Ok(match s {
                        "drop" => (n.min(len), len),
                        "slice" => {
                            if n == 0 {
                                return Err("slice start is one-based".into());
                            }
                            let start = (n - 1).min(len);
                            (start, start.saturating_add(count).min(len))
                        }
                        _ => (0, n.min(len)),
                    })
                };
                match next() {
                    V::Str(s) => {
                        let n = s.chars().count();
                        let (a, b) = transform(n)?;
                        Ok(V::Str(s.chars().skip(a).take(b - a).collect()))
                    }
                    v => {
                        let (xs, t) = seq(v)?;
                        let (a, b) = transform(xs.len())?;
                        Ok(wrap(xs.into_iter().skip(a).take(b - a).collect(), t))
                    }
                }
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

fn evaluate_program(program: &Program) -> Result<(String, V), Error> {
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
                        ty: result_type.as_ref().map(|t| t.source().to_owned()),
                        env: 0,
                    },
                );
            }
            Stmt::Def { .. } => {
                let (n, v) = e.available_fun(s, 0).expect("nonempty formal list");
                e.put(0, n, Binding::Ready(v));
            }
            Stmt::Sum { start, .. } => {
                return Err(Error::new(*start, "sum declarations require typed HIR"))
            }
            Stmt::Schema { name, fields, .. } => {
                e.frames[0]
                    .schemas
                    .insert(name.clone(), schema_annotations(fields));
            }
            Stmt::Effect { start, .. } => {
                return Err(Error::new(
                    *start,
                    "effect statement outside the anonymous entry",
                ));
            }
        }
    }
    for s in &program.defs {
        if let Stmt::Def {
            name,
            params,
            start,
            ..
        } = s
        {
            if params.is_empty() {
                e.resolve(0, name)
                    .map_err(|message| Error::new(*start, message))?;
            }
        }
    }
    let v = if let Some(entry) = &program.entry {
        e.eval(entry, 0)
            .and_then(|act| e.execute(act))
            .map_err(|message| Error::new(program.entry_start.unwrap_or(0), message))?
    } else {
        V::Unit
    };
    Ok((e.stdout, v))
}

fn native_program(program: &Program) -> Result<Output, Error> {
    let (stdout, v) = evaluate_program(program)?;
    let status = match v {
        V::Unit => 0,
        V::Num(n) if n.d == 1 => u8::try_from(n.n).map_err(|_| {
            Error::new(
                program.entry_start.unwrap_or(0),
                "entry exit status must be from 0 to 255",
            )
        })?,
        _ => {
            return Err(Error::new(
                program.entry_start.unwrap_or(0),
                "entry must return unit or a natural exit status",
            ))
        }
    };
    Ok(Output { stdout, status })
}

pub struct ReplOutput {
    pub value: String,
    pub stdout: String,
}
pub fn view_entry(program: &Program) -> Result<ReplOutput, Error> {
    let (stdout, v) = evaluate_program(program)?;
    let value = match v {
        V::Table(rows, columns) => format_table(&rows, &columns),
        v => repl_repr(&v),
    };
    Ok(ReplOutput { stdout, value })
}

fn repl_repr(v: &V) -> String {
    match v {
        V::Str(s) => format!("{s:?}"),
        V::Char(c) => format!("{c:?}"),
        V::Array(xs) => format!(
            "[{}]",
            xs.iter().map(repl_repr).collect::<Vec<_>>().join(" ")
        ),
        V::Tuple(xs) => format!(
            "({})",
            xs.iter().map(repl_repr).collect::<Vec<_>>().join(", ")
        ),
        V::Record(fs) => format!(
            ".{{ {} }}",
            fs.iter()
                .map(|(k, v)| format!("{k} := {}", repl_repr(v)))
                .collect::<Vec<_>>()
                .join("; ")
        ),
        V::Dict(fs) => format!(
            "#{{ {} }}",
            fs.iter()
                .map(|(k, v)| format!("{} : {}", repl_repr(k), repl_repr(v)))
                .collect::<Vec<_>>()
                .join("; ")
        ),
        V::Opt(Some(v)) => format!("?1 {}", repl_repr(v)),
        v => repr(v)
            .chars()
            .flat_map(|c| {
                if c.is_control() {
                    c.escape_default().collect::<Vec<_>>()
                } else {
                    vec![c]
                }
            })
            .collect(),
    }
}
fn format_table(rows: &[V], known_columns: &[String]) -> String {
    let headers = if !known_columns.is_empty() {
        known_columns.to_vec()
    } else if let Some(V::Tuple(xs)) = rows.first() {
        (1..=xs.len()).map(|i| format!(".{i}")).collect()
    } else {
        return format!("#[ ]  ({} rows)", rows.len());
    };
    let safe = |text: String| -> String {
        let shortened = text.chars().take(32).collect::<String>();
        let mut visible = String::new();
        for c in shortened.chars() {
            if c.is_control() {
                visible.extend(c.escape_default());
            } else {
                visible.push(c);
            }
        }
        if shortened.len() < text.len() {
            visible.push_str("...")
        }
        visible
    };
    let mut view = vec![headers.iter().map(|x| safe(x.clone())).collect::<Vec<_>>()];
    for r in rows.iter().take(12) {
        let xs = match r {
            V::Record(fs) => headers
                .iter()
                .map(|name| {
                    fs.iter()
                        .find(|(f, _)| f == name)
                        .map(|(_, v)| safe(repr(v)))
                        .unwrap_or_else(|| "-".into())
                })
                .collect(),
            V::Tuple(xs) => xs.iter().map(|v| safe(repr(v))).collect(),
            _ => return repr(&V::Table(rows.to_vec(), known_columns.to_vec())),
        };
        view.push(xs);
    }
    let widths = (0..headers.len())
        .map(|i| {
            view.iter()
                .filter_map(|row| row.get(i))
                .map(|x| x.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect::<Vec<_>>();
    let mut out = Vec::new();
    for (i, r) in view.into_iter().enumerate() {
        out.push(
            r.into_iter()
                .zip(&widths)
                .map(|(x, w)| format!("{x:<width$}", width = *w))
                .collect::<Vec<_>>()
                .join(" | "),
        );
        if i == 0 {
            out.push(
                widths
                    .iter()
                    .map(|w| "-".repeat(*w))
                    .collect::<Vec<_>>()
                    .join("-+-"),
            )
        }
    }
    if rows.len() > 12 {
        out.push(format!("... ({} rows total)", rows.len()))
    }
    out.join("\n")
}

/// Evaluate a parsed (and normally checked) program. Explicit CSV actions may read local files.
/// Output is returned to the caller, not written directly to stdout.
pub fn run(program: &Program) -> Result<Output, Error> {
    native_program(program)
}

fn parse_cell(s: &str, t: &str) -> Rt<V> {
    let t = t.trim();
    if let Some(sub) = t.strip_prefix('?') {
        return if s.is_empty() {
            Ok(optional(None))
        } else {
            Ok(optional(Some(parse_cell(s, sub)?)))
        };
    }
    match t {
        "r" => finite(s.parse::<f64>().map_err(|_| "expected a finite real")?),
        "s" => Ok(V::Str(s.to_owned())),
        "c" => {
            let mut chars = s.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Ok(V::Char(c)),
                _ => Err("expected a single character".into()),
            }
        }
        "b" => match s {
            "true" | "1b" => Ok(V::Bool(true)),
            "false" | "0b" => Ok(V::Bool(false)),
            _ => Err("expected true, false, 1b, or 0b".into()),
        },
        "n" | "z" | "q" => {
            let integer = |x: &str| {
                x.parse::<i128>()
                    .map_err(|_| format!("expected a bounded {t} value"))
            };
            let value = if t == "q" {
                if let Some((n, d)) = s.split_once('/') {
                    Num::new(integer(n)?, integer(d)?, Kind::Q)?
                } else if s.is_empty()
                    || s.bytes()
                        .any(|b| !matches!(b, b'0'..=b'9' | b'-' | b'+' | b'.'))
                {
                    return Err("expected a decimal or rational".into());
                } else {
                    Num::lit(s)?
                }
            } else {
                Num::int(integer(s)?, Kind::Z)
            };
            convert(V::Num(value), t)
        }
        _ => Err(format!("CSV column type {t} is not supported")),
    }
}
fn read_csv(path: &str, schema: Option<&[(String, String)]>) -> Rt<V> {
    let s = crate::csv::read(path)?;
    if let Some(schema) = schema {
        let headers = schema.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
        let rows =
            crate::csv::typed_rows(&s, &headers).map_err(|e| format!("CSV {path:?}: {e}"))?;
        let rows = rows
            .into_iter()
            .enumerate()
            .map(|(i, row)| {
                let fs = schema
                    .iter()
                    .zip(row)
                    .map(|((name, t), s)| {
                        let v = parse_cell(&s, t).map_err(|e| {
                            format!("CSV {path:?}: row {}, column {name}: {e}", i + 2)
                        })?;
                        Ok((name.clone(), v))
                    })
                    .collect::<Rt<_>>()?;
                Ok(V::Record(fs))
            })
            .collect::<Rt<_>>()?;
        Ok(V::Table(rows, headers))
    } else {
        Ok(V::Array(
            crate::csv::decode(&s)
                .map_err(|e| format!("CSV {path:?}: {e}"))?
                .into_iter()
                .map(|row| V::Array(row.into_iter().map(V::Str).collect()))
                .collect(),
        ))
    }
}
fn csv_cell(v: &V) -> Rt<String> {
    match v {
        V::Str(s) => Ok(s.clone()),
        V::Char(c) => Ok(c.to_string()),
        V::Num(_) | V::Real(_) => Ok(repr(v)),
        V::Bool(b) => Ok(b.to_string()),
        V::Opt(None) => Ok(String::new()),
        V::Opt(Some(x)) => {
            let cell = csv_cell(x)?;
            if cell.is_empty() {
                Err("present optional cell encodes as blank and would be read as ?0".into())
            } else {
                Ok(cell)
            }
        }
        _ => Err("unsupported CSV output cell type".into()),
    }
}
fn write_csv(path: &str, t: V) -> Rt<()> {
    let (rows, headers) = table(t)?;
    if headers.is_empty() {
        return Err("CSV write needs a table with named columns".into());
    }
    let mut out = vec![headers.clone()];
    for (i, r) in rows.into_iter().enumerate() {
        let fs = record(r)?;
        let line = headers
            .iter()
            .map(|name| {
                let v = fs
                    .iter()
                    .find(|(k, _)| k == name)
                    .ok_or_else(|| format!("CSV row {}, column {name} missing", i + 2))?;
                csv_cell(&v.1).map_err(|e| format!("CSV row {}, column {name}: {e}", i + 2))
            })
            .collect::<Rt<_>>()?;
        out.push(line);
    }
    crate::csv::write(path, &out)
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
        assert!(q("1n").calc("-", q("2n")).is_err());
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
            crate::csv::decode(
                "\u{feff}site,value\r\n\"north, west\",\"a\"\"b\r\nc\"\r\nSouth,\r\n"
            )
            .unwrap(),
            vec![
                vec!["site", "value"],
                vec!["north, west", "a\"b\r\nc"],
                vec!["South", ""]
            ]
        );
        assert!(crate::csv::decode("").unwrap().is_empty());
        assert_eq!(crate::csv::decode(",").unwrap(), vec![vec!["", ""]]);
        assert!(crate::csv::decode("x,y\r\nz\r\n")
            .unwrap_err()
            .contains("record 2"));
        assert!(crate::csv::decode("\"x\"z\n")
            .unwrap_err()
            .contains("after closing quote"));
        assert!(crate::csv::decode("\"x").unwrap_err().contains("unclosed"));
        assert!(crate::csv::decode("x\"y\n")
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
    #[test]
    fn pipeline_evaluates_once_and_stage_bindings_are_lexical() {
        let p = crate::parser::parse(
            r#"
            << "io";
            !{
                x := |{ ! !{ ! io.print "once"; 4 }; + $ $; @ (\(y:z)-> * y 2) $ };
                ! io.print x;
            };
        "#,
        )
        .unwrap();
        assert_eq!(run(&p).unwrap().stdout, "once\n16\n");
    }

    #[test]
    fn table_metadata_survives_empty_columns_and_grouping() {
        let p = crate::parser::parse(
            r#"
            << "df";
            r :: { station:s; value:q; };
            summary(k:s,t:#r) := .{ station:=k; count:=# t; };
            !{
                t:#r := #: .{ station:=[]; value:=[]; };
                df.agg &.station &summary t
            };
        "#,
        )
        .unwrap();
        let res = view_entry(&p).unwrap();
        assert_eq!(res.value, "station | count\n--------+------");
        assert!(res.stdout.is_empty());
    }

    #[test]
    fn runtime_shape_errors_are_checked() {
        let p = crate::parser::parse(r#"t := #: .{ a:=[1]; b:=[2 3]; };"#).unwrap();
        assert!(run(&p)
            .err()
            .unwrap()
            .to_string()
            .contains("unequal lengths"));
        let c =
            crate::parser::parse(r#" << "df"; t := df.extend .{ a:=[2]; } (#: .{ a:=[1]; }); "#)
                .unwrap();
        assert!(run(&c)
            .err()
            .unwrap()
            .to_string()
            .contains("duplicate existing field"));
    }

    #[test]
    fn convenience_operations_preserve_order() {
        let p = crate::parser::parse(
            r#"
            << "io";
            !{
                ! io.print slice 2 2 [10 20 30 40];
                ! io.print tr [[1 2] [3 4]];
                ! io.print var [1 3];
                ! io.print - 1 2;
                ! io.print ^/ 9;
            };
        "#,
        )
        .unwrap();
        assert_eq!(
            run(&p).unwrap().stdout,
            "[20 30]\n[[1 3] [2 4]]\n?1 1\n-1\n3\n"
        );
    }
    #[test]
    fn indexed_grouping_and_distinct_preserve_semantic_equality_and_order() {
        let mut e = Engine {
            frames: vec![Frame::default()],
            stdout: String::new(),
            fuel: 1_000_000,
            depth: 0,
            effects: 0,
        };
        let n = V::Num(Num::int(7, Kind::N));
        let q = V::Num(Num::new(14, 2, Kind::Q).unwrap());
        let structural = V::Tuple(vec![n.clone()]);
        let same = V::Tuple(vec![q.clone()]);
        let xs = vec![
            structural.clone(),
            n.clone(),
            V::Real(-0.0),
            q,
            V::Real(0.0),
            same,
            V::Real(7.0),
            V::Bool(false),
        ];
        let distinct = e.builtin("=.", vec![V::Array(xs.clone())]).unwrap();
        let V::Array(values) = distinct else {
            panic!("expected sequence")
        };
        assert!(
            matches!(&values[..],[V::Tuple(_),V::Num(Num{kind:Kind::N,..}),V::Real(z),V::Real(r),V::Bool(false)] if z.is_sign_negative()&&*r==7.0)
        );
        let rows = xs
            .into_iter()
            .enumerate()
            .map(|(i, k)| V::Tuple(vec![k, V::Num(Num::int(i as i128, Kind::N))]))
            .collect();
        let v = e.builtin("=:", vec![known(".1"), V::Array(rows)]).unwrap();
        let V::Dict(groups) = v else {
            panic!("expected ordered groups")
        };
        assert_eq!(groups.len(), 5);
        assert!(equal(&groups[0].0, &structural) && equal(&groups[1].0, &n));
        let indices = groups
            .into_iter()
            .map(|(_, v)| {
                array(v)
                    .unwrap()
                    .into_iter()
                    .map(|t| num(select("2", t).unwrap()).unwrap().n)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            indices,
            vec![vec![0, 5], vec![1, 3], vec![2, 4], vec![6], vec![7]]
        );
    }

    #[test]
    fn structural_schema_and_present_blank_csv_are_unambiguous() {
        assert_eq!(
            structural_fields(".{station : s ; value : ? q ; valid : b ; }").unwrap(),
            vec![
                ("station".into(), "s".into()),
                ("value".into(), "? q".into()),
                ("valid".into(), "b".into())
            ]
        );
        assert!(csv_cell(&optional(Some(V::Str(String::new()))))
            .unwrap_err()
            .contains("would be read as ?0"));
        assert_eq!(csv_cell(&optional(None)).unwrap(), "");
    }
    #[test]
    fn runtime_errors_have_at_least_the_entry_or_initializer_span() {
        let source = "a := 1;\nb := / 1 0;\n!{ (); };";
        let p = crate::parser::parse(source).unwrap();
        assert_eq!(run(&p).err().unwrap().start, source.find("b :=").unwrap());
        let source = "a := 1;\n!{ / 1 0 };";
        let p = crate::parser::parse(source).unwrap();
        assert_eq!(run(&p).err().unwrap().start, source.find("!{").unwrap());
    }
    #[test]
    fn collection_counts_wider_than_usize_clip_naturally() {
        let p = crate::parser::parse(
            r#"
            << "io";
            !{
                ! io.print take 18446744073709551616 [1 2];
                ! io.print drop 18446744073709551616 "xy";
                ! io.print slice 1 18446744073709551616 [3 4];
                ! io.print .: 18446744073709551616 [3 4];
            };
        "#,
        )
        .unwrap();
        assert_eq!(run(&p).unwrap().stdout, "[1 2]\n\n[3 4]\n?0\n");
    }

    #[test]
    fn empty_group_headers_follow_direct_helper_without_invoking_it() {
        let p = crate::parser::parse(
            r#"
            << "df";
            helper(k:s) := .{ station:=k; x:=/ 1 0; };
            summary(k:s,t:#.{ station:s; }) := helper k;
            !{
                t:#.{ station:s; } := #: .{ station:=[]; };
                df.agg &.station &summary t
            };
        "#,
        )
        .unwrap();
        assert_eq!(view_entry(&p).unwrap().value, "station | x\n--------+--");
    }
}
