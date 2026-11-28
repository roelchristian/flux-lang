//! Partial-pivot Gaussian elimination, shared by real rank and square LU.
use super::{
    bounded, charge, max_abs, negligible, relative_epsilon, Buffer, Ctx, Fault, Matrix, Result, Rhs,
};
use crate::hir_eval::math::linear::{natural, record};
use crate::hir_eval::{resources::MemoryReservation, V};

/// Eliminate in place, storing multipliers below each pivot for square LU.
/// Ties keep the first row. A rank scan skips negligible columns.
fn eliminate(
    a: &mut [f64],
    m: usize,
    n: usize,
    tol: f64,
    mut perm: Option<&mut [usize]>,
) -> Result<(usize, bool)> {
    let scale = max_abs(a);
    let (mut rank, mut odd) = (0, false);
    for col in 0..n {
        if rank == m {
            break;
        }
        let mut pivot = rank;
        for row in rank + 1..m {
            if a[row * n + col].abs() > a[pivot * n + col].abs() {
                pivot = row;
            }
        }
        if negligible(a[pivot * n + col], scale, tol) {
            if perm.is_some() {
                return Ok((rank, odd)); // not an ordinary nonsingular LU
            }
            continue;
        }
        if pivot != rank {
            for j in 0..n {
                a.swap(rank * n + j, pivot * n + j);
            }
            if let Some(rows) = perm.as_deref_mut() {
                rows.swap(rank, pivot);
            }
            odd = !odd;
        }
        let pivot = a[rank * n + col];
        for i in rank + 1..m {
            let factor = a[i * n + col] / pivot;
            a[i * n + col] = factor;
            for j in col + 1..n {
                let diagonal = a[rank * n + j];
                let cell = &mut a[i * n + j];
                // For the determinant's zero-only classification, avoid
                // amplifying a rounded quotient with FMA in simple exact
                // cancellations (for example, 1 - (1/3)*3).
                *cell = bounded(if tol == 0.0 {
                    *cell - factor * diagonal
                } else {
                    (-factor).mul_add(diagonal, *cell)
                })?;
            }
        }
        rank += 1;
    }
    Ok((rank, odd))
}

struct LowerUpper {
    factors: Buffer,
    permutation: Vec<usize>,
    _permutation_lease: MemoryReservation,
    odd_swaps: bool,
    rank: usize,
}

impl LowerUpper {
    fn factor(a: &Matrix<'_>, n: usize, tol: f64, ctx: &Ctx<'_, '_>) -> Result<Self> {
        let mut factors = Buffer::matrix(ctx, a)?;
        let (mut permutation, lease) = ctx.zeros(n, 0usize)?;
        for (i, row) in permutation.iter_mut().enumerate() {
            *row = i;
        }
        let (rank, odd_swaps) = eliminate(&mut factors.cells, n, n, tol, Some(&mut permutation))?;
        Ok(Self {
            factors,
            permutation,
            _permutation_lease: lease,
            odd_swaps,
            rank,
        })
    }

    fn nonsingular(&self) -> Result<()> {
        if self.rank == self.permutation.len() {
            Ok(())
        } else {
            Err(Fault::Singular("partial-pivot LU found a singular matrix"))
        }
    }
}

/// Multiply pivots without intermediate overflow or underflow.
fn pivot_product(pivots: impl IntoIterator<Item = f64>, mut negative: bool) -> Result<f64> {
    const FRAC: u64 = (1u64 << 52) - 1;
    const UNIT: u64 = 1023u64 << 52;
    const NORMALIZE_SUBNORMAL: f64 = (1u64 << 54) as f64;
    let (mut mantissa, mut exponent) = (1.0_f64, 0_i64);
    for pivot in pivots {
        if pivot == 0.0 {
            return Ok(0.0);
        }
        negative ^= pivot.is_sign_negative();
        let mut mag = pivot.abs();
        if mag < f64::MIN_POSITIVE {
            mag *= NORMALIZE_SUBNORMAL;
            exponent -= 54;
        }
        let bits = mag.to_bits();
        mantissa *= f64::from_bits((bits & FRAC) | UNIT);
        exponent += ((bits >> 52) as i64) - 1023;
        if mantissa >= 2.0 {
            mantissa *= 0.5;
            exponent += 1;
        }
    }
    let exponent = exponent.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
    bounded(libm::scalbn(
        if negative { -mantissa } else { mantissa },
        exponent,
    ))
}

pub(super) fn det(value: &V, ctx: &mut Ctx<'_, '_>) -> Result<f64> {
    let a = Matrix::read(value, ctx)?;
    let n = a.square()?;
    ctx.iterations(n)?;
    charge(ctx, &[&[n, n], &[n, n, n]])?;
    // A small nonzero determinant is a useful answer; only pivot zero
    // classifies the determinant as zero. Solve and rank use a tolerance.
    let fact = LowerUpper::factor(&a, n, 0.0, ctx)?;
    if fact.rank != n {
        return Ok(0.0);
    }
    pivot_product(
        (0..n).map(|i| fact.factors.cells[i * n + i]),
        fact.odd_swaps,
    )
}

pub(super) fn rank(value: &V, ctx: &mut Ctx<'_, '_>) -> Result<usize> {
    let a = Matrix::read(value, ctx)?;
    let k = a.rows.min(a.cols);
    ctx.iterations(if k == 0 { 0 } else { a.cols })?;
    charge(ctx, &[&[a.rows, a.cols], &[a.rows, a.cols, k]])?;
    if k == 0 {
        return Ok(0);
    }
    let mut cells = Buffer::matrix(ctx, &a)?;
    let (rank, _) = eliminate(
        &mut cells.cells,
        a.rows,
        a.cols,
        relative_epsilon(a.rows, a.cols),
        None,
    )?;
    Ok(rank)
}

pub(super) fn decompose(value: &V, ctx: &mut Ctx<'_, '_>) -> Result<V> {
    let a = Matrix::read(value, ctx)?;
    let n = a.square()?;
    ctx.iterations(n)?;
    charge(ctx, &[&[n, n, 3], &[n, n, n]])?;
    let fact = LowerUpper::factor(&a, n, relative_epsilon(n, n), ctx)?;
    fact.nonsingular()?;
    let mut lower = Buffer::zeroed(ctx, a.dense.len())?;
    let mut upper = Buffer::zeroed(ctx, a.dense.len())?;
    for i in 0..n {
        lower.cells[i * n + i] = 1.0;
        for j in 0..i {
            lower.cells[i * n + j] = fact.factors.cells[i * n + j];
        }
        for j in i..n {
            upper.cells[i * n + j] = fact.factors.cells[i * n + j];
        }
    }
    let perm = V::Sequence(
        fact.permutation
            .iter()
            .map(|&row| natural(row + 1))
            .collect(),
    );
    drop(fact);
    Ok(record([
        ("p", perm),
        ("l", lower.output(&[n, n], ctx)?),
        ("u", upper.output(&[n, n], ctx)?),
    ]))
}

pub(super) fn solve(value: &V, rhs: Option<&V>, ctx: &mut Ctx<'_, '_>) -> Result<V> {
    let a = Matrix::read(value, ctx)?;
    let n = a.square()?;
    let rhs = rhs.map(|b| Rhs::read(b, n, ctx)).transpose()?;
    let cols = rhs.as_ref().map_or(n, |b| b.cols);
    let (shape, rank) = rhs.as_ref().map_or(([n, n], 2), |b| b.out_shape(n));
    let cells = ctx.volume(&shape[..rank])?;
    ctx.iterations(n)?;
    charge(ctx, &[&[n, n], &[n, n, n], &[n, n, cols, 2], &[n, cols]])?;
    let fact = LowerUpper::factor(&a, n, relative_epsilon(n, n), ctx)?;
    fact.nonsingular()?;
    let mut out = Buffer::zeroed(ctx, cells)?;
    for (i, &row) in fact.permutation.iter().enumerate() {
        for j in 0..cols {
            out.cells[i * cols + j] = if let Some(rhs) = &rhs {
                super::real(rhs.get(row, j), ctx)?
            } else if row == j {
                1.0
            } else {
                0.0
            };
        }
    }
    let a = &fact.factors.cells;
    for i in 0..n {
        for j in 0..cols {
            let mut x = out.cells[i * cols + j];
            for k in 0..i {
                x = bounded((-a[i * n + k]).mul_add(out.cells[k * cols + j], x))?;
            }
            out.cells[i * cols + j] = x;
        }
    }
    for i in (0..n).rev() {
        for j in 0..cols {
            let mut x = out.cells[i * cols + j];
            for k in i + 1..n {
                x = bounded((-a[i * n + k]).mul_add(out.cells[k * cols + j], x))?;
            }
            out.cells[i * cols + j] = bounded(x / a[i * n + i])?;
        }
    }
    drop(fact);
    out.output(&shape[..rank], ctx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn determinant_pivots_avoid_spurious_overflow_and_underflow() {
        let verify = |pivots, odd, expected: f64| {
            let observed = pivot_product(pivots, odd).ok().unwrap();
            assert!((observed / expected - 1.0_f64).abs() < 3.0 * f64::EPSILON);
        };
        verify([1e308, 1e308, 1e-308, 1e-308], false, 1.0);
        verify([1e-308, 1e-308, 1e308, 1e308], true, -1.0);
    }
}
