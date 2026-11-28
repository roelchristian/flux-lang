//! Bounded binary64 kernels. All traversals of input views are logical.
use super::{bounded, natural, real, Ctx, Fault, Matrix, Result, Rhs, Vector};
use crate::hir_eval::{resources::MemoryReservation, wide, CoreOp, V};

mod lu;
mod qr;

struct Buffer {
    cells: Vec<f64>,
    lease: MemoryReservation,
}

impl Buffer {
    fn zeroed(ctx: &Ctx<'_, '_>, len: usize) -> Result<Self> {
        let (cells, lease) = ctx.zeros(len, 0.0)?;
        Ok(Self { cells, lease })
    }

    fn matrix(ctx: &Ctx<'_, '_>, matrix: &Matrix<'_>) -> Result<Self> {
        let mut work = Self::zeroed(ctx, matrix.dense.len())?;
        if let Some(cells) = matrix.dense.real_slice() {
            work.cells.copy_from_slice(cells);
        } else {
            for (cell, value) in work.cells.iter_mut().zip(matrix.dense.iter()) {
                *cell = real(value, ctx)?;
            }
        }
        Ok(work)
    }

    fn rhs(ctx: &Ctx<'_, '_>, rhs: &Rhs<'_>) -> Result<Self> {
        let mut work = Self::zeroed(ctx, rhs.dense.len())?;
        for (cell, value) in work.cells.iter_mut().zip(rhs.dense.iter()) {
            *cell = real(value, ctx)?;
        }
        Ok(work)
    }

    fn output(self, shape: &[usize], ctx: &Ctx<'_, '_>) -> Result<V> {
        ctx.real_array(shape, self.cells, self.lease)
    }
}

fn charge(ctx: &mut Ctx<'_, '_>, terms: &[&[usize]]) -> Result<()> {
    let total = terms.iter().try_fold(0usize, |total, axes| {
        if axes.contains(&0) {
            return Some(total);
        }
        let units = axes
            .iter()
            .try_fold(1usize, |prod, &n| prod.checked_mul(n))?;
        total.checked_add(units)
    });
    ctx.charge(total.ok_or_else(|| ctx.host("linear algebra work count exceeds host limits"))?)
}

fn max_abs(values: &[f64]) -> f64 {
    values.iter().fold(0.0_f64, |max, &x| max.max(x.abs()))
}

fn relative_epsilon(rows: usize, cols: usize) -> f64 {
    f64::EPSILON * rows.max(cols).max(1) as f64
}

fn negligible(x: f64, scale: f64, tolerance: f64) -> bool {
    x == 0.0 || (tolerance > 0.0 && scale > 0.0 && x.abs() / scale <= tolerance)
}

#[derive(Default)]
struct ScaledSquares {
    scale: f64,
    sum: f64,
}

impl ScaledSquares {
    fn add(&mut self, x: f64) {
        let x = x.abs();
        if x == 0.0 {
            return;
        }
        if self.scale < x {
            let ratio = self.scale / x;
            self.sum = 1.0 + self.sum * (ratio * ratio);
            self.scale = x;
        } else {
            let ratio = x / self.scale;
            self.sum += ratio * ratio;
        }
    }

    fn norm(self) -> Result<f64> {
        bounded(self.scale * self.sum.sqrt())
    }
}

fn norm(op: CoreOp, value: &V, ctx: &mut Ctx<'_, '_>) -> Result<V> {
    let mut accumulator = ScaledSquares::default();
    if op == CoreOp::LinNorm {
        let vector = Vector::read(value, ctx)?;
        ctx.charge(vector.len)?;
        for i in 0..vector.len {
            accumulator.add(real(vector.get(i), ctx)?);
        }
    } else {
        let matrix = Matrix::read(value, ctx)?;
        ctx.charge(matrix.dense.len())?;
        for value in matrix.dense.iter() {
            accumulator.add(real(value, ctx)?);
        }
    }
    Ok(V::Real(accumulator.norm()?))
}

fn cholesky(value: &V, ctx: &mut Ctx<'_, '_>) -> Result<V> {
    let matrix = Matrix::read(value, ctx)?;
    let n = matrix.square()?;
    ctx.iterations(n)?;
    charge(ctx, &[&[n, n], &[n, n, n]])?;
    let mut lower = Buffer::zeroed(ctx, matrix.dense.len())?;
    for i in 0..n {
        for j in 0..i {
            if real(matrix.get(i, j), ctx)? != real(matrix.get(j, i), ctx)? {
                return Err(Fault::NotPositiveDefinite(
                    "lin.cholesky requires a symmetric matrix",
                ));
            }
        }
    }
    for i in 0..n {
        for j in 0..=i {
            let original = real(matrix.get(i, j), ctx)?;
            let mut remaining = original;
            for k in 0..j {
                remaining =
                    bounded((-lower.cells[i * n + k]).mul_add(lower.cells[j * n + k], remaining))?;
            }
            lower.cells[i * n + j] = if i == j {
                // Compare to this original diagonal, not the global norm:
                // a scaled positive definite diagonal can span all of binary64.
                if remaining <= 0.0 || remaining / original <= relative_epsilon(n, n) {
                    return Err(Fault::NotPositiveDefinite(
                        "lin.cholesky encountered a nonpositive or numerically zero pivot",
                    ));
                }
                remaining.sqrt()
            } else {
                bounded(remaining / lower.cells[j * n + j])?
            };
        }
    }
    lower.output(&[n, n], ctx)
}

fn matrix_multiply(a: &[f64], b: &[f64], n: usize, ctx: &Ctx<'_, '_>) -> Result<Buffer> {
    let mut out = Buffer::zeroed(ctx, a.len())?;
    // LinPow uses the same multiply-then-add and order as arr.mul.
    for i in 0..n {
        for j in 0..n {
            let mut sum = 0.0;
            for k in 0..n {
                let term = bounded(a[i * n + k] * b[k * n + j])?;
                sum = bounded(sum + term)?;
            }
            out.cells[i * n + j] = sum;
        }
    }
    Ok(out)
}

fn power(value: &V, exponent: &V, ctx: &mut Ctx<'_, '_>) -> Result<V> {
    let matrix = Matrix::read(value, ctx)?;
    let n = matrix.square()?;
    if n == 0 {
        return Buffer::zeroed(ctx, 0)?.output(&[0, 0], ctx);
    }
    let exponent = wide::exponent(exponent).map_err(|detail| ctx.invariant(detail))?;
    let exponent = exponent.magnitude();
    let bits = usize::try_from(exponent.bits())
        .map_err(|_| ctx.host("linear algebra exponent exceeds host limits"))?;
    ctx.iterations(bits)?;
    let ones = usize::try_from(exponent.count_ones())
        .map_err(|_| ctx.host("linear algebra exponent exceeds host limits"))?;
    let steps = bits
        .saturating_sub(1)
        .checked_add(ones.saturating_sub(1))
        .ok_or_else(|| ctx.host("linear algebra exponent exceeds host limits"))?;
    charge(ctx, &[&[n, n], &[steps, n, n, n]])?;
    if bits == 0 {
        let mut out = Buffer::zeroed(ctx, matrix.dense.len())?;
        for i in 0..n {
            out.cells[i * n + i] = 1.0;
        }
        return out.output(&[n, n], ctx);
    }
    let mut base = Buffer::matrix(ctx, &matrix)?;
    if bits == 1 {
        return base.output(&[n, n], ctx);
    }
    let mut out: Option<Buffer> = None;
    for bit in 0..bits {
        if exponent.bit(bit as u64) {
            out = Some(if let Some(ref previous) = out {
                matrix_multiply(&previous.cells, &base.cells, n, ctx)?
            } else {
                let mut start = Buffer::zeroed(ctx, base.cells.len())?;
                start.cells.copy_from_slice(&base.cells);
                start
            });
        }
        if bit + 1 < bits {
            base = matrix_multiply(&base.cells, &base.cells, n, ctx)?;
        }
    }
    out.ok_or_else(|| ctx.invariant("nonzero linear power has no set bits"))?
        .output(&[n, n], ctx)
}

pub(super) fn apply(op: CoreOp, args: &[V], ctx: &mut Ctx<'_, '_>) -> Result<V> {
    match (op, args) {
        (CoreOp::LinNorm | CoreOp::LinFrob, [a]) => norm(op, a, ctx),
        (CoreOp::LinCholesky, [a]) => cholesky(a, ctx),
        (CoreOp::LinPow, [a, exponent]) => power(a, exponent, ctx),
        (CoreOp::LinRank, [a]) => lu::rank(a, ctx).map(natural),
        (CoreOp::LinDet, [a]) => lu::det(a, ctx).map(V::Real),
        (CoreOp::LinLu, [a]) => lu::decompose(a, ctx),
        (CoreOp::LinSolve, [a, b]) => lu::solve(a, Some(b), ctx),
        (CoreOp::LinInv, [a]) => lu::solve(a, None, ctx),
        (CoreOp::LinQr, [a]) => qr::decompose(a, ctx),
        (CoreOp::LinLstsq, [tolerance, a, b]) => qr::least_squares(tolerance, a, b, ctx),
        _ => Err(ctx.invariant("invalid operands for checked real linear algebra")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn norm_resists_intermediate_overflow_and_underflow() {
        let check = |x, y, want: f64| {
            let mut sum = ScaledSquares::default();
            sum.add(x);
            sum.add(y);
            assert!((sum.norm().ok().unwrap() / want - 1.0_f64).abs() < 3.0 * f64::EPSILON);
        };
        check(3e200, 4e200, 5e200);
        check(3e-200, 4e-200, 5e-200);
        assert_eq!(ScaledSquares::default().norm().ok(), Some(0.0));
    }
}
