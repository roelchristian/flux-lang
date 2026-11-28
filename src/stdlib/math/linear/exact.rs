//! Fraction-free determinants and exact elimination for checked real-domain matrices.
use super::{Ctx, Fault, Matrix, Result};
use crate::{
    hir::CoreOp,
    hir_eval::{domain, error, exact_binary, exact_i128, numeric_identity, wide, Domain, V},
    types::Type,
};
use num_bigint::BigInt;
use num_integer::Integer;
use num_rational::BigRational;
use num_traits::{One, Zero};

fn host(ctx: &Ctx<'_, '_>, message: &str) -> Fault {
    Fault::Host(error(ctx.span, message))
}

fn int_guard(ctx: &Ctx<'_, '_>, value: BigInt) -> Result<BigInt> {
    wide::guard(&value).map_err(|message| host(ctx, message))?;
    Ok(value)
}

fn ratio_guard(ctx: &Ctx<'_, '_>, value: BigRational) -> Result<BigRational> {
    for part in [value.numer(), value.denom()] {
        wide::guard(part).map_err(|message| host(ctx, message))?;
    }
    Ok(value)
}

fn output(ctx: &Ctx<'_, '_>, value: BigRational, domain: Domain) -> Result<V> {
    wide::ratio_value(value, domain).map_err(|message| host(ctx, message))
}

fn count(ctx: &Ctx<'_, '_>, a: usize, b: usize) -> Result<usize> {
    a.checked_mul(b).ok_or_else(|| {
        host(
            ctx,
            "linear algebra matrix dimensions exceed host resources",
        )
    })
}

fn bareiss(matrix: &Matrix<'_>, ctx: &mut Ctx<'_, '_>) -> Result<V> {
    let n = matrix.rows;
    if n == 0 {
        return output(ctx, BigRational::one(), Domain::Integer);
    }
    let len = count(ctx, n, n)?;
    let _lease = ctx.reserve::<BigInt>(len)?;
    ctx.charge(len)?;
    let mut cells = Vec::with_capacity(len);
    for i in 0..n {
        for j in 0..n {
            cells.push(wide::integral(&matrix.get(i, j)).map_err(|message| host(ctx, message))?);
        }
    }
    let mut previous = BigInt::one();
    let mut swapped = false;
    for k in 0..n - 1 {
        let mut pivot = None;
        for i in k..n {
            ctx.charge(1)?;
            if !cells[i * n + k].is_zero() {
                pivot = Some(i);
                break;
            }
        }
        let Some(pivot) = pivot else {
            return Ok(exact_i128(0, Domain::Integer));
        };
        if pivot != k {
            ctx.charge(n)?;
            for j in 0..n {
                cells.swap(k * n + j, pivot * n + j);
            }
            swapped = !swapped;
        }
        let diagonal = cells[k * n + k].clone();
        for i in k + 1..n {
            let lower = cells[i * n + k].clone();
            ctx.charge(n - k - 1)?;
            for j in k + 1..n {
                let cross = int_guard(ctx, &cells[i * n + j] * &diagonal)?;
                let correction = int_guard(ctx, &lower * &cells[k * n + j])?;
                let numerator = int_guard(ctx, cross - correction)?;
                let (quotient, remainder) = numerator.div_rem(&previous);
                if !remainder.is_zero() {
                    return Err(host(ctx, "Bareiss elimination lost exact divisibility"));
                }
                cells[i * n + j] = int_guard(ctx, quotient)?;
            }
            cells[i * n + k] = BigInt::zero();
        }
        previous = diagonal;
    }
    let determinant = cells.pop().expect("nonempty square matrix");
    output(
        ctx,
        BigRational::from_integer(if swapped { -determinant } else { determinant }),
        Domain::Integer,
    )
}

fn load_matrix(matrix: &Matrix<'_>, ctx: &mut Ctx<'_, '_>) -> Result<Vec<BigRational>> {
    let len = count(ctx, matrix.rows, matrix.cols)?;
    ctx.charge(len)?;
    let mut cells = Vec::with_capacity(len);
    for i in 0..matrix.rows {
        for j in 0..matrix.cols {
            cells.push(wide::ratio(&matrix.get(i, j)).map_err(|message| host(ctx, message))?);
        }
    }
    Ok(cells)
}

fn swap_rows<T>(matrix: &mut [T], columns: usize, left: usize, right: usize) {
    for c in 0..columns {
        matrix.swap(left * columns + c, right * columns + c);
    }
}

fn exact_eliminate(
    matrix: &Matrix<'_>,
    cells: &mut [BigRational],
    rhs: &mut [BigRational],
    rhs_cols: usize,
    reduced: bool,
    determinant: bool,
    ctx: &mut Ctx<'_, '_>,
) -> Result<(usize, BigRational)> {
    let (m, n) = (matrix.rows, matrix.cols);
    let mut rank = 0;
    let mut det = BigRational::one();
    for j in 0..n {
        let mut pivot = None;
        for i in rank..m {
            ctx.charge(1)?;
            if !cells[i * n + j].is_zero() {
                pivot = Some(i);
                break;
            }
        }
        let Some(pivot) = pivot else { continue };
        if pivot != rank {
            ctx.charge(n.saturating_add(rhs_cols))?;
            swap_rows(cells, n, pivot, rank);
            swap_rows(rhs, rhs_cols, pivot, rank);
            if determinant {
                det = -det;
            }
        }
        let diagonal = cells[rank * n + j].clone();
        if determinant {
            det = ratio_guard(ctx, det * &diagonal)?;
        }
        if reduced {
            ctx.charge((n - j).saturating_add(rhs_cols))?;
            for k in j..n {
                let cell = &mut cells[rank * n + k];
                *cell = ratio_guard(ctx, &*cell / &diagonal)?;
            }
            for k in 0..rhs_cols {
                let cell = &mut rhs[rank * rhs_cols + k];
                *cell = ratio_guard(ctx, &*cell / &diagonal)?;
            }
        }
        let first = if reduced { 0 } else { rank + 1 };
        for i in first..m {
            ctx.charge(1)?;
            if i == rank || cells[i * n + j].is_zero() {
                continue;
            }
            let factor = if reduced {
                cells[i * n + j].clone()
            } else {
                ratio_guard(ctx, &cells[i * n + j] / &diagonal)?
            };
            ctx.charge((n - j - 1).saturating_add(rhs_cols))?;
            for k in j + 1..n {
                let product = ratio_guard(ctx, &factor * &cells[rank * n + k])?;
                cells[i * n + k] = ratio_guard(ctx, &cells[i * n + k] - product)?;
            }
            for k in 0..rhs_cols {
                let product = ratio_guard(ctx, &factor * &rhs[rank * rhs_cols + k])?;
                rhs[i * rhs_cols + k] = ratio_guard(ctx, &rhs[i * rhs_cols + k] - product)?;
            }
            cells[i * n + j] = BigRational::zero();
        }
        rank += 1;
        if rank == m {
            break;
        }
    }
    Ok((rank, det))
}

fn solve(matrix: &Matrix<'_>, rhs: Option<&V>, ctx: &mut Ctx<'_, '_>) -> Result<V> {
    let rows = matrix.rows;
    let (columns, shape, rhs_array) = match rhs {
        Some(V::Dense(rhs)) => match rhs.shape() {
            [m] if *m == rows => (1, vec![rows], Some(rhs)),
            [m, n] if *m == rows => (*n, vec![rows, *n], Some(rhs)),
            _ => {
                return Err(Fault::BadShape(
                    "right-hand side must have the same leading dimension",
                ))
            }
        },
        Some(_) => {
            return Err(host(
                ctx,
                "checked solve right-hand side is not a dense array",
            ))
        }
        None => (rows, vec![rows, rows], None),
    };
    let len = ctx.volume(&shape)?;
    let _matrix = ctx.reserve::<BigRational>(count(ctx, rows, rows)?)?;
    let mut cells = load_matrix(matrix, ctx)?;
    let _lease = ctx.reserve::<BigRational>(len)?;
    ctx.charge(len)?;
    let mut values = Vec::with_capacity(len);
    if let Some(rhs) = rhs_array {
        for v in rhs.iter() {
            values.push(wide::ratio(&v).map_err(|message| host(ctx, message))?);
        }
    } else {
        for i in 0..rows {
            for j in 0..columns {
                values.push(if i == j {
                    BigRational::one()
                } else {
                    BigRational::zero()
                });
            }
        }
    }
    let (rank, _) = exact_eliminate(matrix, &mut cells, &mut values, columns, true, false, ctx)?;
    if rank != rows {
        return Err(Fault::Singular("linear system is singular"));
    }
    let output_lease = ctx.reserve::<V>(len)?;
    let values = values
        .into_iter()
        .map(|x| output(ctx, x, Domain::Rational))
        .collect::<Result<Vec<_>>>()?;
    ctx.array_reserved(&shape, values, output_lease)
}

fn product(
    left: &[V],
    right: &[V],
    n: usize,
    kind: Domain,
    zero: &V,
    ctx: &mut Ctx<'_, '_>,
) -> Result<Vec<V>> {
    let len = count(ctx, n, n)?;
    let mut result = Vec::with_capacity(len);
    for i in 0..n {
        for j in 0..n {
            ctx.charge(n)?;
            let mut total = zero.clone();
            for k in 0..n {
                let p = exact_binary(CoreOp::Mul, &left[i * n + k], &right[k * n + j], kind)
                    .map_err(|message| host(ctx, message))?;
                total = exact_binary(CoreOp::Add, &total, &p, kind)
                    .map_err(|message| host(ctx, message))?;
            }
            result.push(total);
        }
    }
    Ok(result)
}

fn powers(
    matrix: &Matrix<'_>,
    exponent: &V,
    ty: &Type,
    kind: Domain,
    ctx: &mut Ctx<'_, '_>,
) -> Result<V> {
    let mut exponent = wide::exponent(exponent).map_err(|message| host(ctx, message))?;
    let n = matrix.rows;
    let len = count(ctx, n, n)?;
    let _lease = ctx.reserve::<V>(if exponent.is_zero() {
        len
    } else {
        count(ctx, len, 3)?
    })?;
    let zero = numeric_identity(ty, false).map_err(|message| host(ctx, message))?;
    let one = numeric_identity(ty, true).map_err(|message| host(ctx, message))?;
    ctx.charge(len)?;
    if exponent.is_zero() {
        let identity = (0..len)
            .map(|i| {
                if i / n == i % n {
                    one.clone()
                } else {
                    zero.clone()
                }
            })
            .collect();
        return ctx.array(&[n, n], identity);
    }
    // No product is needed for a zero square or for the first set exponent bit.
    // This preserves exact decimal representation for exponent one.
    if n == 0 {
        return ctx.array(&[0, 0], Vec::new());
    }
    let mut base: Vec<V> = (0..len).map(|i| matrix.get(i / n, i % n)).collect();
    let mut result: Option<Vec<V>> = None;
    while !exponent.is_zero() {
        ctx.charge(1)?;
        if exponent.is_odd() {
            result = Some(match result {
                Some(ref left) => product(left, &base, n, kind, &zero, ctx)?,
                None => base.clone(),
            });
        }
        exponent >>= 1;
        if !exponent.is_zero() {
            base = product(&base, &base, n, kind, &zero, ctx)?;
        }
    }
    ctx.array(&[n, n], result.expect("positive exponent"))
}

pub(super) fn apply(op: CoreOp, args: &[V], types: &[Type], ctx: &mut Ctx<'_, '_>) -> Result<V> {
    let matrix = Matrix::read(&args[0], ctx)?;
    let ty = types[0]
        .array_element()
        .ok_or_else(|| host(ctx, "missing exact array witness"))?;
    let kind = domain(ty).ok_or_else(|| host(ctx, "missing exact numeric witness"))?;
    if !matches!(op, CoreOp::LinRank) && matrix.rows != matrix.cols {
        return Err(Fault::BadShape(
            "linear algebra operation requires a square matrix",
        ));
    }
    if op == CoreOp::LinRank && (matrix.rows == 0 || matrix.cols == 0) {
        return Ok(exact_i128(0, Domain::Natural));
    }
    if op == CoreOp::LinDet && matches!(kind, Domain::Natural | Domain::Integer) {
        return bareiss(&matrix, ctx);
    }
    if op == CoreOp::LinPow {
        return powers(&matrix, &args[1], ty, kind, ctx);
    }
    match op {
        CoreOp::LinRank | CoreOp::LinDet => {
            let _lease = ctx.reserve::<BigRational>(count(ctx, matrix.rows, matrix.cols)?)?;
            let mut cells = load_matrix(&matrix, ctx)?;
            let (rank, det) = exact_eliminate(
                &matrix,
                &mut cells,
                &mut [],
                0,
                false,
                op == CoreOp::LinDet,
                ctx,
            )?;
            if op == CoreOp::LinRank {
                output(ctx, BigRational::from_integer(rank.into()), Domain::Natural)
            } else {
                output(
                    ctx,
                    if rank == matrix.rows {
                        det
                    } else {
                        BigRational::zero()
                    },
                    Domain::Rational,
                )
            }
        }
        CoreOp::LinSolve => solve(&matrix, args.get(1), ctx),
        CoreOp::LinInv => solve(&matrix, None, ctx),
        _ => Err(host(ctx, "invalid checked exact linear algebra operation")),
    }
}
