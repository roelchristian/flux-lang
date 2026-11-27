//! Executable core for owned typed HIR. This backend never dispatches on surface AST.
use crate::{
    eval::Output,
    hir::{
        CoreOp, Definition, ExprId, ExprKind, HirProgram, LiftMode, Literal, Span, Step,
        TableSchema, Target,
    },
    resolve::SymbolId,
    types::{Prim, Type},
    Error,
};
use std::{cmp::Ordering, collections::HashMap, rc::Rc};

type R<T> = Result<T, Error>;
type NumericResult<T> = Result<T, &'static str>;
const OVERFLOW: &str = "exact arithmetic exceeds the prototype's i128 bound";
const DIV_BY_ZERO: &str = "division by zero";
const COLLECTION_LIMIT: usize = 100_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Domain {
    Natural,
    Integer,
    Rational,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Exact {
    n: i128,
    d: i128,
    domain: Domain,
}
impl Exact {
    fn integer(n: i128, domain: Domain) -> Self {
        Self { n, d: 1, domain }
    }
    fn new(n: i128, d: i128, domain: Domain) -> NumericResult<Self> {
        if d == 0 {
            return Err(DIV_BY_ZERO);
        }
        if n == 0 {
            return Ok(Self::integer(0, domain));
        }
        let g = gcd(n.unsigned_abs(), d.unsigned_abs());
        let (n, d) = if g == 1_u128 << 127 {
            (1, 1)
        } else {
            (n / g as i128, d / g as i128)
        };
        let (n, d) = if d < 0 {
            (
                n.checked_neg().ok_or(OVERFLOW)?,
                d.checked_neg().ok_or(OVERFLOW)?,
            )
        } else {
            (n, d)
        };
        Ok(Self { n, d, domain })
    }
    fn common(a: Domain, b: Domain) -> Domain {
        use Domain::*;
        match (a, b) {
            (Rational, _) | (_, Rational) => Rational,
            (Integer, _) | (_, Integer) => Integer,
            _ => Natural,
        }
    }
    fn integral(self) -> NumericResult<i128> {
        if self.d == 1 {
            Ok(self.n)
        } else {
            Err("expected an integral value")
        }
    }
    /// Round the exact quotient once, to nearest binary64, ties to even.
    fn real(self) -> f64 {
        if self.n == 0 {
            return 0.0;
        }
        let n = self.n.unsigned_abs();
        let d = self.d as u128;
        let mut exponent = d.leading_zeros() as i32 - n.leading_zeros() as i32;
        if if exponent >= 0 {
            n < (d << exponent)
        } else {
            (n << -exponent) < d
        } {
            exponent -= 1;
        }
        let (scaled_n, scaled_d) = if exponent >= 0 {
            (n, d << exponent)
        } else {
            (n << -exponent, d)
        };
        let mut rem = scaled_n - scaled_d;
        let mut significand = 1u64;
        for _ in 0..52 {
            significand <<= 1;
            let complement = scaled_d - rem;
            if rem >= complement {
                significand |= 1;
                rem -= complement;
            } else {
                rem *= 2;
            }
        }
        let complement = scaled_d - rem;
        if rem > complement || (rem == complement && significand & 1 == 1) {
            significand += 1;
            if significand == 1u64 << 53 {
                significand >>= 1;
                exponent += 1;
            }
        }
        let sign = u64::from(self.n < 0) << 63;
        let exponent = ((exponent + 1023) as u64) << 52;
        f64::from_bits(sign | exponent | (significand & ((1u64 << 52) - 1)))
    }

    /// Convert the *stored* binary64 value to a reduced, bounded exact fraction.
    fn from_real(value: f64) -> NumericResult<Self> {
        if !value.is_finite() {
            return Err("nonfinite real value");
        }
        let bits = value.to_bits();
        let exponent = ((bits >> 52) & 0x7ff) as i32;
        let mantissa = bits & ((1u64 << 52) - 1);
        let (mut m, mut shift) = if exponent == 0 {
            (mantissa, -1074)
        } else {
            (mantissa | (1u64 << 52), exponent - 1075)
        };
        if m == 0 {
            return Ok(Self::integer(0, Domain::Rational));
        }
        if shift < 0 {
            let cancel = (m.trailing_zeros() as i32).min(-shift);
            m >>= cancel;
            shift += cancel;
        }
        let (magnitude, denominator) = if shift < 0 {
            if -shift >= 127 {
                return Err(OVERFLOW);
            }
            (u128::from(m), 1i128 << -shift)
        } else {
            let magnitude = u128::from(m);
            if shift >= 128 || magnitude > (u128::MAX >> shift) {
                return Err(OVERFLOW);
            }
            (magnitude << shift, 1)
        };
        let signed = if bits >> 63 != 0 {
            if magnitude == 1u128 << 127 {
                i128::MIN
            } else {
                -i128::try_from(magnitude).map_err(|_| OVERFLOW)?
            }
        } else {
            i128::try_from(magnitude).map_err(|_| OVERFLOW)?
        };
        Self::new(signed, denominator, Domain::Rational)
    }
    fn as_domain(self, domain: Domain) -> NumericResult<Exact> {
        if domain == Domain::Natural && (self.n < 0 || self.d != 1) {
            return Err("cannot represent value as n");
        }
        if domain == Domain::Integer && self.d != 1 {
            return Err("cannot represent value as z");
        }
        Ok(Self { domain, ..self })
    }
    fn compute(self, op: CoreOp, b: Self, chosen: Option<Domain>) -> NumericResult<Self> {
        use CoreOp::*;
        use Domain::*;
        let domain = chosen.unwrap_or(Self::common(self.domain, b.domain));
        let mul = |a: i128, b: i128| a.checked_mul(b).ok_or(OVERFLOW);
        match op {
            Add | Sub => {
                let g = gcd(self.d as u128, b.d as u128) as i128;
                let a = mul(self.n, b.d / g)?;
                let c = mul(b.n, self.d / g)?;
                let n = if op == Add {
                    a.checked_add(c)
                } else {
                    a.checked_sub(c)
                }
                .ok_or(OVERFLOW)?;
                if domain == Natural && n < 0 {
                    return Err("natural subtraction underflow");
                }
                Self::new(n, mul(self.d / g, b.d)?, domain)
            }
            Mul => {
                let g = gcd(self.n.unsigned_abs(), b.d as u128) as i128;
                let h = gcd(b.n.unsigned_abs(), self.d as u128) as i128;
                Self::new(mul(self.n / g, b.n / h)?, mul(self.d / h, b.d / g)?, domain)
            }
            Div => {
                if b.n == 0 {
                    return Err(DIV_BY_ZERO);
                }
                if self.n == 0 {
                    return Ok(Self::integer(0, Rational));
                }
                let g = gcd(self.n.unsigned_abs(), b.n.unsigned_abs());
                let (a, c) = if g == 1u128 << 127 {
                    (1, 1)
                } else {
                    let g = g as i128;
                    (self.n / g, b.n / g)
                };
                let h = gcd(self.d as u128, b.d as u128) as i128;
                Self::new(mul(a, b.d / h)?, mul(self.d / h, c)?, Rational)
            }
            Quotient | Remainder => {
                let a = self.integral()?;
                let b = b.integral()?;
                if b == 0 {
                    return Err(DIV_BY_ZERO);
                }
                let q = a.checked_div(b).ok_or(OVERFLOW)?;
                let r = a.checked_rem(b).ok_or(OVERFLOW)?;
                let opposite = r != 0 && (r < 0) != (b < 0);
                let n = match (op, opposite) {
                    (Quotient, true) => q.checked_sub(1).ok_or(OVERFLOW)?,
                    (Quotient, false) => q,
                    (_, true) => r.checked_add(b).ok_or(OVERFLOW)?,
                    (_, false) => r,
                };
                Ok(Self::integer(n, domain))
            }
            Pow => {
                let exp = b.integral()?;
                if exp < 0 || b.domain != Natural {
                    return Err("power requires a natural exponent");
                }
                let mut a = self;
                let mut out = Self::integer(1, domain);
                let mut e = exp as u128;
                while e != 0 {
                    if e & 1 == 1 {
                        out = out.compute(Mul, a, Some(domain))?;
                    }
                    e >>= 1;
                    if e != 0 {
                        a = a.compute(Mul, a, Some(domain))?;
                    }
                }
                Ok(out)
            }
            _ => Err("invalid exact operation in typed HIR"),
        }
    }
    fn order(self, b: Self) -> Ordering {
        fn positive(mut an: u128, mut ad: u128, mut bn: u128, mut bd: u128) -> Ordering {
            let mut reverse = false;
            loop {
                let mut c = (an / ad).cmp(&(bn / bd));
                let (ar, br) = (an % ad, bn % bd);
                if c == Ordering::Equal && (ar == 0 || br == 0) {
                    c = ar.cmp(&br)
                }
                if c != Ordering::Equal || ar == 0 {
                    return if reverse { c.reverse() } else { c };
                }
                (an, ad, bn, bd) = (ad, ar, bd, br);
                reverse = !reverse;
            }
        }
        match (self.n < 0, b.n < 0) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (negative, _) => {
                let c = positive(
                    self.n.unsigned_abs(),
                    self.d as u128,
                    b.n.unsigned_abs(),
                    b.d as u128,
                );
                if negative {
                    c.reverse()
                } else {
                    c
                }
            }
        }
    }
}
fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b)
    }
    a
}
fn domain(ty: &Type) -> Option<Domain> {
    match ty {
        Type::Prim(Prim::N) => Some(Domain::Natural),
        Type::Prim(Prim::Z) => Some(Domain::Integer),
        Type::Prim(Prim::Q) => Some(Domain::Rational),
        Type::Seq(inner) => domain(inner),
        _ => None,
    }
}
fn parse_exact(s: &str, ty: &Type) -> NumericResult<Exact> {
    let (raw, chosen) = match s.as_bytes().last() {
        Some(b'n') => (&s[..s.len() - 1], Domain::Natural),
        Some(b'z') => (&s[..s.len() - 1], Domain::Integer),
        Some(b'q') => (&s[..s.len() - 1], Domain::Rational),
        _ => (
            s,
            domain(ty).unwrap_or(if s.contains('.') {
                Domain::Rational
            } else {
                Domain::Integer
            }),
        ),
    };
    let parse = |s: &str| s.parse::<i128>().map_err(|_| OVERFLOW);
    let value = if let Some((left, right)) = raw.split_once('.') {
        if chosen != Domain::Rational {
            return Err("fractional literal requires q in this backend");
        }
        let r = right.trim_end_matches('0');
        if r.is_empty() {
            Exact::integer(parse(left)?, chosen)
        } else {
            let d = 10_i128
                .checked_pow(r.len().try_into().map_err(|_| OVERFLOW)?)
                .ok_or(OVERFLOW)?;
            Exact::new(parse(&format!("{left}{r}"))?, d, chosen)?
        }
    } else {
        Exact::integer(parse(raw)?, chosen)
    };
    value.as_domain(chosen)
}

#[derive(Clone)]
enum V {
    Exact(Exact),
    Real(f64),
    Bool(bool),
    String(Rc<str>),
    Char(char),
    Unit,
    Tuple(Rc<[V]>),
    Sequence(Rc<[V]>),
    Record(Rc<[(Box<str>, V)]>),
    Table(Rc<[V]>, Rc<TableSchema>),
    Dict(Rc<[(V, V)]>),
    Optional(Option<Rc<V>>),
    Fun(Rc<Fun>),
    Action(Rc<Action>),
}
#[derive(Clone)]
enum Function {
    User {
        formals: Rc<[SymbolId]>,
        body: ExprId,
        env: usize,
    },
    Core(CoreOp, Type, Option<Rc<TableSchema>>),
    Projection(Box<str>),
}
#[derive(Clone)]
struct Fun {
    code: Function,
    bound: Vec<V>,
}
#[derive(Clone)]
enum Action {
    Block { expr: ExprId, env: usize },
    Print(V),
    CsvRead(Rc<str>, Option<Rc<TableSchema>>),
    CsvWrite(Rc<str>, V),
}
#[derive(Default)]
struct Frame {
    parent: Option<usize>,
    ready: HashMap<SymbolId, V>,
}
struct Eval<'a> {
    p: &'a HirProgram,
    frames: Vec<Frame>,
    stdout: String,
    fuel: usize,
    depth: u16,
    effects: u16,
}
fn error(span: Span, message: impl Into<String>) -> Error {
    Error::new(span.start, message)
}
fn repr(v: &V) -> String {
    match v {
        V::Exact(n) if n.d == 1 => n.n.to_string(),
        V::Exact(n) => format!("{}/{}", n.n, n.d),
        V::Real(n) => n.to_string(),
        V::Bool(b) => if *b { "1b" } else { "0b" }.into(),
        V::String(s) => s.to_string(),
        V::Char(c) => c.to_string(),
        V::Unit => "()".into(),
        V::Tuple(xs) => format!("({})", xs.iter().map(repr).collect::<Vec<_>>().join(", ")),
        V::Sequence(xs) => format!("[{}]", xs.iter().map(repr).collect::<Vec<_>>().join(" ")),
        V::Record(fs) => format!(
            "{{{}}}",
            fs.iter()
                .map(|(k, v)| format!("{k} := {}", repr(v)))
                .collect::<Vec<_>>()
                .join("; ")
        ),
        V::Table(xs, _) => format!("#[{}]", xs.iter().map(repr).collect::<Vec<_>>().join(" ")),
        V::Dict(xs) => format!(
            "#{{{}}}",
            xs.iter()
                .map(|(k, v)| format!("{} : {}", repr(k), repr(v)))
                .collect::<Vec<_>>()
                .join("; ")
        ),
        V::Optional(None) => "?0".into(),
        V::Optional(Some(x)) => format!("?1 {}", repr(x)),
        V::Fun(_) => "<function>".into(),
        V::Action(_) => "<action>".into(),
    }
}
fn equal(a: &V, b: &V) -> bool {
    match (a, b) {
        (V::Exact(a), V::Exact(b)) => a.n == b.n && a.d == b.d,
        (V::Real(a), V::Real(b)) => a == b,
        (V::Bool(a), V::Bool(b)) => a == b,
        (V::String(a), V::String(b)) => a == b,
        (V::Char(a), V::Char(b)) => a == b,
        (V::Unit, V::Unit) | (V::Optional(None), V::Optional(None)) => true,
        (V::Optional(Some(a)), V::Optional(Some(b))) => equal(a, b),
        (V::Tuple(a), V::Tuple(b)) | (V::Sequence(a), V::Sequence(b)) => {
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(a, b)| equal(a, b))
        }
        (V::Record(a), V::Record(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, v)| b.iter().any(|(l, w)| k == l && equal(v, w)))
        }
        (V::Table(a, ta), V::Table(b, tb)) => {
            ta == tb && a.len() == b.len() && a.iter().zip(b.iter()).all(|(a, b)| equal(a, b))
        }
        (V::Dict(a), V::Dict(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, v)| b.iter().any(|(l, w)| equal(k, l) && equal(v, w)))
        }
        _ => false,
    }
}
fn projection(field: &str, v: V, span: Span) -> R<V> {
    match v {
        V::Record(fs) => fs
            .iter()
            .find(|(k, _)| k.as_ref() == field)
            .map(|(_, v)| v.clone())
            .ok_or_else(|| error(span, format!("record has no field .{field}"))),
        V::Tuple(xs) => field
            .parse::<usize>()
            .ok()
            .and_then(|i| i.checked_sub(1))
            .and_then(|i| xs.get(i))
            .cloned()
            .ok_or_else(|| error(span, format!("tuple has no field .{field}"))),
        V::Sequence(xs) | V::Table(xs, _) => Ok(V::Sequence(
            xs.iter()
                .cloned()
                .map(|x| projection(field, x, span))
                .collect::<R<Vec<_>>>()?
                .into(),
        )),
        _ => Err(error(
            span,
            "projection expects a row or structured collection",
        )),
    }
}

type Collection = (Rc<[V]>, Option<Rc<TableSchema>>);
fn collection(v: V, span: Span) -> R<Collection> {
    match v {
        V::Sequence(xs) => Ok((xs, None)),
        V::Table(xs, s) => Ok((xs, Some(s))),
        _ => Err(error(span, "expected sequence or table")),
    }
}
fn wrap(xs: Vec<V>, s: Option<Rc<TableSchema>>) -> V {
    if let Some(s) = s {
        V::Table(xs.into(), s)
    } else {
        V::Sequence(xs.into())
    }
}
fn layout(schema: Option<&Rc<TableSchema>>, span: Span) -> R<Rc<TableSchema>> {
    schema
        .cloned()
        .ok_or_else(|| error(span, "closed table schema missing from typed HIR"))
}
fn named(v: &V, span: Span) -> R<&[(Box<str>, V)]> {
    if let V::Record(fields) = v {
        Ok(fields)
    } else {
        Err(error(span, "expected a record"))
    }
}
fn named_cell<'a>(fs: &'a [(Box<str>, V)], key: &str, span: Span) -> R<&'a V> {
    fs.iter()
        .find(|(k, _)| k.as_ref() == key)
        .map(|(_, v)| v)
        .ok_or_else(|| error(span, format!("record lacks column {key}")))
}
type RowCells = Vec<(Box<str>, V)>;
fn column_rows(v: &V, span: Span) -> R<Vec<RowCells>> {
    let fields = named(v, span)?;
    let mut n = None;
    let mut columns = Vec::new();
    for (k, c) in fields {
        let items = as_seq(c).map_err(|m| error(span, m))?;
        if n.is_some_and(|n| n != items.len()) {
            return Err(error(span, "record columns have unequal lengths"));
        }
        n = Some(items.len());
        columns.push((k, items));
    }
    let mut rows = (0..n.unwrap_or(0))
        .map(|_| Vec::with_capacity(fields.len()))
        .collect::<Vec<_>>();
    for (k, items) in columns {
        for (r, v) in rows.iter_mut().zip(items) {
            r.push((k.clone(), v.clone()));
        }
    }
    Ok(rows)
}
#[derive(PartialEq, Eq, Hash)]
enum Key {
    Num(i128, i128),
    Real(u64),
    Bool(bool),
    Str(Rc<str>),
    Char(char),
    Unit,
    None,
}
fn key(v: &V) -> Option<Key> {
    Some(match v {
        V::Exact(x) => Key::Num(x.n, x.d),
        V::Real(r) => Key::Real(if *r == 0.0 { 0 } else { r.to_bits() }),
        V::Bool(b) => Key::Bool(*b),
        V::String(s) => Key::Str(s.clone()),
        V::Char(c) => Key::Char(*c),
        V::Unit => Key::Unit,
        V::Optional(None) => Key::None,
        _ => return None,
    })
}
fn order(a: &V, b: &V) -> NumericResult<Ordering> {
    match (a, b) {
        (V::Exact(a), V::Exact(b)) => Ok(a.order(*b)),
        (V::Real(a), V::Real(b)) => a.partial_cmp(b).ok_or("nonfinite real result"),
        (V::String(a), V::String(b)) => Ok(a.cmp(b)),
        (V::Char(a), V::Char(b)) => Ok(a.cmp(b)),
        _ => Err("values are not comparable in this backend"),
    }
}
fn as_bool(v: &V) -> NumericResult<bool> {
    if let V::Bool(b) = v {
        Ok(*b)
    } else {
        Err("expected scalar Boolean")
    }
}
fn as_exact(v: &V) -> NumericResult<Exact> {
    if let V::Exact(n) = v {
        Ok(*n)
    } else {
        Err("expected an exact number")
    }
}
fn as_seq(v: &V) -> NumericResult<&[V]> {
    if let V::Sequence(xs) = v {
        Ok(xs)
    } else {
        Err("expected a sequence")
    }
}
fn finite(v: f64) -> NumericResult<V> {
    if v.is_finite() {
        Ok(V::Real(v))
    } else {
        Err("nonfinite real result")
    }
}
fn inner_type(t: &Type) -> &Type {
    if let Type::Seq(x) = t {
        x
    } else {
        t
    }
}
fn result_type(mut t: &Type, n: usize) -> &Type {
    for _ in 0..n {
        if let Type::Fun(_, r) = t {
            t = r
        } else {
            break;
        }
    }
    t
}

fn lift(
    a: V,
    b: V,
    mode: Option<LiftMode>,
    f: &impl Fn(V, V) -> NumericResult<V>,
) -> NumericResult<V> {
    // Indirect callable references are not specialized yet. Direct calls carry
    // an explicit static mode and never use these value tags for selection.
    let mode = match mode {
        Some(typed) => typed,
        None => match (&a, &b) {
            (V::Sequence(_), V::Sequence(_)) => LiftMode::Pairwise,
            (V::Sequence(_), _) => LiftMode::LeftSequence,
            (_, V::Sequence(_)) => LiftMode::RightSequence,
            _ => LiftMode::Scalar,
        },
    };
    Ok(match mode {
        LiftMode::Scalar => f(a, b)?,
        LiftMode::LeftSequence => {
            let xs = as_seq(&a)?;
            V::Sequence(
                xs.iter()
                    .cloned()
                    .map(|x| f(x, b.clone()))
                    .collect::<NumericResult<Vec<_>>>()?
                    .into(),
            )
        }
        LiftMode::RightSequence => {
            let xs = as_seq(&b)?;
            V::Sequence(
                xs.iter()
                    .cloned()
                    .map(|x| f(a.clone(), x))
                    .collect::<NumericResult<Vec<_>>>()?
                    .into(),
            )
        }
        LiftMode::Pairwise => {
            let (a, b) = (as_seq(&a)?, as_seq(&b)?);
            if a.len() != b.len() {
                return Err("shape mismatch: sequence lengths differ");
            }
            V::Sequence(
                a.iter()
                    .cloned()
                    .zip(b.iter().cloned())
                    .map(|(x, y)| f(x, y))
                    .collect::<NumericResult<Vec<_>>>()?
                    .into(),
            )
        }
    })
}

fn lift_one(a: V, mode: Option<LiftMode>, f: &impl Fn(V) -> NumericResult<V>) -> NumericResult<V> {
    let mode = match mode {
        Some(typed) => typed,
        None if matches!(a, V::Sequence(_)) => LiftMode::LeftSequence,
        None => LiftMode::Scalar,
    };
    match mode {
        LiftMode::Scalar => f(a),
        LiftMode::LeftSequence => Ok(V::Sequence(
            as_seq(&a)?
                .iter()
                .cloned()
                .map(f)
                .collect::<NumericResult<Vec<_>>>()?
                .into(),
        )),
        _ => Err("invalid unary lifting mode in typed HIR"),
    }
}

fn scalar_binary(op: CoreOp, a: V, b: V, ty: &Type) -> NumericResult<V> {
    use CoreOp::*;
    Ok(match op {
        Add | Sub | Mul | Div | Quotient | Remainder | Pow => {
            if matches!((&a, &b), (V::Real(_), V::Real(_))) || matches!(ty, Type::Prim(Prim::R)) {
                let r = |v| match v {
                    V::Real(n) => Ok(n),
                    V::Exact(n) => Ok(n.real()),
                    _ => Err("expected a number"),
                };
                if op == Pow {
                    let mut a = r(a)?;
                    let b = as_exact(&b)?;
                    let mut exponent = u128::try_from(b.integral()?)
                        .map_err(|_| "power requires a natural exponent")?;
                    if b.domain != Domain::Natural {
                        return Err("power requires a natural exponent");
                    }
                    let mut value = 1.0;
                    while exponent > 0 {
                        if exponent & 1 == 1 {
                            value *= a;
                        }
                        exponent >>= 1;
                        if exponent != 0 {
                            a *= a;
                        }
                    }
                    return finite(value);
                }
                let (a, b) = (r(a)?, r(b)?);
                return finite(match op {
                    Add => a + b,
                    Sub => a - b,
                    Mul => a * b,
                    Div if b != 0.0 => a / b,
                    Div => return Err(DIV_BY_ZERO),
                    _ => return Err("real operator not supported"),
                });
            }
            V::Exact(as_exact(&a)?.compute(op, as_exact(&b)?, domain(ty))?)
        }
        Equal => V::Bool(equal(&a, &b)),
        NotEqual => V::Bool(!equal(&a, &b)),
        Lt => V::Bool(order(&a, &b)?.is_lt()),
        Le => V::Bool(!order(&a, &b)?.is_gt()),
        Gt => V::Bool(order(&a, &b)?.is_gt()),
        Ge => V::Bool(!order(&a, &b)?.is_lt()),
        And => V::Bool(as_bool(&a)? & as_bool(&b)?),
        Or => V::Bool(as_bool(&a)? | as_bool(&b)?),
        _ => return Err("invalid binary operation in typed HIR"),
    })
}
fn operation_arity(op: CoreOp) -> usize {
    use CoreOp::*;
    match op {
        Absent => 0,
        Aggregate => 3,
        TableRows | TableFrom | Rows | CsvRead | Mean | Median | StdDev => 1,
        Negate | Abs | Not | Sum | Product | Count | Execute | Print | Present | Has | Unwrap
        | ToNatural | ToInteger | ToRational | ToReal => 1,
        _ => 2,
    }
}
impl Eval<'_> {
    fn frame(&mut self, parent: usize) -> usize {
        let id = self.frames.len();
        self.frames.push(Frame {
            parent: Some(parent),
            ready: HashMap::new(),
        });
        id
    }
    fn put(&mut self, env: usize, s: SymbolId, v: V) {
        self.frames[env].ready.insert(s, v);
    }
    fn load(&self, mut env: usize, s: SymbolId, span: Span) -> R<V> {
        loop {
            if let Some(v) = self.frames[env].ready.get(&s) {
                return Ok(v.clone());
            }
            env = self.frames[env]
                .parent
                .ok_or_else(|| error(span, "value used before initialization"))?;
        }
    }
    fn define_fun(&mut self, d: &Definition, env: usize) {
        let code = Function::User {
            formals: d.formals.clone().into(),
            body: d.body,
            env,
        };
        self.put(
            env,
            d.symbol,
            V::Fun(Rc::new(Fun {
                code,
                bound: Vec::new(),
            })),
        );
    }
    fn tick(&mut self, span: Span) -> R<()> {
        self.fuel = self
            .fuel
            .checked_sub(1)
            .ok_or_else(|| error(span, "prototype execution limit exhausted"))?;
        Ok(())
    }
    fn eval(&mut self, id: ExprId, env: usize) -> R<V> {
        let p = self.p;
        let node = p.node(id);
        self.tick(node.span)?;
        if self.depth >= 128 {
            return Err(error(
                node.span,
                "prototype nesting or recursion limit (128) exceeded",
            ));
        }
        self.depth += 1;
        let r = self.eval_kind(id, env);
        self.depth -= 1;
        r
    }
    fn eval_kind(&mut self, id: ExprId, env: usize) -> R<V> {
        let p = self.p;
        let n = p.node(id);
        let s = n.span;
        let convert = |message| error(s, message);
        Ok(match &n.kind {
            ExprKind::Literal(v) => match v {
                Literal::Integer(i) => {
                    if n.ty == Type::Prim(Prim::R) {
                        finite(*i as f64).map_err(convert)?
                    } else {
                        let d = domain(&n.ty).unwrap_or(Domain::Integer);
                        V::Exact(Exact::integer(*i, d).as_domain(d).map_err(convert)?)
                    }
                }
                Literal::Numeric(text) => {
                    if text.ends_with('r') || n.ty == Type::Prim(Prim::R) {
                        let raw = text.strip_suffix('r').unwrap_or(text);
                        finite(raw.parse().map_err(|_| error(s, "invalid real literal"))?)
                            .map_err(convert)?
                    } else {
                        V::Exact(parse_exact(text, &n.ty).map_err(convert)?)
                    }
                }
                Literal::Bool(b) => V::Bool(*b),
                Literal::String(text) => V::String(Rc::from(text.as_ref())),
                Literal::Char(c) => V::Char(*c),
                Literal::Unit => V::Unit,
            },
            ExprKind::Load(sid) => self.load(env, *sid, s)?,
            ExprKind::Reference(Target::Symbol(sid)) => self.load(env, *sid, s)?,
            ExprKind::Reference(Target::Projection(field)) => V::Fun(Rc::new(Fun {
                code: Function::Projection(field.clone()),
                bound: Vec::new(),
            })),
            ExprKind::Reference(Target::Builtin(op)) => V::Fun(Rc::new(Fun {
                code: Function::Core(*op, n.ty.clone(), n.table_schema.clone()),
                bound: Vec::new(),
            })),
            ExprKind::Call(Target::Builtin(CoreOp::Default), args) => {
                let [fallback, stored] = args.as_slice() else {
                    return Err(error(s, "invalid fallback operands"));
                };
                match self.eval(*stored, env)? {
                    V::Optional(Some(v)) => v.as_ref().clone(),
                    V::Optional(None) => self.eval(*fallback, env)?,
                    _ => return Err(error(s, "fallback requires an optional second operand")),
                }
            }
            ExprKind::Call(target, args) => {
                let f = match target {
                    Target::Symbol(sid) => Some(self.load(env, *sid, s)?),
                    Target::Builtin(_) | Target::Projection(_) => None,
                };
                let args = args
                    .iter()
                    .map(|id| self.eval(*id, env))
                    .collect::<R<Vec<_>>>()?;
                match target {
                    Target::Builtin(op) => {
                        let ty = match &n.evidence {
                            Some(crate::infer::Operation::Numeric { result, .. }) => result,
                            _ => &n.ty,
                        };
                        self.core(*op, args, ty, n.table_schema.as_ref(), n.lift_mode, s)?
                    }
                    Target::Projection(field) => {
                        let [v]: [V; 1] = args
                            .try_into()
                            .map_err(|_| error(s, "invalid projection operand count"))?;
                        projection(field, v, s)?
                    }
                    Target::Symbol(_) => self.apply(f.expect("callee loaded"), args, s)?,
                }
            }
            ExprKind::Tuple(ids) | ExprKind::Sequence(ids) => {
                let xs = ids
                    .iter()
                    .map(|id| self.eval(*id, env))
                    .collect::<R<Vec<_>>>()?
                    .into();
                if matches!(n.kind, ExprKind::Tuple(_)) {
                    V::Tuple(xs)
                } else {
                    V::Sequence(xs)
                }
            }
            ExprKind::Record(fields) => V::Record(
                fields
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), self.eval(*v, env)?)))
                    .collect::<R<Vec<_>>>()?
                    .into(),
            ),
            ExprKind::Cases {
                branches,
                otherwise,
            } => {
                for &(a, b) in branches {
                    if as_bool(&self.eval(a, env)?).map_err(convert)? {
                        return self.eval(b, env);
                    }
                }
                self.eval(*otherwise, env)?
            }
            ExprKind::Closure { formals, body } => V::Fun(Rc::new(Fun {
                code: Function::User {
                    formals: formals.clone().into(),
                    body: *body,
                    env,
                },
                bound: Vec::new(),
            })),
            ExprKind::Pipeline(stages) => {
                let mut value = V::Unit;
                for stage in stages {
                    let scope = if let Some(input) = stage.input {
                        let scope = self.frame(env);
                        self.put(scope, input, value);
                        scope
                    } else {
                        env
                    };
                    value = self.eval(stage.expr, scope)?;
                }
                value
            }
            ExprKind::Block {
                suspended: true, ..
            } => V::Action(Rc::new(Action::Block { expr: id, env })),
            ExprKind::Block { steps, tail, .. } => self.block(steps, *tail, env)?,
        })
    }
    fn block(&mut self, steps: &[Step], tail: Option<ExprId>, parent: usize) -> R<V> {
        let env = self.frame(parent);
        for step in steps {
            if let Step::Define(d) = step {
                if !d.formals.is_empty() {
                    self.define_fun(d, env)
                }
            }
        }
        for step in steps {
            match step {
                Step::Define(d) if d.formals.is_empty() => {
                    let v = self.eval(d.body, env)?;
                    self.put(env, d.symbol, v);
                }
                Step::Evaluate { binding, body } => {
                    let v = self.eval(*body, env)?;
                    if let Some(s) = binding {
                        self.put(env, *s, v)
                    }
                }
                _ => {}
            }
        }
        tail.map(|id| self.eval(id, env)).unwrap_or(Ok(V::Unit))
    }
    fn apply(&mut self, f: V, args: Vec<V>, span: Span) -> R<V> {
        self.tick(span)?;
        let V::Fun(f) = f else {
            return Err(error(span, "expected a callable function value"));
        };
        let mut f = Rc::unwrap_or_clone(f);
        f.bound.extend(args);
        let arity = match &f.code {
            Function::User { formals, .. } => formals.len(),
            Function::Core(op, _, _) => operation_arity(*op),
            Function::Projection(_) => 1,
        };
        if f.bound.len() < arity {
            return Ok(V::Fun(Rc::new(f)));
        }
        if f.bound.len() > arity {
            return Err(error(span, "too many arguments for function"));
        }
        match f.code {
            Function::User { formals, body, env } => {
                let scope = self.frame(env);
                for (s, v) in formals.iter().zip(f.bound) {
                    self.put(scope, *s, v)
                }
                self.eval(body, scope)
            }
            Function::Core(op, ty, schema) => self.core(
                op,
                f.bound,
                result_type(&ty, arity),
                schema.as_ref(),
                None,
                span,
            ),
            Function::Projection(field) => {
                projection(&field, f.bound.pop().expect("checked arity"), span)
            }
        }
    }
    fn execute(&mut self, v: V, span: Span) -> R<V> {
        let V::Action(action) = v else {
            return Err(error(span, "expected a suspended action"));
        };
        match action.as_ref() {
            Action::Print(v) => {
                self.stdout.push_str(&repr(v));
                self.stdout.push('\n');
                Ok(V::Unit)
            }
            Action::CsvRead(path, schema) => read_csv(path, schema.as_ref(), span),
            Action::CsvWrite(path, data) => {
                write_csv(path, data, span)?;
                Ok(V::Unit)
            }
            Action::Block { expr, env } => {
                let p = self.p;
                let ExprKind::Block { steps, tail, .. } = &p.node(*expr).kind else {
                    return Err(error(span, "invalid action expression in typed HIR"));
                };
                if self.effects >= 128 {
                    return Err(error(
                        span,
                        "prototype action recursion limit (128) exceeded",
                    ));
                }
                self.effects += 1;
                let result = self.block(steps, *tail, *env);
                self.effects -= 1;
                result
            }
        }
    }
    fn group(&mut self, f: V, xs: &[V], span: Span) -> R<Vec<(V, Vec<V>)>> {
        let mut groups: Vec<(V, Vec<V>)> = Vec::new();
        let mut index = HashMap::new();
        for row in xs {
            let k = self.apply(f.clone(), vec![row.clone()], span)?;
            let i = if let Some(key) = key(&k) {
                *index.entry(key).or_insert(groups.len())
            } else {
                groups
                    .iter()
                    .position(|(old, _)| equal(old, &k))
                    .unwrap_or(groups.len())
            };
            if let Some(g) = groups.get_mut(i) {
                g.1.push(row.clone())
            } else {
                groups.push((k, vec![row.clone()]))
            }
        }
        Ok(groups)
    }
    fn core(
        &mut self,
        op: CoreOp,
        args: Vec<V>,
        ty: &Type,
        schema: Option<&Rc<TableSchema>>,
        mode: Option<LiftMode>,
        span: Span,
    ) -> R<V> {
        use CoreOp::*;
        self.tick(span)?;
        if args.len() != operation_arity(op) {
            return Err(error(span, "invalid typed HIR operator arity"));
        }
        let mut args = args.into_iter();
        let mut a = || args.next().expect("arity checked");
        let fail = |message| error(span, message);
        let result = match op {
            Add | Sub | Mul | Div | Quotient | Remainder | Pow | Equal | NotEqual | Lt | Le
            | Gt | Ge | And | Or => lift(a(), a(), mode, &|x, y| {
                scalar_binary(op, x, y, inner_type(ty))
            }),
            EqualWhole => Ok(V::Bool(equal(&a(), &a()))),
            Negate | Abs | Not => lift_one(a(), mode, &|v| {
                if op == Not {
                    return Ok(V::Bool(!as_bool(&v)?));
                }
                if let V::Real(n) = v {
                    return finite(if op == Abs { n.abs() } else { -n });
                }
                let n = as_exact(&v)?;
                let (num, kind) = if op == Abs {
                    (n.n.checked_abs(), n.domain)
                } else {
                    (
                        n.n.checked_neg(),
                        if n.domain == Domain::Natural {
                            Domain::Integer
                        } else {
                            n.domain
                        },
                    )
                };
                Ok(V::Exact(Exact::new(num.ok_or(OVERFLOW)?, n.d, kind)?))
            }),
            Count => {
                let count = match a() {
                    V::Sequence(v) | V::Table(v, _) => v.len(),
                    V::Dict(v) => v.len(),
                    V::String(v) => v.chars().count(),
                    _ => {
                        return Err(error(
                            span,
                            "count expects a sequence or string in this backend",
                        ))
                    }
                };
                Ok(V::Exact(Exact::integer(count as i128, Domain::Natural)))
            }
            Range => {
                let lo = as_exact(&a()).map_err(fail)?;
                let hi = as_exact(&a()).map_err(fail)?;
                let type_domain = domain(ty).unwrap_or_else(|| Exact::common(lo.domain, hi.domain));
                let lo = lo.integral().map_err(fail)?;
                let hi = hi.integral().map_err(fail)?;
                if hi < lo {
                    return Ok(V::Sequence(Rc::from([])));
                }
                let n = hi.checked_sub(lo).ok_or_else(|| error(span, OVERFLOW))?;
                if n >= COLLECTION_LIMIT as i128 {
                    return Err(error(span, "prototype range limit (100000 items) exceeded"));
                }
                Ok(V::Sequence(
                    (lo..=hi)
                        .map(|i| V::Exact(Exact::integer(i, type_domain)))
                        .collect::<Vec<_>>()
                        .into(),
                ))
            }
            Sum | Product => {
                let xs = a();
                let xs = as_seq(&xs).map_err(fail)?;
                let identity = i128::from(op == Product);
                let choice = if op == Sum { Add } else { Mul };
                let mut n = if *ty == Type::Prim(Prim::R) {
                    V::Real(identity as f64)
                } else {
                    V::Exact(Exact::integer(
                        identity,
                        domain(ty).unwrap_or(Domain::Natural),
                    ))
                };
                for v in xs {
                    n = scalar_binary(choice, n, v.clone(), ty).map_err(fail)?;
                }
                Ok(n)
            }
            Apply => return self.apply(a(), vec![a()], span),
            ApplyTuple => {
                let mut f = a();
                let xs = a();
                let V::Tuple(xs) = xs else {
                    return Err(error(span, "@@ expects a tuple bundle"));
                };
                for operand in xs.iter() {
                    f = self.apply(f, vec![operand.clone()], span)?;
                }
                return Ok(f);
            }
            Map => {
                let (f, v) = (a(), a());
                if let V::Dict(entries) = v {
                    let mut out = Vec::with_capacity(entries.len());
                    for (k, v) in entries.iter() {
                        out.push((k.clone(), self.apply(f.clone(), vec![v.clone()], span)?))
                    }
                    return Ok(V::Dict(out.into()));
                }
                let (xs, _) = collection(v, span)?;
                let mut out = Vec::with_capacity(xs.len());
                for v in xs.iter() {
                    out.push(self.apply(f.clone(), vec![v.clone()], span)?)
                }
                Ok(V::Sequence(out.into()))
            }
            TableRows | TableFrom => {
                let v = a();
                let schema = layout(schema, span)?;
                if let V::Sequence(rows) = v {
                    return Ok(V::Table(rows, schema));
                }
                if op == TableRows {
                    return Err(error(span, "table rows require a sequence"));
                }
                let rows = column_rows(&v, span)?
                    .into_iter()
                    .map(|r| V::Record(r.into()))
                    .collect::<Vec<_>>();
                Ok(V::Table(rows.into(), schema))
            }
            Rows => match a() {
                V::Table(rows, _) => Ok(V::Sequence(rows)),
                _ => Err(":# expects a table"),
            },
            Filter => {
                let (predicate, v) = (a(), a());
                let (xs, shape) = collection(v, span)?;
                let mut out = Vec::new();
                if let V::Sequence(mask) = predicate {
                    if mask.len() != xs.len() {
                        return Err(error(span, "mask length differs from collection length"));
                    }
                    for (x, b) in xs.iter().zip(mask.iter()) {
                        if as_bool(b).map_err(fail)? {
                            out.push(x.clone())
                        }
                    }
                } else {
                    for x in xs.iter() {
                        if as_bool(&self.apply(predicate.clone(), vec![x.clone()], span)?)
                            .map_err(fail)?
                        {
                            out.push(x.clone())
                        }
                    }
                }
                Ok(wrap(out, shape))
            }
            Group | Aggregate => {
                let key = a();
                let summarizer = if op == Aggregate { Some(a()) } else { None };
                let (xs, in_schema) = collection(a(), span)?;
                if xs.len() > 8192 {
                    return Err(error(span, "prototype grouping limit (8192 rows) exceeded"));
                }
                let groups = self.group(key, &xs, span)?;
                if let Some(f) = summarizer {
                    let schema = layout(schema, span)?;
                    let mut rows = Vec::with_capacity(groups.len());
                    for (key, items) in groups {
                        let p = self.apply(f.clone(), vec![key], span)?;
                        let row = self.apply(p, vec![wrap(items, in_schema.clone())], span)?;
                        rows.push(row);
                    }
                    Ok(V::Table(rows.into(), schema))
                } else {
                    Ok(V::Dict(
                        groups
                            .into_iter()
                            .map(|(k, items)| (k, wrap(items, in_schema.clone())))
                            .collect::<Vec<_>>()
                            .into(),
                    ))
                }
            }
            Extend => {
                let (new, source) = (a(), a());
                let schema = layout(schema, span)?;
                let (rows, _) = collection(source, span)?;
                let extra = column_rows(&new, span)?;
                if extra.len() != rows.len() {
                    return Err(error(
                        span,
                        "extension column length does not match table height",
                    ));
                }
                let mut out = Vec::with_capacity(rows.len());
                for (row, new) in rows.iter().zip(extra) {
                    let mut fs = named(row, span)?.to_vec();
                    for (k, v) in new {
                        if fs.iter().any(|(f, _)| *f == k) {
                            return Err(error(
                                span,
                                format!("df.extend cannot duplicate existing field {k}"),
                            ));
                        }
                        fs.push((k, v));
                    }
                    out.push(V::Record(fs.into()));
                }
                Ok(V::Table(out.into(), schema))
            }
            SortAsc | SortDesc => {
                let (f, source) = (a(), a());
                let (values, schema) = collection(source, span)?;
                let mut pairs = Vec::with_capacity(values.len());
                for v in values.iter() {
                    pairs.push((self.apply(f.clone(), vec![v.clone()], span)?, v.clone()))
                }
                if let Some((first, _)) = pairs.first() {
                    for (k, _) in &pairs {
                        order(first, k).map_err(fail)?;
                    }
                }
                pairs.sort_by(|(a, _), (b, _)| {
                    let cmp = order(a, b).unwrap_or(Ordering::Equal);
                    if op == SortAsc {
                        cmp
                    } else {
                        cmp.reverse()
                    }
                });
                Ok(wrap(pairs.into_iter().map(|(_, v)| v).collect(), schema))
            }
            Mean | Median | StdDev => return stats(op, a(), span),
            CsvRead => {
                let V::String(path) = a() else {
                    return Err(error(span, "csv.read requires a string path"));
                };
                Ok(V::Action(Rc::new(Action::CsvRead(path, schema.cloned()))))
            }
            CsvWrite => {
                let (path, t) = (a(), a());
                let V::String(path) = path else {
                    return Err(error(span, "csv.write requires a string path"));
                };
                Ok(V::Action(Rc::new(Action::CsvWrite(path, t))))
            }
            Execute => {
                if self.effects == 0 {
                    return Err(error(
                        span,
                        "execution is only permitted inside an effect block",
                    ));
                }
                return self.execute(a(), span);
            }
            Print => Ok(V::Action(Rc::new(Action::Print(a())))),
            Present => Ok(V::Optional(Some(Rc::new(a())))),
            Absent => Ok(V::Optional(None)),
            Has => {
                if let V::Optional(x) = a() {
                    Ok(V::Bool(x.is_some()))
                } else {
                    Err("presence test requires an optional")
                }
            }
            Unwrap => match a() {
                V::Optional(Some(v)) => Ok(v.as_ref().clone()),
                V::Optional(None) => Err("cannot unwrap ?0"),
                _ => Err("unwrap requires an optional"),
            },
            Default => match (a(), a()) {
                (_, V::Optional(Some(v))) => Ok(v.as_ref().clone()),
                (fallback, V::Optional(None)) => Ok(fallback),
                _ => Err("fallback requires an optional"),
            },
            ToNatural | ToInteger | ToRational | ToReal => lift_one(a(), mode, &|v| {
                if op == ToReal {
                    return match v {
                        V::Real(x) => Ok(V::Real(x)),
                        v => finite(as_exact(&v)?.real()),
                    };
                }
                let d = match op {
                    ToNatural => Domain::Natural,
                    ToInteger => Domain::Integer,
                    _ => Domain::Rational,
                };
                let exact = match v {
                    V::Real(x) => Exact::from_real(x)?,
                    other => as_exact(&other)?,
                };
                Ok(V::Exact(exact.as_domain(d)?))
            }),
        };
        result.map_err(fail)
    }
}

fn stats(op: CoreOp, v: V, span: Span) -> R<V> {
    let fail = |m| error(span, m);
    let xs = as_seq(&v).map_err(fail)?;
    if xs.is_empty() {
        return Ok(V::Optional(None));
    }
    let value = if op == CoreOp::StdDev {
        let mut mean = 0.0;
        let mut ss = 0.0;
        for (i, x) in xs.iter().enumerate() {
            let x = match x {
                V::Exact(x) => x.real(),
                V::Real(r) => *r,
                _ => return Err(error(span, "sd expects numbers")),
            };
            let d = x - mean;
            mean += d / (i + 1) as f64;
            ss += d * (x - mean);
        }
        if !ss.is_finite() {
            return Err(error(span, "real standard deviation overflow"));
        }
        finite((ss.max(0.0) / xs.len() as f64).sqrt()).map_err(fail)?
    } else {
        let mut xs = xs
            .iter()
            .map(as_exact)
            .collect::<NumericResult<Vec<_>>>()
            .map_err(fail)?;
        let (sum, div) = if op == CoreOp::Median {
            xs.sort_by(|a, b| a.order(*b));
            let mid = xs.len() / 2;
            if xs.len() % 2 == 1 {
                (xs[mid], 1)
            } else {
                (
                    xs[mid - 1]
                        .compute(CoreOp::Add, xs[mid], Some(Domain::Rational))
                        .map_err(fail)?,
                    2,
                )
            }
        } else {
            let mut sum = Exact::integer(0, Domain::Rational);
            for v in &xs {
                sum = sum
                    .compute(CoreOp::Add, *v, Some(Domain::Rational))
                    .map_err(fail)?;
            }
            (sum, xs.len() as i128)
        };
        V::Exact(
            sum.compute(
                CoreOp::Div,
                Exact::integer(div, Domain::Natural),
                Some(Domain::Rational),
            )
            .map_err(fail)?,
        )
    };
    Ok(V::Optional(Some(Rc::new(value))))
}

fn read_cell(s: &str, ty: &Type) -> NumericResult<V> {
    if let Type::Optional(inner) = ty {
        return if s.is_empty() {
            Ok(V::Optional(None))
        } else {
            Ok(V::Optional(Some(Rc::new(read_cell(s, inner)?))))
        };
    }
    let parse_num = |s: &str| s.parse().map_err(|_| "expected a bounded integer");
    Ok(match ty {
        Type::Prim(Prim::S) => V::String(Rc::from(s)),
        Type::Prim(Prim::C) => {
            let mut i = s.chars();
            match (i.next(), i.next()) {
                (Some(c), None) => V::Char(c),
                _ => return Err("expected a single character"),
            }
        }
        Type::Prim(Prim::B) => match s {
            "true" | "1b" => V::Bool(true),
            "false" | "0b" => V::Bool(false),
            _ => return Err("expected true, false, 1b, or 0b"),
        },
        Type::Prim(Prim::N | Prim::Z) => V::Exact(
            Exact::integer(parse_num(s)?, Domain::Integer)
                .as_domain(domain(ty).expect("known domain"))?,
        ),
        Type::Prim(Prim::Q) => {
            let n = if let Some((a, b)) = s.split_once('/') {
                Exact::new(parse_num(a)?, parse_num(b)?, Domain::Rational)?
            } else {
                if s.is_empty()
                    || s.bytes()
                        .any(|b| !matches!(b, b'0'..=b'9' | b'-' | b'+' | b'.'))
                {
                    return Err("expected a decimal or rational");
                }
                parse_exact(s, ty)?
            };
            V::Exact(n)
        }
        Type::Prim(Prim::R) => finite(s.parse().map_err(|_| "expected a finite real")?)?,
        _ => return Err("CSV schema contains an unsupported column type"),
    })
}
fn read_csv(path: &str, shape: Option<&Rc<TableSchema>>, span: Span) -> R<V> {
    let data = crate::csv::read(path).map_err(|m| error(span, m))?;
    let Some(shape) = shape else {
        let xs =
            crate::csv::decode(&data).map_err(|m| error(span, format!("CSV {path:?}: {m}")))?;
        let rows = xs
            .into_iter()
            .map(|row| {
                V::Sequence(
                    row.into_iter()
                        .map(|s| V::String(Rc::from(s)))
                        .collect::<Vec<_>>()
                        .into(),
                )
            })
            .collect::<Vec<_>>();
        return Ok(V::Sequence(rows.into()));
    };
    let TableSchema::Named(schema) = shape.as_ref() else {
        return Err(error(span, "typed CSV requires a named row schema"));
    };
    let keys = schema.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>();
    let cells = crate::csv::typed_rows(&data, &keys)
        .map_err(|m| error(span, format!("CSV {path:?}: {m}")))?;
    let mut rows = Vec::with_capacity(cells.len());
    for (i, cells) in cells.into_iter().enumerate() {
        let mut row = Vec::with_capacity(schema.len());
        for ((name, ty), value) in schema.iter().zip(cells) {
            let decoded = read_cell(&value, ty).map_err(|m| {
                error(
                    span,
                    format!("CSV {path:?}: row {}, column {name}: {m}", i + 2),
                )
            })?;
            row.push((name.as_str().into(), decoded));
        }
        rows.push(V::Record(row.into()));
    }
    Ok(V::Table(rows.into(), shape.clone()))
}
fn write_cell(v: &V) -> NumericResult<String> {
    Ok(match v {
        V::String(s) => s.to_string(),
        V::Char(c) => c.to_string(),
        V::Bool(b) => b.to_string(),
        V::Exact(_) | V::Real(_) => repr(v),
        V::Optional(None) => String::new(),
        V::Optional(Some(v)) => {
            let s = write_cell(v)?;
            if s.is_empty() {
                return Err("present optional cell encodes as blank and would be read as ?0");
            }
            s
        }
        _ => return Err("unsupported CSV output cell type"),
    })
}
fn write_csv(path: &str, value: &V, span: Span) -> R<()> {
    let V::Table(rows, shape) = value else {
        return Err(error(span, "CSV write expects a table"));
    };
    let TableSchema::Named(schema) = shape.as_ref() else {
        return Err(error(span, "CSV write needs a table with named columns"));
    };
    let mut data = vec![schema.iter().map(|(k, _)| k.clone()).collect()];
    for (i, row) in rows.iter().enumerate() {
        let row = named(row, span)?;
        let cells = schema
            .iter()
            .map(|(name, _)| {
                write_cell(named_cell(row, name, span)?)
                    .map_err(|m| error(span, format!("CSV row {}, column {name}: {m}", i + 2)))
            })
            .collect::<R<Vec<_>>>()?;
        data.push(cells);
    }
    crate::csv::write(path, &data).map_err(|m| error(span, m))
}

fn evaluate(program: &HirProgram) -> R<(String, V)> {
    let mut eval = Eval {
        p: program,
        frames: vec![Frame::default()],
        stdout: String::new(),
        fuel: 1_000_000,
        depth: 0,
        effects: 0,
    };
    for group in &program.groups {
        for d in group {
            if !d.formals.is_empty() {
                eval.define_fun(d, 0)
            }
        }
        for d in group {
            if d.formals.is_empty() {
                let v = eval.eval(d.body, 0)?;
                eval.put(0, d.symbol, v);
            }
        }
    }
    let value = if let Some(id) = program.entry {
        let value = eval.eval(id, 0)?;
        eval.execute(value, program.node(id).span)?
    } else {
        V::Unit
    };
    Ok((eval.stdout, value))
}

pub fn run(program: &HirProgram) -> R<Output> {
    let (stdout, value) = evaluate(program)?;
    let span = program
        .entry
        .map(|id| program.node(id).span)
        .unwrap_or(Span { start: 0, end: 0 });
    let status = match value {
        V::Unit => 0,
        V::Exact(n) if n.d == 1 => {
            u8::try_from(n.n).map_err(|_| error(span, "entry exit status must be from 0 to 255"))?
        }
        _ => {
            return Err(error(
                span,
                "entry must return unit or a natural exit status",
            ))
        }
    };
    Ok(Output { stdout, status })
}

fn table_view(rows: &[V], schema: &TableSchema) -> String {
    let headers = match schema {
        TableSchema::Named(fields) => fields.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>(),
        TableSchema::Positional(fields) => (1..=fields.len()).map(|i| format!(".{i}")).collect(),
    };
    if headers.is_empty() {
        return format!("#[ ]  ({} rows)", rows.len());
    }
    let safe = |text: String| {
        let snippet = text.chars().take(32).collect::<String>();
        let mut escaped = String::new();
        for c in snippet.chars() {
            if c.is_control() {
                escaped.extend(c.escape_default())
            } else {
                escaped.push(c)
            }
        }
        if snippet.len() < text.len() {
            escaped.push_str("...")
        }
        escaped
    };
    let mut display = vec![headers.iter().map(|h| safe(h.clone())).collect::<Vec<_>>()];
    for row in rows.iter().take(12) {
        let cells = match row {
            V::Record(fs) => headers
                .iter()
                .map(|key| {
                    fs.iter()
                        .find(|(k, _)| k.as_ref() == key)
                        .map(|(_, v)| safe(repr(v)))
                        .unwrap_or_else(|| "-".into())
                })
                .collect(),
            V::Tuple(xs) => xs.iter().map(|x| safe(repr(x))).collect(),
            _ => vec![safe(repr(row))],
        };
        display.push(cells);
    }
    let width = (0..headers.len())
        .map(|i| {
            display
                .iter()
                .filter_map(|r| r.get(i))
                .map(|x| x.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect::<Vec<_>>();
    let mut out = Vec::new();
    for (i, row) in display.into_iter().enumerate() {
        out.push(
            row.into_iter()
                .zip(&width)
                .map(|(s, w)| format!("{s:<width$}", width = *w))
                .collect::<Vec<_>>()
                .join(" | "),
        );
        if i == 0 {
            out.push(
                width
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

pub fn view_entry(program: &HirProgram) -> R<crate::eval::ReplOutput> {
    let (stdout, value) = evaluate(program)?;
    fn inspect(v: &V) -> String {
        match v {
            V::String(s) => format!("{s:?}"),
            V::Char(c) => format!("{c:?}"),
            V::Tuple(xs) => format!(
                "({})",
                xs.iter().map(inspect).collect::<Vec<_>>().join(", ")
            ),
            V::Sequence(xs) => {
                format!("[{}]", xs.iter().map(inspect).collect::<Vec<_>>().join(" "))
            }
            V::Record(fs) => format!(
                ".{{ {} }}",
                fs.iter()
                    .map(|(k, v)| format!("{k} := {}", inspect(v)))
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
            V::Table(rows, schema) => table_view(rows, schema),
            V::Dict(fs) => format!(
                "#{{ {} }}",
                fs.iter()
                    .map(|(k, v)| format!("{} : {}", inspect(k), inspect(v)))
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
            V::Optional(Some(x)) => format!("?1 {}", inspect(x)),
            v => repr(v),
        }
    }
    Ok(crate::eval::ReplOutput {
        value: inspect(&value),
        stdout,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile(source: &str) -> HirProgram {
        let p = crate::parser::parse(source).unwrap();
        let names = crate::resolve::resolve(&p).unwrap();
        let types = crate::infer::infer(&p, &names).unwrap();
        crate::hir::lower(&p, &types, &names).unwrap()
    }

    #[test]
    fn owned_typed_hir_runs_after_surface_tree_is_dropped() {
        let ir = compile(
            r#"
            << "io";
            id(x) := x;
            f := |{ 4; \y -> + $ y };
            !{
                ! io.print id 9;
                ! io.print id "polymorphic";
                ! io.print @ f 5;
            };
        "#,
        );
        let result = run(&ir).unwrap();
        assert_eq!(result.stdout, "9\npolymorphic\n9\n");
        assert_eq!(result.status, 0);
    }

    #[test]
    fn typed_pipeline_and_lexical_closures() {
        let ir = compile(
            r#"
            << "io";
            twice(x:z):z := + x x;
            !{
                x := |{ ! !{ ! io.print "once"; 4 }; + $ $; @ &twice $ };
                ! io.print x;
            };
        "#,
        );
        assert_eq!(run(&ir).unwrap().stdout, "once\n16\n");
    }

    #[test]
    fn typed_hir_runtime_domain_error_and_lazy_branches() {
        let good = compile(r#" << "io"; !{ ! io.print ? 1b (/ 3 2) (/ 1 0); }; "#);
        assert_eq!(run(&good).unwrap().stdout, "3/2\n");
        let bad = compile("a := 1;\nb := / a 0;\n!{ (); };");
        let error = run(&bad).err().unwrap();
        assert_eq!(error.start, 13);
        assert!(error.message.contains("division by zero"));
    }
    #[test]
    fn typed_row_generalization_runs_without_legacy() {
        let ir = compile(
            r#"
            << "io";
            amount(x) := .amount x;
            !{
                ! io.print amount .{ amount:=12; label:="kept"; };
                ! io.print amount .{ amount:=9; approved:=1b; };
            };
        "#,
        );
        assert_eq!(run(&ir).unwrap().stdout, "12\n9\n");
        assert!(ir
            .arena
            .iter()
            .any(|n| matches!(n.kind, ExprKind::Record(_))));
    }
    #[test]
    fn typed_table_aggregate_uses_static_layout_even_for_no_rows() {
        let ir = compile(
            r#"
            << "df";
            helper(k:s) := .{ station:=k; impossible:=/ 1 0; };
            summarize(k:s,g:#.{station:s;}) := helper k;
            !{
                t:#.{station:s;} := #: .{ station:=[]; };
                df.agg &.station &summarize t
            };
        "#,
        );
        let p = view_entry(&ir).unwrap();
        assert_eq!(p.value, "station | impossible\n--------+-----------");
        assert!(p.stdout.is_empty());
    }
    #[test]
    fn lifted_direct_calls_encode_all_static_shape_modes() {
        use LiftMode::*;
        let ir = compile(
            r#"
            << "io";
            x := + 1 2;
            y := - [4 6] 1;
            z := - 9 [1 2];
            w := * [1 2] [2 3];
            !{ ! io.print (x,y,z,w); };
        "#,
        );
        let found = ir
            .groups
            .iter()
            .flatten()
            .filter_map(|d| ir.node(d.body).lift_mode)
            .collect::<Vec<_>>();
        assert_eq!(found, vec![Scalar, LeftSequence, RightSequence, Pairwise]);
        assert_eq!(run(&ir).unwrap().stdout, "(3, [3 5], [8 7], [2 6])\n");
        let ir = compile("x := + [1 2] [3]; !{ (); };");
        assert!(run(&ir)
            .err()
            .unwrap()
            .message
            .contains("sequence lengths differ"));
    }
}
