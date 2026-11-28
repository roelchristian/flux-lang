//! Executable core for owned typed HIR. This backend never dispatches on surface AST.
use crate::{
    diagnostic::{Category, Diagnostic, Related},
    eval::Output,
    hir::{
        BoundaryId, CoreOp, Definition, ExprId, ExprKind, HirProgram, LiftMode, Literal, Pattern,
        Span, Step, TableSchema, Target,
    },
    resolve::SymbolId,
    types::{Prim, Type},
    Error,
};
use num_bigint::BigInt;
use num_integer::Integer;
use num_rational::BigRational;
use num_traits::{Signed, ToPrimitive, Zero};
use std::{
    cmp::Ordering,
    collections::HashMap,
    io::{BufRead, Write},
    rc::Rc,
};
mod array_sequences012;
mod arrays012;
mod collections042;
mod combinators05;
mod decimal05;
mod display;
mod joins06;
mod json05;
mod library05;
mod math;
mod reduce014;
mod relations06;
mod wide05;
use display::{inspect, repr};
use wide05::Wide;
mod operations041;
pub use crate::csv::{FsInput, OpenedInput, RuntimeInput};

type R<T> = Result<T, Control>;
type NumericResult<T> = Result<T, &'static str>;
#[derive(Clone)]
struct Failure {
    payload: Rc<V>,
    origin: Span,
    propagations: Vec<Span>,
    diagnostic: Option<String>,
}
impl Failure {
    fn new(payload: Rc<V>, origin: Span, diagnostic: Option<String>) -> Rc<Self> {
        Rc::new(Self {
            payload,
            origin,
            propagations: Vec::new(),
            diagnostic,
        })
    }
}
enum Control {
    Fatal(Error),
    Raise {
        boundary: BoundaryId,
        failure: Rc<Failure>,
    },
}
impl Control {
    fn into_error(self) -> Error {
        match self {
            Self::Fatal(e) => e,
            Self::Raise { failure, .. } => Diagnostic::new(
                Category::RuntimeInvariant,
                failure.origin,
                "checked caret transfer escaped its lexical boundary",
            )
            .into_error(),
        }
    }
}
fn succeeded(value: V) -> V {
    V::Outcome(Ok(Rc::new(value)))
}
const OVERFLOW: &str = "exact arithmetic exceeds the prototype's i128 bound";
const DIV_BY_ZERO: &str = "division by zero";
const COLLECTION_LIMIT: usize = 100_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Domain {
    Natural,
    Integer,
    Rational,
    Decimal,
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
    fn compute(self, op: CoreOp, b: Self, domain: Domain) -> NumericResult<Self> {
        use CoreOp::*;
        use Domain::*;
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
                        out = out.compute(Mul, a, domain)?;
                    }
                    e >>= 1;
                    if e != 0 {
                        a = a.compute(Mul, a, domain)?;
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
        Type::Prim(Prim::D) => Some(Domain::Decimal),
        Type::Seq(inner) => domain(inner),
        _ => None,
    }
}
fn parse_inline_exact(s: &str, ty: &Type) -> NumericResult<Exact> {
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

fn exact_i128(n: i128, d: Domain) -> V {
    V::Exact(Exact::integer(n, d))
}

fn rational(n: BigInt, d: BigInt) -> NumericResult<V> {
    if d.is_zero() {
        return Err(DIV_BY_ZERO);
    }
    wide05::ratio_value(BigRational::new(n, d), Domain::Rational)
}
fn as_ratio(v: &V) -> NumericResult<(BigInt, BigInt)> {
    Ok(wide05::ratio(v)?.into_raw())
}
fn parse_exact_value(raw: &str, d: Domain) -> NumericResult<V> {
    if d == Domain::Decimal {
        return decimal05::decimal_literal(raw);
    }
    let ty = Type::Prim(match d {
        Domain::Natural => Prim::N,
        Domain::Integer => Prim::Z,
        _ => Prim::Q,
    });
    match parse_inline_exact(raw, &ty) {
        Ok(n) => return Ok(V::Exact(n)),
        Err(e) if e != OVERFLOW => return Err(e),
        Err(_) => {}
    }
    let value = if let Some((a, b)) = raw.split_once('.') {
        if d != Domain::Rational {
            return Err("fractional literal requires q");
        }
        let b = b.trim_end_matches('0');
        let decimal_digits = u32::try_from(b.len()).map_err(|_| wide05::RESOURCE)?;
        if decimal_digits > 315_652 {
            return Err(wide05::RESOURCE);
        }
        let num = wide05::parse_int(&format!("{a}{b}"))?;
        let den = BigInt::from(10u8).pow(decimal_digits);
        BigRational::new(num, den)
    } else {
        BigRational::from_integer(wide05::parse_int(raw)?)
    };
    wide05::ratio_value(value, d)
}
fn parse_exact(s: &str, ty: &Type) -> NumericResult<V> {
    let (raw, d) = match s.as_bytes().last() {
        Some(b'n') => (&s[..s.len() - 1], Domain::Natural),
        Some(b'z') => (&s[..s.len() - 1], Domain::Integer),
        Some(b'q') => (&s[..s.len() - 1], Domain::Rational),
        Some(b'd') => (&s[..s.len() - 1], Domain::Decimal),
        _ => (
            s,
            domain(ty).ok_or("numeric literal has no checked domain")?,
        ),
    };
    parse_exact_value(raw, d)
}

fn exact_binary(op: CoreOp, a: &V, b: &V, d: Domain) -> NumericResult<V> {
    if d == Domain::Decimal {
        return decimal05::decimal_binary(op, a, b);
    }
    if let (V::Exact(a), V::Exact(b)) = (a, b) {
        match a.compute(op, *b, d) {
            Ok(v) => return Ok(V::Exact(v)),
            Err(e) if e != OVERFLOW => return Err(e),
            Err(_) => {}
        }
    }
    wide05::binary(op, a, b, d)
}
fn real_of(v: &V) -> NumericResult<f64> {
    match v {
        V::Real(x) => Ok(*x),
        V::Exact(x) => Ok(x.real()),
        v => wide05::to_real(&wide05::ratio(v)?),
    }
}
fn to_exact(v: &V) -> NumericResult<V> {
    match v {
        V::Real(x) => match Exact::from_real(*x) {
            Ok(v) => Ok(V::Exact(v)),
            Err(e) if e != OVERFLOW => Err(e),
            Err(_) => wide05::ratio_value(wide05::from_real(*x)?, Domain::Rational),
        },
        _ => Ok(v.clone()),
    }
}
fn exact_index(v: &V) -> NumericResult<usize> {
    if let V::Exact(n) = v {
        let n = n.integral()?;
        if n < 0 {
            return Err("expected a nonnegative integer");
        }
        return Ok(usize::try_from(n).unwrap_or(usize::MAX));
    }
    let n = wide05::integral(v)?;
    if n.is_negative() {
        return Err("expected a nonnegative integer");
    }
    Ok(n.to_usize().unwrap_or(usize::MAX))
}
fn exact_unary(op: CoreOp, v: &V, d: Domain) -> NumericResult<V> {
    if d == Domain::Decimal {
        return decimal05::decimal_unary(op, v);
    }
    if let V::Exact(n) = v {
        let num = if op == CoreOp::Abs {
            n.n.checked_abs()
        } else {
            n.n.checked_neg()
        };
        if let Some(i) = num {
            return Ok(V::Exact(Exact {
                n: i,
                d: n.d,
                domain: d,
            }));
        }
    }
    let v = wide05::ratio(v)?;
    wide05::ratio_value(if op == CoreOp::Abs { v.abs() } else { -v }, d)
}
fn range(lo: &V, hi: &V, d: Domain) -> NumericResult<V> {
    let too_big = "prototype range limit (100000 items) exceeded";
    let out: Vec<V> = if let (V::Exact(lo), V::Exact(hi)) = (lo, hi) {
        let (lo, hi) = (lo.integral()?, hi.integral()?);
        if hi >= lo
            && hi
                .checked_sub(lo)
                .is_none_or(|n| n >= COLLECTION_LIMIT as i128)
        {
            return Err(too_big);
        }
        (lo..=hi).map(|i| exact_i128(i, d)).collect()
    } else {
        let (mut lo, hi) = (wide05::integral(lo)?, wide05::integral(hi)?);
        if lo > hi {
            return Ok(V::Sequence(Rc::from([])));
        }
        let n = (&hi - &lo)
            .to_usize()
            .filter(|x| *x < COLLECTION_LIMIT)
            .ok_or(too_big)?;
        // Repeating a large endpoint up to the ordinary item cap could
        // allocate gigabytes without growing any individual integer.
        if lo.bits().max(hi.bits()).saturating_mul((n + 1) as u64) > 64 * 1024 * 1024 * 8 {
            return Err("wide range resource limit exceeded (64 MiB of integer data)");
        }
        let mut out = Vec::with_capacity(n + 1);
        for _ in 0..=n {
            out.push(wide05::ratio_value(
                BigRational::from_integer(lo.clone()),
                d,
            )?);
            lo += 1u8;
        }
        out
    };
    Ok(V::Sequence(out.into()))
}

#[derive(Clone)]
enum V {
    Exact(Exact),
    Wide(Rc<Wide>),
    Real(f64),
    Bool(bool),
    String(Rc<str>),
    Char(char),
    Unit,
    Tuple(Rc<[V]>),
    Sequence(Rc<[V]>),
    Dense(Rc<arrays012::Dense>),
    Record(Rc<[(Box<str>, V)]>),
    Table(Rc<[V]>, Rc<TableSchema>),
    Dict(Rc<[(V, V)]>),
    Optional(Option<Rc<V>>),
    Variant(Rc<VariantValue>),
    // Keep exceptional provenance off all ordinary numeric/collection values.
    Outcome(Result<Rc<V>, Rc<Failure>>),
    Fun(Rc<Fun>),
    Action(Rc<Action>),
}
/// Keep sums off the hot path for ordinary sequence element layouts.
struct VariantValue {
    constructor: SymbolId,
    name: Rc<str>,
    fields: Rc<[V]>,
}

#[derive(Clone)]
enum Function {
    User {
        formals: Rc<[SymbolId]>,
        body: ExprId,
        env: usize,
        declared: Type,
        open_rows: bool,
    },
    Core(CoreOp, Type, Option<Rc<TableSchema>>),
    Constructor(SymbolId, usize),
    Projection(Box<str>),
}
#[derive(Clone)]
struct Fun {
    code: Function,
    bound: Vec<V>,
    // Types come from checked expressions/signatures, never from Value tags.
    bound_types: Vec<Type>,
    instance: Option<Type>,
}
fn function(code: Function) -> V {
    V::Fun(Rc::new(Fun {
        code,
        bound: Vec::new(),
        bound_types: Vec::new(),
        instance: None,
    }))
}
#[derive(Clone)]
enum Action {
    Block { expr: ExprId, env: usize },
    IoRead(Rc<str>),
    IoWrite(Rc<str>, Rc<str>),
    IoExists(Rc<str>),
    IoPrint(V),
    CsvRead(Rc<str>, Option<Rc<TableSchema>>),
    CsvWrite(Rc<str>, V),
    Library(CoreOp, Vec<V>),
}
#[derive(Default)]
struct Frame {
    parent: Option<usize>,
    ready: HashMap<SymbolId, V>,
    type_scope: Option<usize>,
    substs: HashMap<u32, Type>,
    row_substs: Option<Box<RowInstances>>,
}
type RowInstances = HashMap<u32, RowInstance>;
struct RowInstance {
    /// Only tail-owned fields may supply field types to the output occurrence.
    residual: crate::types::Row,
    /// Preserve the checked input's interleaving of required and extra fields.
    order: Vec<String>,
    fixed: Vec<String>,
}
struct Eval<'a> {
    p: &'a HirProgram,
    frames: Vec<Frame>,
    input: &'a mut dyn RuntimeInput,
    output: &'a mut dyn Write,
    stdin: &'a mut dyn BufRead,
    args: &'a [String],
    fuel: usize,
    depth: u16,
    effects: u16,
}
fn error(span: Span, message: impl Into<String>) -> Control {
    Control::Fatal(Diagnostic::new(Category::RuntimeDomain, span, message).into_error())
}
fn invariant(span: Span, message: impl Into<String>) -> Control {
    Control::Fatal(Diagnostic::new(Category::RuntimeInvariant, span, message).into_error())
}
fn uncaught(program: &HirProgram, f: &Failure) -> Error {
    let declared = program.entry.and_then(|id| {
        let ty = &program.node(id).ty;
        let Type::Action(value) = ty else { return None };
        let Type::Result(_, err) = value.as_ref() else {
            return None;
        };
        Some(err.as_ref())
    });
    let shown = declared
        .map(|t| format!("{t}: {}", repr(&f.payload)))
        .unwrap_or_else(|| repr(&f.payload));
    let mut d = Diagnostic::new(
        Category::RuntimeOperational,
        f.origin,
        format!("uncaught operational failure: {shown}"),
    );
    if let Some(detail) = &f.diagnostic {
        d.related.push(Related {
            span: f.origin,
            label: detail.clone(),
        });
    }
    for &at in &f.propagations {
        d.related.push(Related {
            span: at,
            label: "propagated here".into(),
        });
    }
    d.into_error()
}
// A schema's stored order controls presentation, not structural equality.
// Preserve order for tuple slots and retain lexical identity for nominal types.
fn same_field_types(a: &[(String, Type)], b: &[(String, Type)]) -> bool {
    a.len() == b.len()
        && a.iter().all(|(name, ty)| {
            b.iter()
                .find(|(other, _)| name == other)
                .is_some_and(|(_, rhs)| same_type_layout(ty, rhs))
        })
}
fn same_tuple_types(a: &[Type], b: &[Type]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_type_layout(a, b))
}
fn same_type_layout(a: &Type, b: &Type) -> bool {
    use Type::*;
    match (a, b) {
        (Record(a), Record(b)) => a.tail == b.tail && same_field_types(&a.fields, &b.fields),
        (Tuple(a), Tuple(b)) => same_tuple_types(a, b),
        (Seq(a), Seq(b))
        | (Table(a), Table(b))
        | (Optional(a), Optional(b))
        | (Action(a), Action(b)) => same_type_layout(a, b),
        (Dict(a, b), Dict(c, d)) | (Result(a, b), Result(c, d)) | (Fun(a, b), Fun(c, d)) => {
            same_type_layout(a, c) && same_type_layout(b, d)
        }
        _ => a == b,
    }
}
fn same_table_layout(a: &TableSchema, b: &TableSchema) -> bool {
    a == b
        || match (a, b) {
            (TableSchema::Named(a), TableSchema::Named(b)) => same_field_types(a, b),
            (TableSchema::Positional(a), TableSchema::Positional(b)) => same_tuple_types(a, b),
            _ => false,
        }
}
fn equal(a: &V, b: &V) -> bool {
    match (a, b) {
        (V::Dense(a), V::Dense(b)) => a.equal(b),
        (V::Exact(a), V::Exact(b)) => a.n == b.n && a.d == b.d,
        (V::Exact(_) | V::Wide(_), V::Exact(_) | V::Wide(_)) => {
            order(a, b).is_ok_and(|v| v.is_eq())
        }
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
            same_table_layout(ta, tb)
                && a.len() == b.len()
                && a.iter().zip(b.iter()).all(|(a, b)| equal(a, b))
        }
        (V::Dict(a), V::Dict(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, v)| b.iter().any(|(l, w)| equal(k, l) && equal(v, w)))
        }
        (V::Variant(a), V::Variant(b)) => {
            a.constructor == b.constructor
                && a.fields.len() == b.fields.len()
                && a.fields
                    .iter()
                    .zip(b.fields.iter())
                    .all(|(x, y)| equal(x, y))
        }
        (V::Outcome(Ok(a)), V::Outcome(Ok(b))) => equal(a, b),
        (V::Outcome(Err(a)), V::Outcome(Err(b))) => equal(&a.payload, &b.payload),
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
const KEY_WORK_LIMIT: usize = 64 * 1024 * 1024;

#[derive(PartialEq, Eq, Hash)]
enum Key {
    Num(i128, i128),
    WideNum(Box<BigRational>),
    Real(u64),
    Bool(bool),
    Str(Rc<str>),
    Char(char),
    Unit,
    None,
    Some(Box<Key>),
    Tuple(Box<[Key]>),
}
/// Builtins whose evaluator needs checked input types, not only the result
/// type and lift plan. Direct calls and saturated references share this path.
enum WitnessedOp {
    Dense(CoreOp),
    Indexed(CoreOp),
    Join(CoreOp),
    Relation(CoreOp),
    Combinator(CoreOp),
    ArrayOverload(CoreOp, usize),
}

fn witnessed_op(op: CoreOp) -> Option<WitnessedOp> {
    Some(if arrays012::handles(op) {
        WitnessedOp::Dense(op)
    } else if array_sequences012::handles(op) {
        WitnessedOp::Indexed(op)
    } else if joins06::handles(op) {
        WitnessedOp::Join(op)
    } else if relations06::handles(op) {
        WitnessedOp::Relation(op)
    } else if combinators05::handles(op) {
        WitnessedOp::Combinator(op)
    } else {
        let index = arrays012::overloaded_operand(op)?;
        WitnessedOp::ArrayOverload(op, index)
    })
}
fn key(v: &V) -> Option<Key> {
    let mut budget = KEY_WORK_LIMIT;
    key_with_budget(v, &mut budget, 0)
}
fn key_with_budget(v: &V, budget: &mut usize, depth: usize) -> Option<Key> {
    if depth >= 128 {
        return None;
    }
    *budget = budget.checked_sub(std::mem::size_of::<Key>())?;
    Some(match v {
        V::Exact(x) => Key::Num(x.n, x.d),
        V::Wide(_) => {
            let q = wide05::ratio(v).ok()?;
            let bytes = (q.numer().bits() + q.denom().bits()).div_ceil(8);
            *budget = budget.checked_sub(usize::try_from(bytes).ok()?)?;
            if let (Some(n), Some(d)) = (q.numer().to_i128(), q.denom().to_i128()) {
                Key::Num(n, d)
            } else {
                Key::WideNum(Box::new(q))
            }
        }
        V::Real(r) => Key::Real(if *r == 0.0 { 0 } else { r.to_bits() }),
        V::Bool(b) => Key::Bool(*b),
        V::String(s) => {
            *budget = budget.checked_sub(s.len())?;
            Key::Str(s.clone())
        }
        V::Char(c) => Key::Char(*c),
        V::Unit => Key::Unit,
        V::Optional(None) => Key::None,
        V::Optional(Some(x)) => Key::Some(Box::new(key_with_budget(x, budget, depth + 1)?)),
        V::Tuple(xs) => Key::Tuple(
            xs.iter()
                .map(|x| key_with_budget(x, budget, depth + 1))
                .collect::<Option<Vec<_>>>()?
                .into_boxed_slice(),
        ),
        _ => return None,
    })
}
fn order(a: &V, b: &V) -> NumericResult<Ordering> {
    match (a, b) {
        (V::Exact(a), V::Exact(b)) => Ok(a.order(*b)),
        (V::Exact(_) | V::Wide(_), V::Exact(_) | V::Wide(_)) => {
            Ok(wide05::ratio(a)?.cmp(&wide05::ratio(b)?))
        }
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
fn contains_open_row(ty: &Type) -> bool {
    match ty {
        Type::Record(row) => {
            row.tail.is_some() || row.fields.iter().any(|(_, ty)| contains_open_row(ty))
        }
        Type::Seq(t) | Type::Table(t) | Type::Optional(t) | Type::Action(t) => contains_open_row(t),
        Type::Dict(a, b) | Type::Fun(a, b) | Type::Result(a, b) => {
            contains_open_row(a) || contains_open_row(b)
        }
        Type::Tuple(ts) | Type::Applied(_, ts) => ts.iter().any(contains_open_row),
        _ => false,
    }
}
fn dynamic_schema_operation(op: CoreOp) -> bool {
    matches!(
        op,
        CoreOp::TableFrom
            | CoreOp::TableRows
            | CoreOp::Aggregate
            | CoreOp::Extend
            | CoreOp::Cartesian
            | CoreOp::CsvRead
    )
}
fn fit_instance(
    pattern: &Type,
    actual: &Type,
    vars: &mut HashMap<u32, Type>,
    rows: &mut RowInstances,
) {
    match (pattern, actual) {
        (Type::Var(i), t) if !matches!(t, Type::Var(_)) => {
            vars.entry(*i).or_insert_with(|| t.clone());
        }
        (Type::Seq(a), Type::Seq(b))
        | (Type::Table(a), Type::Table(b))
        | (Type::Optional(a), Type::Optional(b))
        | (Type::Action(a), Type::Action(b)) => fit_instance(a, b, vars, rows),
        (Type::Dict(a, b), Type::Dict(c, d))
        | (Type::Fun(a, b), Type::Fun(c, d))
        | (Type::Result(a, b), Type::Result(c, d)) => {
            fit_instance(a, c, vars, rows);
            fit_instance(b, d, vars, rows);
        }
        (Type::Applied(a, args), Type::Applied(b, actual))
            if a == b && args.len() == actual.len() =>
        {
            for (arg, actual) in args.iter().zip(actual) {
                fit_instance(arg, actual, vars, rows);
            }
        }
        (Type::Tuple(a), Type::Tuple(b)) => {
            for (a, b) in a.iter().zip(b) {
                fit_instance(a, b, vars, rows);
            }
        }
        (Type::Record(a), Type::Record(b)) => {
            if let Some(tail) = a.tail {
                // An identical unresolved tail is not additional evidence.
                if b.tail != Some(tail)
                    || b.fields
                        .iter()
                        .any(|(name, _)| !a.fields.iter().any(|(fixed, _)| fixed == name))
                {
                    let capture = || RowInstance {
                        residual: crate::types::Row {
                            fields: b
                                .fields
                                .iter()
                                .filter(|(name, _)| {
                                    !a.fields.iter().any(|(fixed, _)| fixed == name)
                                })
                                .cloned()
                                .collect(),
                            tail: b.tail,
                        },
                        order: b.fields.iter().map(|(name, _)| name.clone()).collect(),
                        fixed: a.fields.iter().map(|(name, _)| name.clone()).collect(),
                    };
                    match rows.entry(tail) {
                        std::collections::hash_map::Entry::Vacant(slot) => {
                            slot.insert(capture());
                        }
                        std::collections::hash_map::Entry::Occupied(mut slot)
                            if slot.get().residual.tail.is_some() && b.tail.is_none() =>
                        {
                            slot.insert(capture());
                        }
                        _ => {}
                    }
                }
            }
            for (n, a) in &a.fields {
                if let Some((_, b)) = b.fields.iter().find(|(m, _)| n == m) {
                    fit_instance(a, b, vars, rows);
                }
            }
        }
        _ => {}
    }
}
fn fun_arg(mut sig: &Type, i: usize) -> Option<&Type> {
    for _ in 0..i {
        if let Type::Fun(_, next) = sig {
            sig = next
        } else {
            return None;
        }
    }
    if let Type::Fun(arg, _) = sig {
        Some(arg)
    } else {
        None
    }
}
fn callable_lift(op: CoreOp, operands: &[Type]) -> NumericResult<Option<LiftMode>> {
    let Some(arity) = op.lift_arity() else {
        return Ok(None);
    };
    if operands.len() != arity {
        return Err("invalid typed lifting argument count");
    }
    if operands.iter().any(|ty| matches!(ty, Type::Var(_))) {
        return Err("numeric lift shape is unresolved at indirect application");
    }
    LiftMode::for_operands(&operands[0], operands.get(1)).map(Some)
}
fn inner_type(t: &Type) -> &Type {
    if let Type::Seq(x) = t {
        x
    } else {
        t.array_element().unwrap_or(t)
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
    let mode = mode.ok_or("direct or indirect call lacks checked lifting shape")?;
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
        LiftMode::LeftArray | LiftMode::RightArray | LiftMode::ArrayPair => {
            arrays012::lift_binary(a, b, mode, f)?
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
    let mode = mode.ok_or("direct or indirect call lacks checked lifting shape")?;
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
        LiftMode::LeftArray => arrays012::lift_unary(a, f),
        _ => Err("invalid unary lifting mode in typed HIR"),
    }
}

fn scalar_binary(op: CoreOp, a: V, b: V, ty: &Type) -> NumericResult<V> {
    use CoreOp::*;
    Ok(match op {
        Add | Sub | Mul | Div | Quotient | Remainder | Pow => {
            if matches!(ty, Type::Prim(Prim::R)) {
                let r = |v| {
                    if let V::Real(x) = v {
                        Ok(x)
                    } else {
                        Err("expected a checked real operand")
                    }
                };
                if op == Pow {
                    let mut a = r(a)?;
                    let e = if let V::Exact(n) = &b {
                        u128::try_from(n.integral()?)
                            .ok()
                            .filter(|_| n.domain == Domain::Natural)
                            .ok_or("power requires a natural exponent")?
                    } else {
                        let e = wide05::exponent(&b)?;
                        match a {
                            1.0 => return finite(1.0),
                            -1.0 => return finite(if e.is_odd() { -1.0 } else { 1.0 }),
                            x if x == 0.0 => {
                                return finite(if e.is_zero() {
                                    1.0
                                } else if e.is_odd() {
                                    x
                                } else {
                                    0.0
                                })
                            }
                            _ => {}
                        }
                        e.to_u128().ok_or(wide05::RESOURCE)?
                    };
                    let mut exponent = e;
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
            let k = domain(ty).ok_or("numeric domain is unresolved at typed execution")?;
            exact_binary(op, &a, &b, k)?
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
fn numeric_identity(ty: &Type, product: bool) -> NumericResult<V> {
    let identity = i128::from(product);
    if *ty == Type::Prim(Prim::R) {
        return Ok(V::Real(identity as f64));
    }
    let d = domain(ty).ok_or("reduction has no checked numeric domain")?;
    if d == Domain::Decimal {
        decimal05::decimal_literal(if product { "1" } else { "0" })
    } else {
        Ok(exact_i128(identity, d))
    }
}

fn numeric_reduce(op: CoreOp, items: impl IntoIterator<Item = V>, ty: &Type) -> NumericResult<V> {
    let product = op == CoreOp::Product;
    let choice = if product { CoreOp::Mul } else { CoreOp::Add };
    let mut accumulator = numeric_identity(ty, product)?;
    for item in items {
        accumulator = scalar_binary(choice, accumulator, item, ty)?;
    }
    Ok(accumulator)
}

fn operation_arity(op: CoreOp, span: Span) -> R<usize> {
    crate::builtins::for_operation(op)
        .map(|m| m.arity)
        .ok_or_else(|| {
            invariant(
                span,
                format!("typed operation {op:?} has no registry identity"),
            )
        })
}
impl Eval<'_> {
    fn frame(&mut self, parent: usize) -> usize {
        let id = self.frames.len();
        self.frames.push(Frame {
            parent: Some(parent),
            ready: HashMap::new(),
            type_scope: self.frames[parent].type_scope,
            substs: HashMap::new(),
            row_substs: None,
        });
        id
    }
    fn type_at(&self, ty: &Type, env: usize) -> Type {
        fn subst(e: &Eval<'_>, ty: &Type, scope: Option<usize>, fuel: usize) -> Type {
            if scope.is_none() || fuel == 0 {
                return ty.clone();
            }
            let recur = |t: &Type| subst(e, t, scope, fuel - 1);
            match ty {
                Type::Var(id) => {
                    let mut cursor = scope;
                    while let Some(i) = cursor {
                        let frame = &e.frames[i];
                        if let Some(t) = frame.substs.get(id) {
                            return subst(
                                e,
                                t,
                                frame.parent.and_then(|p| e.frames[p].type_scope),
                                fuel - 1,
                            );
                        }
                        cursor = frame.parent.and_then(|p| e.frames[p].type_scope);
                    }
                    ty.clone()
                }
                Type::Seq(t) => Type::Seq(Box::new(recur(t))),
                Type::Table(t) => Type::Table(Box::new(recur(t))),
                Type::Optional(t) => Type::Optional(Box::new(recur(t))),
                Type::Action(t) => Type::Action(Box::new(recur(t))),
                Type::Tuple(ts) => Type::Tuple(ts.iter().map(recur).collect()),
                Type::Fun(a, b) => Type::Fun(Box::new(recur(a)), Box::new(recur(b))),
                Type::Result(a, b) => Type::Result(Box::new(recur(a)), Box::new(recur(b))),
                Type::Dict(a, b) => Type::Dict(Box::new(recur(a)), Box::new(recur(b))),
                Type::Applied(owner, args) => {
                    Type::Applied(owner.clone(), args.iter().map(recur).collect())
                }
                Type::Record(r) => {
                    let mut r = r.clone();
                    for (_, t) in &mut r.fields {
                        *t = recur(t);
                    }
                    let mut cursor = scope;
                    let mut steps = fuel;
                    while let (Some(tail), Some(i)) = (r.tail, cursor) {
                        if steps == 0 {
                            break;
                        }
                        steps -= 1;
                        let frame = &e.frames[i];
                        cursor = frame.parent.and_then(|p| e.frames[p].type_scope);
                        let Some(witness) =
                            frame.row_substs.as_ref().and_then(|rows| rows.get(&tail))
                        else {
                            continue;
                        };
                        let preserve_interleaving = witness
                            .fixed
                            .iter()
                            .all(|fixed| r.fields.iter().any(|(name, _)| fixed == name));
                        for (name, t) in &witness.residual.fields {
                            if !r.fields.iter().any(|(existing, _)| existing == name) {
                                r.fields.push((name.clone(), subst(e, t, cursor, steps)));
                            }
                        }
                        if preserve_interleaving {
                            // Existing columns retain their checked input order; fields
                            // introduced by this expression follow in declaration order.
                            r.fields.sort_by_key(|(name, _)| {
                                witness
                                    .order
                                    .iter()
                                    .position(|original| original == name)
                                    .unwrap_or(usize::MAX)
                            });
                        }
                        r.tail = witness.residual.tail;
                    }
                    Type::Record(r)
                }
                _ => ty.clone(),
            }
        }
        subst(self, ty, self.frames[env].type_scope, 64)
    }
    fn specialize_fun(&self, v: V, ty: &Type, env: usize) -> V {
        if let V::Fun(f) = v {
            if matches!(ty, Type::Fun(..) | Type::Var(_)) {
                let ty = self.type_at(ty, env);
                if matches!(ty, Type::Fun(..)) && f.instance.as_ref() != Some(&ty) {
                    let mut f = Rc::unwrap_or_clone(f);
                    f.instance = Some(ty);
                    return V::Fun(Rc::new(f));
                }
            }
            V::Fun(f)
        } else {
            v
        }
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
            declared: d.scheme.ty.clone(),
            open_rows: contains_open_row(&d.scheme.ty),
        };
        self.put(env, d.symbol, function(code));
    }
    fn make_variant(&self, id: SymbolId, fields: Vec<V>, span: Span) -> R<V> {
        let info = self
            .p
            .constructors
            .get(&id)
            .ok_or_else(|| invariant(span, "checked constructor identity is absent"))?;
        if fields.len() != info.payload.len() {
            return Err(invariant(span, "checked constructor payload arity differs"));
        }
        Ok(V::Variant(Rc::new(VariantValue {
            constructor: id,
            name: info.name.clone(),
            fields: fields.into(),
        })))
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
        r.map(|v| self.specialize_fun(v, &node.ty, env))
    }
    fn eval_kind(&mut self, id: ExprId, env: usize) -> R<V> {
        let p = self.p;
        let n = p.node(id);
        let s = n.span;
        let convert = |message| error(s, message);
        Ok(match &n.kind {
            ExprKind::Literal(v) => match v {
                Literal::Integer(i) => {
                    let ty = self.type_at(&n.ty, env);
                    if ty == Type::Prim(Prim::R) {
                        finite(*i as f64).map_err(convert)?
                    } else {
                        let d = domain(&ty).ok_or_else(|| {
                            error(s, "integer literal has no checked numeric domain")
                        })?;
                        if d == Domain::Decimal {
                            decimal05::decimal_literal(&i.to_string()).map_err(convert)?
                        } else {
                            V::Exact(Exact::integer(*i, d).as_domain(d).map_err(convert)?)
                        }
                    }
                }
                Literal::Numeric(text) => {
                    let ty = self.type_at(&n.ty, env);
                    if text.ends_with('r') || ty == Type::Prim(Prim::R) {
                        let raw = text.strip_suffix('r').unwrap_or(text);
                        finite(raw.parse().map_err(|_| error(s, "invalid real literal"))?)
                            .map_err(convert)?
                    } else {
                        parse_exact(text, &ty).map_err(convert)?
                    }
                }
                Literal::Bool(b) => V::Bool(*b),
                Literal::String(text) => V::String(Rc::from(text.as_ref())),
                Literal::Char(c) => V::Char(*c),
                Literal::Unit => V::Unit,
            },
            ExprKind::Load(sid) => self.load(env, *sid, s)?,
            ExprKind::Reference(Target::Symbol(sid)) => self.load(env, *sid, s)?,
            ExprKind::Reference(Target::Constructor(id)) => {
                let arity = p
                    .constructors
                    .get(id)
                    .ok_or_else(|| invariant(s, "checked constructor is absent"))?
                    .payload
                    .len();
                if arity == 0 {
                    self.make_variant(*id, Vec::new(), s)?
                } else {
                    function(Function::Constructor(*id, arity))
                }
            }
            ExprKind::Reference(Target::Projection(field)) => {
                function(Function::Projection(field.clone()))
            }
            ExprKind::Reference(Target::Builtin(op)) => function(Function::Core(
                *op,
                self.type_at(&n.ty, env),
                n.table_schema.clone(),
            )),
            ExprKind::Call(Target::Builtin(CoreOp::TableFrom), args)
                if n.table_schema.is_none()
                    && matches!(
                        &p.node(args[0]).kind,
                        ExprKind::Call(Target::Builtin(CoreOp::Rows), _)
                    ) =>
            {
                // The direct round trip preserves its actual input layout.
                // Two arguments can share an open row type while differing
                // in column order; the other argument is not its witness.
                let ExprKind::Call(_, source) = &p.node(args[0]).kind else {
                    unreachable!("checked row round trip")
                };
                match self.eval(source[0], env)? {
                    table @ V::Table(_, _) => table,
                    _ => return Err(invariant(s, "checked row round trip needs a table")),
                }
            }
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
            ExprKind::Call(target, ids) => {
                let f = match target {
                    Target::Symbol(sid) => Some(self.load(env, *sid, s)?),
                    Target::Builtin(_) | Target::Projection(_) | Target::Constructor(_) => None,
                };
                let args = ids
                    .iter()
                    .map(|id| self.eval(*id, env))
                    .collect::<R<Vec<_>>>()?;
                let arg_types = |eval: &Self| {
                    ids.iter()
                        .map(|id| eval.type_at(&p.node(*id).ty, env))
                        .collect::<Vec<_>>()
                };
                match target {
                    Target::Builtin(CoreOp::Apply | CoreOp::ApplyTuple) => {
                        let [f, arg]: [V; 2] = args
                            .try_into()
                            .map_err(|_| error(s, "invalid typed application"))?;
                        let input = self.type_at(&p.node(ids[1]).ty, env);
                        let output = self.type_at(&n.ty, env);
                        if matches!(target, Target::Builtin(CoreOp::ApplyTuple)) {
                            let (V::Tuple(v), Type::Tuple(types)) = (arg, input) else {
                                return Err(error(s, "@@ expects a checked tuple bundle"));
                            };
                            let mut f = f;
                            let sig = self.type_at(&p.node(ids[0]).ty, env);
                            for (i, (operand, actual)) in v.iter().zip(types).enumerate() {
                                let next = result_type(&sig, i + 1);
                                let result = if i + 1 == v.len() {
                                    output.clone()
                                } else {
                                    next.clone()
                                };
                                f = self.apply_unary(f, operand.clone(), actual, result, s)?;
                            }
                            f
                        } else {
                            self.apply_unary(f, arg, input, output, s)?
                        }
                    }
                    Target::Builtin(op) => {
                        let ty = match &n.evidence {
                            Some(crate::infer::Operation::Numeric { result, .. }) => result,
                            _ => &n.ty,
                        };
                        let ty = self.type_at(ty, env);
                        if let Some(op) = witnessed_op(*op) {
                            self.witnessed_core(
                                op,
                                args,
                                &arg_types(self),
                                &ty,
                                n.table_schema.as_ref(),
                                s,
                            )?
                        } else {
                            let layout = if dynamic_schema_operation(*op)
                                && self.frames[env].type_scope.is_some()
                            {
                                self.p.instance_schema(&ty)
                            } else {
                                None
                            };
                            self.core(
                                *op,
                                args,
                                &ty,
                                layout.as_ref().or(n.table_schema.as_ref()),
                                n.lift_mode,
                                s,
                            )?
                        }
                    }
                    Target::Constructor(id) => self.make_variant(*id, args, s)?,
                    Target::Projection(field) => {
                        let [v]: [V; 1] = args
                            .try_into()
                            .map_err(|_| error(s, "invalid projection operand count"))?;
                        projection(field, v, s)?
                    }
                    Target::Symbol(_) => {
                        let mut f = f.expect("callee loaded");
                        if let Some(sig) = n.callee_type.as_ref() {
                            f = self.specialize_fun(f, sig, env);
                            if !matches!(&f, V::Fun(f) if matches!(f.code, Function::User { open_rows: true, .. }))
                            {
                                return self.apply(f, args, s);
                            }
                        }
                        let input = arg_types(self);
                        let output = self.type_at(&n.ty, env);
                        self.apply_checked(f, args, Some(input), Some(output), s)?
                    }
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
            ExprKind::Closure { formals, body } => function(Function::User {
                formals: formals.clone().into(),
                body: *body,
                env,
                declared: n.ty.clone(),
                open_rows: contains_open_row(&n.ty),
            }),
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
            ExprKind::Fallible {
                boundary,
                steps,
                tail,
            } => match self.block(steps, *tail, env) {
                Ok(v) => succeeded(v),
                Err(Control::Raise {
                    boundary: target,
                    failure,
                }) if target == *boundary => V::Outcome(Err(failure)),
                Err(other) => return Err(other),
            },
            ExprKind::Raise { boundary, payload } => {
                let v = self.eval(*payload, env)?;
                return Err(Control::Raise {
                    boundary: *boundary,
                    failure: Failure::new(Rc::new(v), s, None),
                });
            }
            ExprKind::Propagate { boundary, result } => match self.eval(*result, env)? {
                V::Outcome(Ok(v)) => v.as_ref().clone(),
                V::Outcome(Err(mut f)) => {
                    Rc::make_mut(&mut f).propagations.push(s);
                    return Err(Control::Raise {
                        boundary: *boundary,
                        failure: f,
                    });
                }
                _ => return Err(invariant(s, "checked propagation operand was not Result")),
            },
            ExprKind::Match { value, arms } => {
                let value = self.eval(*value, env)?;
                for arm in arms {
                    if let Some(bindings) = self.match_value(&arm.pattern, &value)? {
                        let local = self.frame(env);
                        for (id, value) in bindings {
                            self.put(local, id, value.clone());
                        }
                        return self.eval(arm.body, local);
                    }
                }
                return Err(invariant(s, "checked match did not cover the scrutinee"));
            }
            ExprKind::Recover { result, arms } => match self.eval(*result, env)? {
                V::Outcome(Ok(v)) => v.as_ref().clone(),
                V::Outcome(Err(f)) => {
                    for arm in arms {
                        let bindings = match &arm.pattern {
                            Pattern::Wildcard => Vec::new(),
                            Pattern::Bind(id) => vec![(*id, f.payload.as_ref().clone())],
                            Pattern::Constructor { id, fields } => {
                                let V::Variant(variant) = f.payload.as_ref() else {
                                    continue;
                                };
                                let payload = &variant.fields;
                                if variant.constructor != *id {
                                    continue;
                                }
                                if fields.len() != payload.len() {
                                    return Err(invariant(
                                        arm.span,
                                        "checked recovery pattern arity differs",
                                    ));
                                }
                                fields
                                    .iter()
                                    .zip(payload.iter())
                                    .filter_map(|(id, v)| id.map(|i| (i, v.clone())))
                                    .collect()
                            }
                        };
                        let local = self.frame(env);
                        for (id, v) in bindings {
                            self.put(local, id, v);
                        }
                        return self.eval(arm.body, local);
                    }
                    return Err(invariant(
                        s,
                        "checked recovery did not cover the failure variant",
                    ));
                }
                _ => return Err(invariant(s, "checked recovery operand was not Result")),
            },
            ExprKind::Block {
                suspended: true, ..
            } => V::Action(Rc::new(Action::Block { expr: id, env })),
            ExprKind::Block { steps, tail, .. } => self.block(steps, *tail, env)?,
        })
    }
    fn match_value<'v>(
        &mut self,
        pattern: &crate::hir::ValuePattern,
        value: &'v V,
    ) -> R<Option<Vec<(SymbolId, &'v V)>>> {
        use crate::hir::ValuePatternKind as P;
        let mut bindings = Vec::new();
        let mut stack = vec![(pattern, value)];
        while let Some((pattern, value)) = stack.pop() {
            self.tick(pattern.span)?;
            match (&pattern.kind, value) {
                (P::Wildcard, _) => {}
                (P::Bind(id), value) => bindings.push((*id, value)),
                (P::Constructor(id, fields), V::Variant(actual))
                    if *id == actual.constructor && fields.len() == actual.fields.len() =>
                {
                    stack.extend(fields.iter().zip(actual.fields.iter()).rev());
                }
                (P::Tuple(parts), V::Tuple(values)) if parts.len() == values.len() => {
                    stack.extend(parts.iter().zip(values.iter()).rev());
                }
                (P::Record(parts), V::Record(values)) => {
                    for (name, part) in parts.iter().rev() {
                        let Some((_, value)) = values.iter().find(|(key, _)| key == name) else {
                            return Ok(None);
                        };
                        stack.push((part, value));
                    }
                }
                (P::Optional(Some(part)), V::Optional(Some(value))) => stack.push((part, value)),
                (P::Optional(None), V::Optional(None)) | (P::Unit, V::Unit) => {}
                (P::Bool(expected), V::Bool(actual)) if expected == actual => {}
                _ => return Ok(None),
            }
        }
        Ok(Some(bindings))
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
        self.apply_checked(f, args, None, None, span)
    }
    fn apply_unary(&mut self, f: V, arg: V, input: Type, output: Type, span: Span) -> R<V> {
        self.apply_checked(f, vec![arg], Some(vec![input]), Some(output), span)
    }

    /// Apply both stages of a curried callback, in order, without losing
    /// the checked intermediate function type at the first application.
    fn apply_dyadic(
        &mut self,
        f: &V,
        a: (V, &Type),
        b: (V, &Type),
        out: &Type,
        span: Span,
    ) -> R<V> {
        let residual = Type::Fun(Box::new(b.1.clone()), Box::new(out.clone()));
        let partial = self.apply_unary(f.clone(), a.0, a.1.clone(), residual, span)?;
        self.apply_unary(partial, b.0, b.1.clone(), out.clone(), span)
    }

    fn apply_checked(
        &mut self,
        f: V,
        args: Vec<V>,
        supplied_types: Option<Vec<Type>>,
        out_type: Option<Type>,
        span: Span,
    ) -> R<V> {
        self.tick(span)?;
        let V::Fun(f) = f else {
            return Err(error(span, "expected a callable function value"));
        };
        let mut f = Rc::unwrap_or_clone(f);
        let n = f.bound.len();
        let sig = match &f.code {
            Function::User { declared, .. } | Function::Core(_, declared, _) => Some(declared),
            Function::Projection(_) | Function::Constructor(..) => None,
        };
        let suggested = f.instance.as_ref();
        let out = out_type.or_else(|| suggested.map(|sig| result_type(sig, args.len()).clone()));
        for (i, actual) in supplied_types
            .unwrap_or_else(|| vec![Type::Var(u32::MAX); args.len()])
            .into_iter()
            .enumerate()
        {
            // A contextual formal expectation can be more informative than a
            // polymorphic call's supplied expression (e.g., a higher-order ref).
            let expected = suggested
                .and_then(|sig| fun_arg(sig, i))
                .or_else(|| sig.and_then(|sig| fun_arg(sig, n + i)));
            let t = if matches!(actual, Type::Var(_)) {
                expected.cloned().unwrap_or(actual)
            } else {
                actual
            };
            f.bound_types.push(t);
        }
        f.bound.extend(args);
        let arity = match &f.code {
            Function::User { formals, .. } => formals.len(),
            Function::Core(op, ..) => operation_arity(*op, span)?,
            Function::Projection(_) => 1,
            Function::Constructor(_, arity) => *arity,
        };
        if f.bound.len() < arity {
            f.instance = out;
            return Ok(V::Fun(Rc::new(f)));
        }
        if f.bound.len() > arity {
            return Err(error(span, "too many arguments for function"));
        }
        match f.code {
            Function::User {
                formals,
                body,
                env,
                declared,
                ..
            } => {
                let mut substs = HashMap::new();
                let mut row_substs = HashMap::new();
                for (i, arg) in f.bound_types.iter().enumerate() {
                    if let Some(p) = fun_arg(&declared, i) {
                        fit_instance(p, arg, &mut substs, &mut row_substs);
                    }
                }
                if let Some(out) = out.as_ref() {
                    fit_instance(
                        result_type(&declared, arity),
                        out,
                        &mut substs,
                        &mut row_substs,
                    );
                }
                let scope = self.frame(env);
                if !substs.is_empty() || !row_substs.is_empty() {
                    self.frames[scope].substs = substs;
                    self.frames[scope].row_substs =
                        (!row_substs.is_empty()).then(|| Box::new(row_substs));
                    self.frames[scope].type_scope = Some(scope);
                }
                for (s, v) in formals.iter().zip(f.bound) {
                    self.put(scope, *s, v)
                }
                self.eval(body, scope)
            }
            Function::Core(op, ty, schema) => {
                let out = out.as_ref().unwrap_or_else(|| result_type(&ty, arity));
                if let Some(op) = witnessed_op(op) {
                    return self.witnessed_core(
                        op,
                        f.bound,
                        &f.bound_types,
                        out,
                        schema.as_ref(),
                        span,
                    );
                }
                let mode = callable_lift(op, &f.bound_types).map_err(|m| error(span, m))?;
                let instance_schema = if dynamic_schema_operation(op) {
                    self.p.instance_schema(out).or(schema)
                } else {
                    schema
                };
                self.core(op, f.bound, out, instance_schema.as_ref(), mode, span)
            }
            Function::Constructor(id, _) => self.make_variant(id, f.bound, span),
            Function::Projection(field) => {
                projection(&field, f.bound.pop().expect("checked arity"), span)
            }
        }
    }
    fn execute(&mut self, v: V, span: Span) -> R<V> {
        let V::Action(action) = v else {
            return Err(error(span, "expected a suspended action"));
        };
        if let Some(result) = operations041::apply_action(&action, self, span) {
            return result;
        }
        if let Some(result) = library05::apply_action(&action, self, span) {
            return result;
        }
        match action.as_ref() {
            Action::Library(..) => Err(invariant(span, "unknown standard library action")),
            Action::IoRead(_) | Action::IoWrite(..) | Action::IoExists(_) | Action::IoPrint(_) => {
                Err(invariant(span, "fallible I/O action has no implementation"))
            }
            Action::CsvRead(..) | Action::CsvWrite(..) => {
                Err(invariant(span, "fallible CSV action has no implementation"))
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
        let mut budget = KEY_WORK_LIMIT;
        for row in xs {
            let k = self.apply(f.clone(), vec![row.clone()], span)?;
            let key = key_with_budget(&k, &mut budget, 0).ok_or_else(|| {
                error(
                    span,
                    "group key exceeds supported hash complexity or resource limit",
                )
            })?;
            let i = *index.entry(key).or_insert(groups.len());
            if let Some(g) = groups.get_mut(i) {
                g.1.push(row.clone())
            } else {
                groups.push((k, vec![row.clone()]))
            }
        }
        Ok(groups)
    }
    fn witnessed_core(
        &mut self,
        op: WitnessedOp,
        args: Vec<V>,
        inputs: &[Type],
        output: &Type,
        schema: Option<&Rc<TableSchema>>,
        span: Span,
    ) -> R<V> {
        match op {
            WitnessedOp::Dense(core) | WitnessedOp::Indexed(core) => {
                self.tick(span)?;
                if args.len() != operation_arity(core, span)? || inputs.len() != args.len() {
                    return Err(invariant(span, "invalid checked array arity"));
                }
                if matches!(op, WitnessedOp::Dense(_)) {
                    arrays012::apply(core, &args, inputs, output, self, span)
                } else {
                    array_sequences012::apply(core, &args, inputs, self, span)
                }
            }
            WitnessedOp::Join(core) | WitnessedOp::Relation(core) => {
                if args.len() != operation_arity(core, span)? || inputs.len() != args.len() {
                    return Err(invariant(span, "invalid checked relation arity"));
                }
                if matches!(op, WitnessedOp::Join(_)) {
                    return joins06::apply(core, args, inputs, output, schema, self, span);
                }
                self.tick(span)?;
                let concrete = (core == CoreOp::TableSelect)
                    .then(|| self.p.instance_schema(output))
                    .flatten();
                relations06::apply(
                    core,
                    &args,
                    inputs,
                    concrete.as_ref().or(schema),
                    self,
                    span,
                )
            }
            WitnessedOp::Combinator(core) => {
                combinators05::apply(core, args, inputs, output, self, span)
            }
            WitnessedOp::ArrayOverload(core, index) => {
                if inputs.get(index).and_then(Type::array_element).is_some() {
                    self.tick(span)?;
                    arrays012::apply(core, &args, inputs, output, self, span)
                } else if core == CoreOp::Reduce {
                    reduce014::apply(&args, inputs, output, self, span)
                } else {
                    self.core(core, args, output, schema, None, span)
                }
            }
        }
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
        if args.len() != operation_arity(op, span)? {
            return Err(error(span, "invalid typed HIR operator arity"));
        }
        if let Some(result) = operations041::apply_core(op, &args, ty, span, self.p) {
            return result;
        }
        if let Some(result) = collections042::apply(op, &args, ty, schema, self, span) {
            return result;
        }
        if let Some(result) = library05::apply_core(op, &args, span, self.p) {
            return result;
        }
        if let Some(result) = json05::apply(op, &args, ty, span, self.p) {
            return result;
        }
        let mut args = args.into_iter();
        let mut a = || args.next().expect("arity checked");
        let fail = |message| error(span, message);
        let result = match op {
            Add | Sub | Mul | Div | Quotient | Remainder | Pow | Equal | NotEqual | Lt | Le
            | Gt | Ge | And | Or => lift(a(), a(), mode, &|x, y| {
                scalar_binary(op, x, y, inner_type(ty))
            }),
            Sqrt => lift_one(a(), mode, &|v| {
                if matches!(v, V::Wide(_)) {
                    return finite(wide05::sqrt(&wide05::ratio(&v)?)?);
                }
                let x = real_of(&v)?;
                if x < 0.0 {
                    Err("square root of a negative number")
                } else {
                    finite(x.sqrt())
                }
            }),
            IoRead | IoWrite | IoExists | IoPrint | CheckedDiv => {
                return Err(invariant(span, "checked operation has no implementation"))
            }
            Concat | Distinct | Lookup | Zip | Cartesian | Take | Drop | Slice | Reverse
            | Flatten | Transpose | Variance | Percentile | Min | Max | All | Any | Fold | Scan => {
                return Err(invariant(
                    span,
                    "checked collection operation has no implementation",
                ))
            }
            Reduce => return Err(invariant(span, "checked reduce requires operand evidence")),
            ArrayReshape | ArrayForm | ArrayShape | ArrayRavel | ArrayAt | ArrayMap | ArrayZip
            | ArrayOuter | ArrayRankMap | ArrayScan | ArrayWindows | ArraySelect | ArrayGrade
            | ArrayPermute | ArrayAxes | ArraySection | ArrayReduce | ArrayFold | ArrayScanAxis
            | ArrayDot | ArrayMatMul | ArrayDiag => {
                return Err(invariant(
                    span,
                    "checked array operation requires operand evidence",
                ));
            }
            StructuralRows => match a() {
                value @ V::Table(_, _) => return Ok(value),
                _ => return Err(invariant(span, "checked structural view expects a table")),
            },
            InnerJoin | LeftJoin | SemiJoin | AntiJoin | TableUnion | TableDistinct
            | TableSelect | DropNa | Fill | PresentOnly => {
                return Err(invariant(
                    span,
                    "checked relation requires operand evidence",
                ));
            }
            OptMap | OptBind | OptResult | ResMap | ResMapErr | ResCollect | ResPartition
            | ResOk | ResOptional | ReadLine | FsList | FsMkdir | FsBase | FsJoin | SysArgs
            | SysEnv | SysPid | StrUpper | StrLower | StrTrim | StrSplit | StrJoin | RxFind
            | DtDays | DtDate | JsonEncode | JsonDecode => {
                return Err(invariant(
                    span,
                    "checked standard library operation has no implementation",
                ));
            }
            EqualWhole => Ok(V::Bool(equal(&a(), &a()))),
            Negate | Abs | Not => lift_one(a(), mode, &|v| {
                if op == Not {
                    return Ok(V::Bool(!as_bool(&v)?));
                }
                if *inner_type(ty) == Type::Prim(Prim::R) {
                    let V::Real(n) = v else {
                        return Err("expected finite real operand");
                    };
                    return finite(if op == Abs { n.abs() } else { -n });
                }
                let k = domain(inner_type(ty))
                    .ok_or("numeric domain is unresolved at typed execution")?;
                exact_unary(op, &v, k)
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
                let d =
                    domain(ty).ok_or_else(|| error(span, "range has no checked numeric domain"))?;
                range(&a(), &a(), d)
            }
            Sum | Product => {
                let xs = a();
                let xs = as_seq(&xs).map_err(fail)?;
                numeric_reduce(op, xs.iter().cloned(), ty)
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
                let (rows, input) = collection(source, span)?;
                let (Some(TableSchema::Named(source_fields)), TableSchema::Named(output_fields)) =
                    (input.as_deref(), schema.as_ref())
                else {
                    return Err(invariant(
                        span,
                        "df.extend needs checked named table layouts",
                    ));
                };
                let schema = if source_fields
                    .iter()
                    .zip(output_fields)
                    .all(|(a, b)| a.0 == b.0)
                {
                    schema
                } else {
                    // Column type evidence remains the checked output layout.
                    // Existing labels follow the checked source table's order.
                    let position = source_fields
                        .iter()
                        .enumerate()
                        .map(|(i, (name, _))| (name, i))
                        .collect::<HashMap<_, _>>();
                    let mut fields = output_fields.clone();
                    fields
                        .sort_by_key(|(name, _)| position.get(name).copied().unwrap_or(usize::MAX));
                    Rc::new(TableSchema::Named(fields))
                };
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
            Quantize => {
                let scale = exact_index(&a()).map_err(fail)?;
                decimal05::quantize(&a(), scale)
            }
            MathFloor | MathCeil | MathTrunc | MathRound | MathSign | MathFrac | MathRecip
            | MathExp | MathExpm1 | MathLn | MathLog10 | MathLog1p | MathSin | MathCos
            | MathTan | MathAsin | MathAcos | MathAtan | MathSinh | MathCosh | MathTanh
            | MathAsinh | MathAcosh | MathAtanh | MathRad | MathDeg | MathLog | MathPow
            | MathHypot | MathAtan2 | MathRoot | MathPi | MathE | MathTau | MathGcd | MathLcm
            | MathFact | MathChoose | MathPerm | MathModPow | MathIsqrt | MathPrime
            | MathFactor | MathDivisors | MathFib | MathGamma | MathLgamma | MathErf | MathErfc
            | MathBeta | PolyEval | PolyAdd | PolySub | PolyMul | PolyDeriv | PolyInteg
            | PolyDegree | NumBisect | NumNewton | NumIntegrate | NumInterp | NumDiff => {
                return math::apply(op, args.as_slice(), ty, mode, self, span)
            }
            ToNatural | ToInteger | ToRational | ToDecimal | ToReal => lift_one(a(), mode, &|v| {
                if op == ToReal {
                    return finite(real_of(&v)?);
                }
                let v = to_exact(&v)?;
                let d = match op {
                    ToNatural => Domain::Natural,
                    ToInteger => Domain::Integer,
                    ToDecimal => Domain::Decimal,
                    _ => Domain::Rational,
                };
                if d == Domain::Decimal {
                    return decimal05::decimal_from_exact(&v);
                }
                if let V::Exact(n) = v {
                    return Ok(V::Exact(n.as_domain(d)?));
                }
                wide05::ratio_value(wide05::ratio(&v)?, d)
            }),
        };
        result.map_err(fail)
    }
}

fn wide_standard_deviation(xs: &[V]) -> NumericResult<V> {
    let q = Domain::Rational;
    let origin = xs.first().ok_or("expected nonempty sequence")?;
    let count = exact_i128(xs.len() as i128, q);
    let mut sum = exact_i128(0, q);
    for x in xs {
        let x = exact_binary(CoreOp::Sub, x, origin, q)?;
        sum = exact_binary(CoreOp::Add, &sum, &x, q)?;
    }
    let mean = exact_binary(CoreOp::Div, &sum, &count, q)?;
    let mut ss = exact_i128(0, q);
    for x in xs {
        let x = exact_binary(CoreOp::Sub, x, origin, q)?;
        let x = exact_binary(CoreOp::Sub, &x, &mean, q)?;
        let square = exact_binary(CoreOp::Mul, &x, &x, q)?;
        ss = exact_binary(CoreOp::Add, &ss, &square, q)?;
    }
    let variance = exact_binary(CoreOp::Div, &ss, &count, q)?;
    finite(wide05::sqrt(&wide05::ratio(&variance)?)?)
}

fn stats(op: CoreOp, v: V, span: Span) -> R<V> {
    let fail = |m| error(span, m);
    let xs = as_seq(&v).map_err(fail)?;
    if xs.is_empty() {
        return Ok(V::Optional(None));
    }
    let value = if op == CoreOp::StdDev && xs.iter().any(|v| matches!(v, V::Wide(_))) {
        wide_standard_deviation(xs).map_err(fail)?
    } else if op == CoreOp::StdDev {
        let mut mean = 0.0;
        let mut ss = 0.0;
        for (i, x) in xs.iter().enumerate() {
            let x = real_of(x).map_err(fail)?;
            let d = x - mean;
            mean += d / (i + 1) as f64;
            ss += d * (x - mean);
        }
        if !ss.is_finite() {
            return Err(error(span, "real standard deviation overflow"));
        }
        finite((ss.max(0.0) / xs.len() as f64).sqrt()).map_err(fail)?
    } else {
        let (sum, div) = if op == CoreOp::Median {
            let mut xs = xs.to_vec();
            if xs.iter().any(|v| wide05::kind(v).is_none()) {
                return Err(error(span, "med expects exact numeric values"));
            }
            xs.sort_by(|a, b| order(a, b).unwrap_or(Ordering::Equal));
            let mid = xs.len() / 2;
            if xs.len() % 2 == 1 {
                (xs[mid].clone(), 1)
            } else {
                (
                    exact_binary(CoreOp::Add, &xs[mid - 1], &xs[mid], Domain::Rational)
                        .map_err(fail)?,
                    2,
                )
            }
        } else {
            let mut sum = exact_i128(0, Domain::Rational);
            for v in xs {
                sum = exact_binary(CoreOp::Add, &sum, v, Domain::Rational).map_err(fail)?;
            }
            (sum, xs.len() as i128)
        };
        exact_binary(
            CoreOp::Div,
            &sum,
            &exact_i128(div, Domain::Natural),
            Domain::Rational,
        )
        .map_err(fail)?
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
        Type::Prim(Prim::N | Prim::Z) => parse_exact_value(s, domain(ty).expect("known domain"))?,
        Type::Prim(Prim::Q) => {
            if let Some((a, b)) = s.split_once('/') {
                // Use the fast bounded parser when possible.
                match (a.parse::<i128>(), b.parse::<i128>()) {
                    (Ok(a), Ok(b)) => match Exact::new(a, b, Domain::Rational) {
                        Ok(v) => V::Exact(v),
                        Err(e) if e == OVERFLOW => rational(BigInt::from(a), BigInt::from(b))?,
                        Err(e) => return Err(e),
                    },
                    _ => rational(wide05::parse_int(a)?, wide05::parse_int(b)?)?,
                }
            } else {
                if s.is_empty()
                    || s.bytes()
                        .any(|b| !matches!(b, b'0'..=b'9' | b'-' | b'+' | b'.'))
                {
                    return Err("expected a decimal or rational");
                }
                parse_exact_value(s, Domain::Rational)?
            }
        }
        Type::Prim(Prim::D) => decimal05::decimal_literal(s)?,
        Type::Prim(Prim::R) => finite(s.parse().map_err(|_| "expected a finite real")?)?,
        _ => return Err("CSV schema contains an unsupported column type"),
    })
}
fn write_cell(v: &V) -> NumericResult<String> {
    Ok(match v {
        V::String(s) => s.to_string(),
        V::Char(c) => c.to_string(),
        V::Bool(b) => b.to_string(),
        V::Exact(_) | V::Wide(_) | V::Real(_) => repr(v),
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
fn evaluate(
    program: &HirProgram,
    input: &mut dyn RuntimeInput,
    stdin: &mut dyn BufRead,
    args: &[String],
    output: &mut dyn Write,
) -> R<V> {
    let mut eval = Eval {
        p: program,
        frames: vec![Frame::default()],
        input,
        stdin,
        args,
        output,
        fuel: 1_000_000,
        depth: 0,
        effects: 0,
    };
    for group in &program.groups {
        for d in group {
            if !d.formals.is_empty() {
                eval.define_fun(d, 0);
            }
        }
        for d in group {
            if d.formals.is_empty() {
                let v = eval.eval(d.body, 0)?;
                eval.put(0, d.symbol, v);
            }
        }
    }
    if let Some(id) = program.entry {
        let v = eval.eval(id, 0)?;
        eval.execute(v, program.node(id).span)
    } else {
        Ok(V::Unit)
    }
}

/// Execute using injected files and output; line input is EOF and arguments are empty.
pub fn run_with_io(
    program: &HirProgram,
    input: &mut dyn RuntimeInput,
    sink: &mut dyn Write,
) -> Result<Output, Error> {
    run_with_host(program, input, &mut std::io::empty(), &[], sink)
}

/// Execute with explicit host file input, line input, script arguments, and output.
pub fn run_with_host(
    program: &HirProgram,
    input: &mut dyn RuntimeInput,
    stdin: &mut dyn BufRead,
    args: &[String],
    sink: &mut dyn Write,
) -> Result<Output, Error> {
    crate::hir::validate(program)?;
    run_with_host_prevalidated(program, input, stdin, args, sink)
}

pub(crate) fn run_with_host_prevalidated(
    program: &HirProgram,
    input: &mut dyn RuntimeInput,
    stdin: &mut dyn BufRead,
    args: &[String],
    sink: &mut dyn Write,
) -> Result<Output, Error> {
    let mut value = evaluate(program, input, stdin, args, sink).map_err(Control::into_error)?;
    let span = program
        .entry
        .map(|id| program.node(id).span)
        .unwrap_or_default();
    if let V::Outcome(v) = value {
        value = match v {
            Ok(v) => v.as_ref().clone(),
            Err(f) => return Err(uncaught(program, &f)),
        };
    }
    let status = match value {
        V::Unit => 0,
        V::Wide(_) => wide05::integral(&value)
            .ok()
            .and_then(|n| n.to_u8())
            .ok_or_else(|| error(span, "entry exit status must be from 0 to 255").into_error())?,
        V::Exact(n) if n.d == 1 => u8::try_from(n.n)
            .map_err(|_| error(span, "entry exit status must be from 0 to 255").into_error())?,
        _ => {
            return Err(
                invariant(span, "entry must return unit or a natural exit status").into_error(),
            )
        }
    };
    Ok(Output {
        stdout: String::new(),
        status,
    })
}
pub fn run_with_output(program: &HirProgram, sink: &mut dyn Write) -> Result<Output, Error> {
    run_with_io(program, &mut FsInput, sink)
}

pub fn run(program: &HirProgram) -> Result<Output, Error> {
    crate::hir::validate(program)?;
    run_prevalidated(program)
}

pub(crate) fn run_prevalidated(program: &HirProgram) -> Result<Output, Error> {
    let mut sink = Vec::new();
    let mut out =
        run_with_host_prevalidated(program, &mut FsInput, &mut std::io::empty(), &[], &mut sink)?;
    out.stdout = String::from_utf8(sink)
        .map_err(|_| invariant(Span::default(), "output sink was not UTF-8").into_error())?;
    Ok(out)
}

/// Inspect an entry with EOF line input and no script arguments.
pub fn view_with_io(
    program: &HirProgram,
    input: &mut dyn RuntimeInput,
    sink: &mut dyn Write,
) -> Result<crate::eval::ReplOutput, Error> {
    view_with_host(program, input, &mut std::io::empty(), &[], sink)
}

/// Inspect an entry using explicit host streams and script arguments.
pub fn view_with_host(
    program: &HirProgram,
    input: &mut dyn RuntimeInput,
    stdin: &mut dyn BufRead,
    args: &[String],
    sink: &mut dyn Write,
) -> Result<crate::eval::ReplOutput, Error> {
    crate::hir::validate(program)?;
    view_with_host_prevalidated(program, input, stdin, args, sink)
}

pub(crate) fn view_with_host_prevalidated(
    program: &HirProgram,
    input: &mut dyn RuntimeInput,
    stdin: &mut dyn BufRead,
    args: &[String],
    sink: &mut dyn Write,
) -> Result<crate::eval::ReplOutput, Error> {
    let value = evaluate(program, input, stdin, args, sink).map_err(Control::into_error)?;
    Ok(crate::eval::ReplOutput {
        value: inspect(&value),
        stdout: String::new(),
    })
}

pub fn view_with_output(
    program: &HirProgram,
    sink: &mut dyn Write,
) -> Result<crate::eval::ReplOutput, Error> {
    view_with_io(program, &mut FsInput, sink)
}

pub fn view_entry(program: &HirProgram) -> Result<crate::eval::ReplOutput, Error> {
    crate::hir::validate(program)?;
    view_entry_prevalidated(program)
}

pub(crate) fn view_entry_prevalidated(
    program: &HirProgram,
) -> Result<crate::eval::ReplOutput, Error> {
    let mut sink = Vec::new();
    let mut out =
        view_with_host_prevalidated(program, &mut FsInput, &mut std::io::empty(), &[], &mut sink)?;
    out.stdout = String::from_utf8(sink)
        .map_err(|_| invariant(Span::default(), "output sink was not UTF-8").into_error())?;
    Ok(out)
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
            << "io"; emit(x) := !{ ^= ! io.print x { _ : (); } };
            id(x) := x;
            f := |{ 4; \y -> + $ y };
            !{
                ! emit id 9;
                ! emit id "polymorphic";
                ! emit @ f 5;
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
            << "io"; emit(x) := !{ ^= ! io.print x { _ : (); } };
            twice(x:z):z := + x x;
            !{
                x := |{ ! !{ ! emit "once"; 4 }; + $ $; @ &twice $ };
                ! emit x;
            };
        "#,
        );
        assert_eq!(run(&ir).unwrap().stdout, "once\n16\n");
    }

    #[test]
    fn typed_hir_runtime_domain_error_and_lazy_branches() {
        let good = compile(
            r#" << "io"; emit(x) := !{ ^= ! io.print x { _ : (); } }; !{ ! emit ? 1b (/ 3 2) (/ 1 0); }; "#,
        );
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
            << "io"; emit(x) := !{ ^= ! io.print x { _ : (); } };
            amount(x) := .amount x;
            !{
                ! emit amount .{ amount:=12; label:="kept"; };
                ! emit amount .{ amount:=9; approved:=1b; };
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
            << "io"; emit(x) := !{ ^= ! io.print x { _ : (); } };
            x := + 1 2;
            y := - [4 6] 1;
            z := - 9 [1 2];
            w := * [1 2] [2 3];
            !{ ! emit (x,y,z,w); };
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
    #[test]
    fn checked_instances_cover_numeric_closures_and_indirect_broadcast() {
        let ir = compile(
            r#"
            << "io"; emit(x) := !{ ^= ! io.print x { _ : (); } };
            twice(x) := + x x;
            through(f,x) := @ f x;
            twice_with(f,x) := @ (@ f x) x;
            plus := &+;
            factory(x) := \y -> + x y;
            add:[z] z->[z] := &+;
            scale:z [z]->[z] := &*;
            pair:[z] [z]->[z] := &-;
            f := factory 0.5q;
            !{
                ! emit twice 4z;
                ! emit twice 0.25q;
                ! emit twice 0.25r;
                ! emit through &twice 0.25r;
                ! emit twice_with plus 0.25q;
                ! emit @ f 0.25q;
                ! emit @@ add ([1 2],4);
                ! emit @ (@ scale 2) [4 5];
                ! emit @@ pair ([9 8],[3 2]);
            };
        "#,
        );
        assert_eq!(
            run(&ir).unwrap().stdout,
            "8\n1/2\n0.5\n0.5\n1/2\n3/4\n[5 6]\n[8 10]\n[6 6]\n"
        );
        assert!(ir.arena.iter().any(|n|matches!(&n.callee_type,Some(Type::Fun(arg,out)) if **arg==Type::Prim(Prim::R)&&**out==Type::Prim(Prim::R))));
    }

    #[test]
    fn indirect_pairwise_preserves_the_shape_check() {
        let ir = compile("add:[z] [z]->[z] := &+; fail := @@ add ([1 2],[3]); !{ (); };");
        let err = run(&ir).err().unwrap();
        assert!(err.message.contains("shape mismatch"));
    }

    #[test]
    fn generic_natural_instance_preserves_underflow() {
        let ir = compile("sub(x,y) := - x y; fail := sub 1n 2n; !{ (); };");
        assert!(run(&ir)
            .err()
            .unwrap()
            .message
            .contains("natural subtraction underflow"));
    }
    #[test]
    fn failure_provenance_is_not_part_of_result_equality() {
        let ir = compile(
            r#"
            first(x:z):z^z := ^{ ^: x };
            second(x:z):z^z := ^{ ^? first x };
            !{ == (first 3z) (second 3z) };
        "#,
        );
        assert_eq!(view_entry(&ir).unwrap().value, "1b");
    }

    #[test]
    fn optional_tuple_keys_and_duplicate_join_rows_are_stable() {
        let ir = compile(
            r#"
            l:#(?s,z) := #[(?0,1) (?1 "x",2) (?0,3)];
            r:#(?s,z) := #[(?0,4) (?0,5) (?1 "y",6)];
            key(row:(?s,z)) := (.1 row,0z);
            j := *= &key &key l r;
            wanted:#((?s,z),(?s,z)) := #[
                ((?0,1),(?0,4)) ((?0,1),(?0,5))
                ((?0,3),(?0,4)) ((?0,3),(?0,5))
            ];
            !{ == j wanted };
        "#,
        );
        assert_eq!(view_entry(&ir).unwrap().value, "1b");
    }

    #[test]
    fn real_signed_zeros_share_one_join_key() {
        let ir = compile(
            r#"
            left:#(r,z) := #[(-0.0r,4z)];
            right:#(r,z) := #[(0.0r,5z)];
            !{ # (*= &.1 &.1 left right) };
        "#,
        );
        assert_eq!(view_entry(&ir).unwrap().value, "1");
    }

    #[test]
    fn empty_join_keeps_instantiated_paired_layout() {
        let ir = compile(
            r#"
            left:#(z,s) := #[];
            right:#(z,b) := #[(1,1b)];
            lk(row:(z,s)) := .1 row;
            rk(row:(z,b)) := .1 row;
            outer := &*?;
            !{ @@ outer (&lk,&rk,left,right) };
        "#,
        );
        let out = view_entry(&ir).unwrap().value;
        assert!(out.contains(".1 | .2"), "{out}");
        assert_eq!(out.lines().count(), 2, "{out}");
    }

    #[test]
    fn tacit_lowering_equivalents_keep_polymorphism_and_numeric_evidence() {
        let ir = compile(
            r#"
            square := \x -> @@ &* (x,x);
            square_field := \x -> @@ &* (@ &.value x,@ &.value x);
            field := \x -> @ &.value (@ &.inner x);
            vector_square:[n]->[n] := \x -> @@ &* (x,x);
            curried := \x -> \y -> @@ &+ (x,y);
            !{
                (@ square 3n, @ square 0.5q,
                 @ square_field .{ value:=3n; text:="ignored"; },
                 @ square_field .{ value:=0.5q; tag:=1b; },
                 @ field .{ inner:=.{value:="text"; extra:=1n;}; },
                 @ vector_square [2n 3n],
                 @@ curried (2n,3n), @@ curried (0.25q,0.5q))
            };
        "#,
        );
        assert_eq!(
            view_entry(&ir).unwrap().value,
            "(9, 1/4, 9, 1/4, \"text\", [4 9], 5, 3/4)"
        );
    }

    #[test]
    fn tacit_fork_lowering_evaluates_the_left_branch_first() {
        let ir = compile(
            r#"
            left(x:n):n := - x 10n;
            right(x:n):n := // x 0n;
            both := \x -> @@ &+ (@ &left x,@ &right x);
            !{ @ both 1n };
        "#,
        );
        let err = view_entry(&ir).err().unwrap();
        assert!(
            err.message.contains("natural subtraction underflow"),
            "{}",
            err.message
        );
    }

    #[test]
    fn result_combinators_keep_first_failure_and_failure_provenance() {
        let source = r#"
            << "res";
            first(x:z):z^z := ^{ ^: x };
            second(x:z):z^z := ^{ ^? first x };
            shifted := res.maperr (\x -> + x 100z) res.collect [second 5z first 2z];
            !{ ^{ ^? shifted; () } };
        "#;
        let ir = compile(source);
        let err = run(&ir).err().unwrap();
        assert!(err.message.contains("105"), "{}", err.message);
        let diagnostic = err.structured().unwrap();
        assert_eq!(
            &source[diagnostic.primary.start..diagnostic.primary.end],
            "^: x"
        );
        assert!(diagnostic
            .related
            .iter()
            .any(|r| r.label.contains("propagat")));
    }

    #[test]
    fn option_combinators_use_checked_callback_instances_and_skip_missing_payload() {
        let ir = compile(
            r#"
            << "opt";
            choose := &opt.map;
            inc(x) := + x 1n;
            bad(x:n):n := - x 99n;
            none:?n := ?0;
            !{ (@@ choose (&inc,?1 5n), opt.map &bad none) };
        "#,
        );
        assert_eq!(view_entry(&ir).unwrap().value, "(?1 6, ?0)");
    }

    #[test]
    fn recover_selects_one_arm_once_and_wildcard_payload_slots_do_not_bind() {
        let ir = compile(
            r#"
            trouble :: { | other; | both z z; };
            raise_two(x:z):z^trouble := ^{ ^: both 9z x };
            !{ ^= raise_two 3z { other : // 2z 0z; both _ y : y; } };
        "#,
        );
        assert_eq!(view_entry(&ir).unwrap().value, "3");
    }

    #[test]
    fn uncaught_result_contains_original_span_and_each_propagation() {
        let source = "first(x:z):z^z := ^{ ^: x };
second(x:z):z^z := ^{ ^? first x };
!{ ^{ lost := ^? second 5z; () } };";
        let ir = compile(source);
        let err = match run(&ir) {
            Ok(_) => panic!("failure became successful"),
            Err(e) => e,
        };
        let d = err.structured().unwrap();
        assert_eq!(d.category, Category::RuntimeOperational);
        assert!(d.message.contains("z: 5"), "{}", d.message);
        assert_eq!(d.primary.start, source.find("^:").unwrap());
        assert_eq!(
            d.related
                .iter()
                .map(|note| note.span.start)
                .collect::<Vec<_>>(),
            source
                .match_indices("^?")
                .map(|(i, _)| i)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn runtime_result_instantiation_is_polymorphic() {
        let ir = compile(
            r#"
            keep(x:~a):~a^z := ^{ x };
            get(fallback:~a,wrapped:~a^z):~a := ^= wrapped { _ : fallback; };
            !{ (get 1z (keep 8z), get "stale" (keep "fresh")) };
        "#,
        );
        assert_eq!(view_entry(&ir).unwrap().value, "(8, \"fresh\")");
    }
    #[test]
    fn injectable_reader_drives_action_and_does_not_reexecute_it() {
        struct Host {
            reads: Vec<String>,
        }
        impl RuntimeInput for Host {
            fn open(&mut self, path: &str) -> std::io::Result<OpenedInput> {
                self.reads.push(path.into());
                let bytes = b"injected data";
                Ok(OpenedInput {
                    reader: Box::new(std::io::Cursor::new(bytes)),
                    byte_len: Some(bytes.len() as u64),
                })
            }
        }
        let ir = compile(
            r#"
            << "io";
            !{ ^{
                data := ^? ! io.read "virtual-fixture";
                ^? ! io.print data;
                ()
            } };
        "#,
        );
        let mut host = Host { reads: vec![] };
        let mut output = Vec::new();
        let status = run_with_io(&ir, &mut host, &mut output).unwrap();
        assert_eq!(status.status, 0);
        assert_eq!(status.stdout, "");
        assert_eq!(host.reads, ["virtual-fixture"]);
        assert_eq!(output, b"injected data\n");
    }
    #[test]
    fn registry_collection_calls_keep_checked_higher_order_instances() {
        let ir = compile(
            r#"
            << "io";
            emit(x) := !{ ^= ! io.print x { _ : (); } };
            fold := /: &+ 0z [1z 2z 3z];
            scan := \: &+ 0z [1z 2z 3z];
            concat:[z] [z]->[z] := &+:;
            lookup:n [z]->?z := &.:;
            varof:[q]->?q := &var;
            sqrt:z->r := &^/;
            !{
                ! emit (fold, scan);
                ! emit @ (@ concat [1z]) [2z];
                ! emit @@ lookup (2n, [1z 2z]);
                ! emit @ varof [1q 2q 3q];
                ! emit @ sqrt 9z;
            };
        "#,
        );
        assert_eq!(
            run(&ir).unwrap().stdout,
            "(6, [0 1 3 6])\n[1 2]\n?1 2\n?1 2/3\n3\n"
        );
    }
    #[test]
    fn exact_small_values_stay_inline_and_wide_results_demote() {
        #[cfg(target_pointer_width = "64")]
        assert!(std::mem::size_of::<V>() <= 48);
        let big = exact_binary(
            CoreOp::Add,
            &exact_i128(i128::MAX, Domain::Integer),
            &exact_i128(1, Domain::Integer),
            Domain::Integer,
        )
        .unwrap();
        assert!(matches!(big, V::Wide(_)));
        let small = exact_binary(
            CoreOp::Sub,
            &big,
            &exact_i128(1, Domain::Integer),
            Domain::Integer,
        )
        .unwrap();
        assert!(matches!(small,V::Exact(n) if n.n==i128::MAX));
        let odd = wide05::ratio_value(
            BigRational::from_integer((BigInt::from(1u8) << 160usize) + 1u8),
            Domain::Natural,
        )
        .unwrap();
        let ans = exact_binary(
            CoreOp::Pow,
            &exact_i128(-1, Domain::Integer),
            &odd,
            Domain::Integer,
        )
        .unwrap();
        assert!(matches!(ans,V::Exact(n) if n.n == -1));
    }

    #[test]
    fn a_wide_range_obeys_a_distinct_aggregate_resource_limit() {
        let lo = BigInt::from(1u8) << 700_000usize;
        let hi = &lo + 999u16;
        let lo = wide05::ratio_value(BigRational::from_integer(lo), Domain::Natural).unwrap();
        let hi = wide05::ratio_value(BigRational::from_integer(hi), Domain::Natural).unwrap();
        assert_eq!(
            range(&lo, &hi, Domain::Natural).err(),
            Some("wide range resource limit exceeded (64 MiB of integer data)")
        );
    }
    #[test]
    fn wide_sqrt_need_not_first_round_the_radicand() {
        let enormous = BigRational::from_integer(BigInt::from(10u8).pow(400));
        assert!(wide05::to_real(&enormous).is_err());
        assert_eq!(wide05::sqrt(&enormous).unwrap(), 1e200);
        let tiniest = wide05::from_real(f64::from_bits(1)).unwrap();
        assert_eq!(wide05::sqrt(&tiniest).unwrap(), f64::from_bits(1).sqrt());
    }
    #[test]
    fn exact_wide_sd_does_not_lose_the_small_variance_of_a_large_offset() {
        let base = BigInt::from(1u8) << 2048usize;
        let near = &base + 2u8;
        let xs = [
            wide05::ratio_value(BigRational::from_integer(base), Domain::Integer).unwrap(),
            wide05::ratio_value(BigRational::from_integer(near), Domain::Integer).unwrap(),
        ];
        assert!(matches!(wide_standard_deviation(&xs).unwrap(),V::Real(r) if r==1.0));
    }
}
