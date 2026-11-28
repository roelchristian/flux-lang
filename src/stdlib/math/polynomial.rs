//! Dense polynomials in ascending power order. Arithmetic uses the checked
//! scalar result domain; zero tails are never retained in polynomial outputs.
use super::*;

const POLY_LIMIT: &str = "polynomial resource limit exceeded";
const MAX_PAIRS: usize = 1_000_000;
const MAX_WORK: u64 = 4_000_000;
const MAX_OUTPUT_BITS: u64 = 64 * 1024 * 1024 * 8;

fn payload_bits(value: &V) -> u64 {
    match value {
        V::Wide(w) => match w.as_ref() {
            Wide::Exact { ratio, .. } => ratio.numer().bits() + ratio.denom().bits(),
            Wide::Decimal(d) => d.storage_bits(),
        },
        _ => 0,
    }
}

fn is_zero(value: &V) -> bool {
    match value {
        V::Exact(n) => n.n == 0,
        V::Real(r) => *r == 0.0,
        V::Wide(w) => match w.as_ref() {
            Wide::Exact { ratio, .. } => ratio.is_zero(),
            Wide::Decimal(d) => d.storage_bits() == 0,
        },
        _ => false,
    }
}

fn degree_len(coefficients: &[V]) -> usize {
    coefficients
        .iter()
        .rposition(|v| !is_zero(v))
        .map_or(0, |i| i + 1)
}

fn checked_coefficients(v: &V) -> NumericResult<&[V]> {
    let coefficients = as_seq(v)?;
    if coefficients.len() > resources::limits().max_elements {
        return Err(POLY_LIMIT);
    }
    Ok(&coefficients[..degree_len(coefficients)])
}

struct Work(u64);
impl Work {
    fn new() -> Self {
        Self(MAX_WORK)
    }

    fn scalar(&mut self, op: CoreOp, a: &V, b: &V, ty: &Type) -> NumericResult<V> {
        let words = |value| payload_bits(value).div_ceil(64).max(1);
        // Conservative cross-product charge for exact rational operations,
        // also bounding arbitrary-precision Horner and coefficient updates.
        let charge = words(a).saturating_mul(words(b));
        self.0 = self.0.checked_sub(charge).ok_or(POLY_LIMIT)?;
        scalar_binary(op, a.clone(), b.clone(), ty)
    }
}

fn replace_charge(total: &mut u64, a: &V, b: &V) -> NumericResult<()> {
    *total += payload_bits(b);
    *total -= payload_bits(a);
    if *total > MAX_OUTPUT_BITS {
        Err(POLY_LIMIT)
    } else {
        Ok(())
    }
}

fn collect_coefficients(
    items: impl IntoIterator<Item = NumericResult<V>>,
    len: usize,
    zero: &V,
) -> NumericResult<V> {
    if len > resources::limits().max_elements {
        return Err(POLY_LIMIT);
    }
    let mut output = Vec::with_capacity(len);
    let mut total = 0;
    for value in items {
        let value = value?;
        replace_charge(&mut total, zero, &value)?;
        output.push(value);
    }
    Ok(normalize(output))
}

fn normalize(mut coefficients: Vec<V>) -> V {
    coefficients.truncate(degree_len(&coefficients));
    V::Sequence(coefficients.into())
}

fn eval(coefficients: &[V], at: &V, ty: &Type) -> NumericResult<V> {
    let Some((last, earlier)) = coefficients.split_last() else {
        return numeric_identity(ty, false);
    };
    let mut work = Work::new();
    earlier.iter().rev().try_fold(last.clone(), |acc, coeff| {
        let product = work.scalar(CoreOp::Mul, &acc, at, ty)?;
        work.scalar(CoreOp::Add, &product, coeff, ty)
    })
}

fn combine(op: CoreOp, lhs: &[V], rhs: &[V], ty: &Type) -> NumericResult<V> {
    let len = lhs.len().max(rhs.len());
    let zero = numeric_identity(ty, false)?;
    let mut work = Work::new();
    collect_coefficients(
        (0..len).map(|i| {
            work.scalar(
                op,
                lhs.get(i).unwrap_or(&zero),
                rhs.get(i).unwrap_or(&zero),
                ty,
            )
        }),
        len,
        &zero,
    )
}

fn convolve(lhs: &[V], rhs: &[V], ty: &Type) -> NumericResult<V> {
    if lhs.is_empty() || rhs.is_empty() {
        return Ok(V::Sequence(Rc::from([])));
    }
    let len = lhs.len() + rhs.len() - 1;
    if len > resources::limits().max_elements || lhs.len().saturating_mul(rhs.len()) > MAX_PAIRS {
        return Err(POLY_LIMIT);
    }
    let mut work = Work::new();
    let mut output = vec![numeric_identity(ty, false)?; len];
    let mut total = 0;
    for (i, a) in lhs.iter().enumerate() {
        for (j, b) in rhs.iter().enumerate() {
            let p = work.scalar(CoreOp::Mul, a, b, ty)?;
            let dst = &mut output[i + j];
            let value = work.scalar(CoreOp::Add, dst, &p, ty)?;
            replace_charge(&mut total, dst, &value)?;
            *dst = value;
        }
    }
    Ok(normalize(output))
}

fn calculus(op: CoreOp, coefficients: &[V], ty: &Type) -> NumericResult<V> {
    let zero = numeric_identity(ty, false)?;
    let real = matches!(ty, Type::Prim(Prim::R));
    let index = |i: usize| {
        if real {
            V::Real(i as f64)
        } else {
            exact_i128(i as i128, Domain::Natural)
        }
    };
    let mut work = Work::new();
    if op == CoreOp::PolyDeriv {
        let len = coefficients.len().saturating_sub(1);
        collect_coefficients(
            coefficients
                .iter()
                .enumerate()
                .skip(1)
                .map(|(i, c)| work.scalar(CoreOp::Mul, c, &index(i), ty)),
            len,
            &zero,
        )
    } else if coefficients.is_empty() {
        Ok(V::Sequence(Rc::from([])))
    } else {
        collect_coefficients(
            std::iter::once(Ok(zero.clone())).chain(
                coefficients
                    .iter()
                    .enumerate()
                    .map(|(i, c)| work.scalar(CoreOp::Div, c, &index(i + 1), ty)),
            ),
            coefficients.len() + 1,
            &zero,
        )
    }
}

pub(super) fn apply(op: CoreOp, args: &[V], ty: &Type) -> NumericResult<V> {
    use CoreOp::*;
    match (op, args) {
        (PolyEval, [poly, at]) => eval(checked_coefficients(poly)?, at, ty),
        (PolyDegree, [poly]) => {
            let len = checked_coefficients(poly)?.len();
            Ok(V::Optional(len.checked_sub(1).map(|degree| {
                Rc::new(exact_i128(degree as i128, Domain::Natural))
            })))
        }
        (PolyAdd | PolySub | PolyMul | PolyDeriv | PolyInteg, [first, rest @ ..]) => {
            let Type::Seq(elem) = ty else {
                return Err("checked polynomial output is not a coefficient sequence");
            };
            let first = checked_coefficients(first)?;
            match (op, rest) {
                (PolyAdd, [second]) => combine(Add, first, checked_coefficients(second)?, elem),
                (PolySub, [second]) => combine(Sub, first, checked_coefficients(second)?, elem),
                (PolyMul, [second]) => convolve(first, checked_coefficients(second)?, elem),
                (PolyDeriv | PolyInteg, []) => calculus(op, first, elem),
                _ => Err("invalid checked polynomial operands"),
            }
        }
        _ => Err("invalid checked polynomial operation"),
    }
}
