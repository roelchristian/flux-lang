//! Thin Householder QR and its full-column-rank least-squares solve.
use super::{
    bounded, charge, max_abs, negligible, Buffer, Ctx, Fault, Matrix, Result, Rhs, ScaledSquares,
};
use crate::hir_eval::{
    math::linear::{real, record},
    resources::MemoryReservation,
    V,
};

struct Householder {
    factor: Buffer,
    heads: Vec<f64>,
    _heads_lease: MemoryReservation,
    m: usize,
    n: usize,
    scale: f64,
}

#[derive(Clone, Copy)]
enum Delta {
    Plain(f64),
    Scaled { amount: f64, scale: f64 },
}

impl Delta {
    fn read(len: usize, read: impl Fn(usize) -> (f64, f64)) -> Self {
        let mut dot = 0.0;
        for i in 0..len {
            let (v, x) = read(i);
            dot = v.mul_add(x, dot);
        }
        let twice = 2.0 * dot;
        if twice.is_finite() {
            return Self::Plain(twice);
        }
        // The transformation can be representable even when the dot
        // or its doubled value alone would overflow.
        let scale = (0..len).fold(0.0_f64, |scale, i| scale.max(read(i).1.abs()));
        let mut normal = 0.0;
        for i in 0..len {
            let (v, x) = read(i);
            normal = v.mul_add(x / scale, normal);
        }
        Self::Scaled {
            amount: 2.0 * normal,
            scale,
        }
    }

    fn apply(self, v: f64, value: f64) -> Result<f64> {
        if v == 0.0 {
            return Ok(value);
        }
        bounded(match self {
            Self::Plain(amount) => (-amount).mul_add(v, value),
            Self::Scaled { amount, scale } => (-amount).mul_add(v, value / scale) * scale,
        })
    }
}

fn coefficient(a: &[f64], n: usize, column: usize, head: f64, row: usize) -> f64 {
    if row == column {
        head
    } else {
        a[row * n + column]
    }
}

impl Householder {
    fn factor(
        a: &Matrix<'_>,
        mut rhs: Option<(&mut [f64], usize)>,
        ctx: &Ctx<'_, '_>,
    ) -> Result<Self> {
        let (m, n) = (a.rows, a.cols);
        let k = m.min(n);
        let mut factor = Buffer::matrix(ctx, a)?;
        let scale = max_abs(&factor.cells);
        let (mut heads, lease) = ctx.zeros(k, 0.0)?;
        let cells = &mut factor.cells;
        for (j, head) in heads.iter_mut().enumerate() {
            let mut squares = ScaledSquares::default();
            for i in j..m {
                squares.add(cells[i * n + j]);
            }
            if squares.scale == 0.0 {
                continue;
            }
            let scale = squares.scale;
            let norm = squares.sum.sqrt();
            let first = cells[j * n + j];
            let scaled_first = first / scale;
            // Work with a normalized unit reflector. Adding two
            // same-sign scaled numbers avoids cancellation and overflow.
            let scaled_head = scaled_first + norm.copysign(first);
            let length = (2.0 * norm * (norm + scaled_first.abs())).sqrt();
            *head = scaled_head / length;
            for i in j + 1..m {
                cells[i * n + j] = (cells[i * n + j] / scale) / length;
            }
            cells[j * n + j] = bounded(-(scale * norm).copysign(first))?;

            for column in j + 1..n {
                let delta = Delta::read(m - j, |i| {
                    let row = j + i;
                    (
                        coefficient(cells, n, j, *head, row),
                        cells[row * n + column],
                    )
                });
                for row in j..m {
                    let v = coefficient(cells, n, j, *head, row);
                    let slot = &mut cells[row * n + column];
                    *slot = delta.apply(v, *slot)?;
                }
            }
            if let Some((b, width)) = &mut rhs {
                let width = *width;
                for column in 0..width {
                    let delta = Delta::read(m - j, |i| {
                        let row = j + i;
                        (
                            coefficient(cells, n, j, *head, row),
                            b[row * width + column],
                        )
                    });
                    for row in j..m {
                        let v = coefficient(cells, n, j, *head, row);
                        let slot = &mut b[row * width + column];
                        *slot = delta.apply(v, *slot)?;
                    }
                }
            }
        }
        Ok(Self {
            factor,
            heads,
            _heads_lease: lease,
            m,
            n,
            scale,
        })
    }

    fn thin_q(&self, ctx: &Ctx<'_, '_>, len: usize) -> Result<Buffer> {
        let k = self.heads.len();
        let mut q = Buffer::zeroed(ctx, len)?;
        for i in 0..k {
            q.cells[i * k + i] = 1.0;
        }
        let cells = &self.factor.cells;
        for (j, &head) in self.heads.iter().enumerate().rev() {
            if head == 0.0 {
                continue;
            }
            // Q = H0 H1 ... H(k−1) I. Start from the right.
            for column in j..k {
                let delta = Delta::read(self.m - j, |i| {
                    let row = j + i;
                    (
                        coefficient(cells, self.n, j, head, row),
                        q.cells[row * k + column],
                    )
                });
                for row in j..self.m {
                    let v = coefficient(cells, self.n, j, head, row);
                    let slot = &mut q.cells[row * k + column];
                    *slot = delta.apply(v, *slot)?;
                }
            }
        }
        Ok(q)
    }
}

pub(super) fn decompose(value: &V, ctx: &mut Ctx<'_, '_>) -> Result<V> {
    let a = Matrix::read(value, ctx)?;
    let (m, n) = (a.rows, a.cols);
    let k = m.min(n);
    let qshape = [m, k];
    let rshape = [k, n];
    let qlen = ctx.volume(&qshape)?;
    let rlen = ctx.volume(&rshape)?;
    ctx.iterations(k)?;
    charge(ctx, &[&[m, n], &[m, n, k], &[m, k, k], &[qlen], &[rlen]])?;
    let factor = Householder::factor(&a, None, ctx)?;
    let q = factor.thin_q(ctx, qlen)?;
    let mut r = Buffer::zeroed(ctx, rlen)?;
    for i in 0..k {
        let row = i * n + i..(i + 1) * n;
        r.cells[row.clone()].copy_from_slice(&factor.factor.cells[row]);
    }
    drop(factor);
    Ok(record([
        ("q", q.output(&qshape, ctx)?),
        ("r", r.output(&rshape, ctx)?),
    ]))
}

pub(super) fn least_squares(tol: &V, a: &V, b: &V, ctx: &mut Ctx<'_, '_>) -> Result<V> {
    let tolerance = real(tol.clone(), ctx)?;
    if !(0.0 < tolerance && tolerance < 1.0) {
        return Err(Fault::BadTolerance(
            "lin.lstsq tolerance must be strictly between zero and one",
        ));
    }
    let a = Matrix::read(a, ctx)?;
    let (m, n) = (a.rows, a.cols);
    if m < n {
        return Err(Fault::BadShape(
            "lin.lstsq requires at least as many rows as columns",
        ));
    }
    let rhs = Rhs::read(b, m, ctx)?;
    let (shape, rank) = rhs.out_shape(n);
    let width = rhs.cols;
    let out_len = ctx.volume(&shape[..rank])?;
    // Avoid walking the potentially huge leading dimension of an empty view.
    if n == 0 {
        return Buffer::zeroed(ctx, 0)?.output(&shape[..rank], ctx);
    }
    ctx.iterations(n)?;
    charge(
        ctx,
        &[
            &[m, n],
            &[m, n, n],
            &[m, n, width, 2],
            &[n, n, width],
            &[m, width],
        ],
    )?;
    let mut transformed = Buffer::rhs(ctx, &rhs)?;
    let factor = Householder::factor(&a, Some((&mut transformed.cells, width)), ctx)?;
    let r = &factor.factor.cells;
    for i in 0..n {
        if negligible(r[i * n + i], factor.scale, tolerance) {
            return Err(Fault::RankDeficient(
                "lin.lstsq requires full column rank at the supplied tolerance",
            ));
        }
    }
    let mut out = Buffer::zeroed(ctx, out_len)?;
    for i in (0..n).rev() {
        for j in 0..width {
            let mut value = transformed.cells[i * width + j];
            for k in i + 1..n {
                value = bounded((-r[i * n + k]).mul_add(out.cells[k * width + j], value))?;
            }
            out.cells[i * width + j] = bounded(value / r[i * n + i])?;
        }
    }
    drop(factor);
    drop(transformed);
    out.output(&shape[..rank], ctx)
}
