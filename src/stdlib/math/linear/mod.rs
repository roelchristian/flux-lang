//! Shape-strict linear algebra over compiler-selected numeric domains.
use super::super::{
    array, effects, error, exact_i128, exact_index, invariant, numeric_identity, resources,
    scalar_binary, succeeded, Control, CoreOp, Domain, Eval, NumericResult, Prim, Span, Type, R, V,
};
use array::Dense;
use resources::MemoryReservation;

mod approximate;
mod exact;

enum Fault {
    BadShape(&'static str),
    Singular(&'static str),
    RankDeficient(&'static str),
    NotPositiveDefinite(&'static str),
    BadTolerance(&'static str),
    Nonfinite(&'static str),
    #[allow(dead_code)] // Reserved for a method with a convergence test.
    NoConvergence(&'static str),
    Host(Control),
}
type Result<T> = std::result::Result<T, Fault>;

impl From<Control> for Fault {
    fn from(control: Control) -> Self {
        Self::Host(control)
    }
}

struct Ctx<'call, 'eval> {
    eval: &'call mut Eval<'eval>,
    span: Span,
}

impl Ctx<'_, '_> {
    fn charge(&mut self, work: usize) -> Result<()> {
        self.eval.charge_work(work, self.span).map_err(Into::into)
    }

    fn iterations(&self, steps: usize) -> Result<()> {
        resources::check_iterations(steps).map_err(|message| self.host(message))
    }

    fn host(&self, message: &'static str) -> Fault {
        Fault::Host(error(self.span, message))
    }

    fn invariant(&self, message: &'static str) -> Fault {
        Fault::Host(invariant(self.span, message))
    }

    fn reserve<T>(&self, len: usize) -> Result<MemoryReservation> {
        resources::reserve_for::<T>(len).map_err(|message| self.host(message))
    }

    fn zeros<T: Clone>(&self, len: usize, value: T) -> Result<(Vec<T>, MemoryReservation)> {
        let lease = self.reserve::<T>(len)?;
        let mut buffer = Vec::new();
        buffer
            .try_reserve_exact(len)
            .map_err(|_| self.host("linear algebra allocation exceeded host resources"))?;
        buffer.resize(len, value);
        Ok((buffer, lease))
    }

    fn volume(&self, shape: &[usize]) -> Result<usize> {
        array::checked_volume(shape).map_err(|message| self.host(message))
    }

    fn array_reserved(
        &self,
        shape: &[usize],
        cells: Vec<V>,
        lease: MemoryReservation,
    ) -> Result<V> {
        Dense::pack_values_reserved(shape.into(), cells, lease, self.span).map_err(Into::into)
    }

    fn array(&self, shape: &[usize], cells: Vec<V>) -> Result<V> {
        let len = self.volume(shape)?;
        self.array_reserved(shape, cells, self.reserve::<V>(len)?)
    }

    fn real_array(&self, shape: &[usize], cells: Vec<f64>, lease: MemoryReservation) -> Result<V> {
        if resources::optimized() {
            return Dense::pack_real_reserved(shape.into(), cells, lease, self.span)
                .map_err(Into::into);
        }
        // The reference numerical kernels use f64 scratch in either mode.
        // Generic mode keeps the generic array representation after return.
        let (mut values, values_lease) = self.zeros(cells.len(), V::Unit)?;
        for (slot, real) in values.iter_mut().zip(cells) {
            *slot = V::Real(real);
        }
        drop(lease);
        self.array_reserved(shape, values, values_lease)
    }

    fn numeric<T>(&self, result: NumericResult<T>) -> Result<T> {
        result.map_err(|message| {
            if message.contains("nonfinite") {
                Fault::Nonfinite(message)
            } else {
                self.host(message)
            }
        })
    }
}

struct Matrix<'a> {
    dense: &'a Dense,
    rows: usize,
    cols: usize,
}

impl<'a> Matrix<'a> {
    fn read(value: &'a V, ctx: &Ctx<'_, '_>) -> Result<Self> {
        let V::Dense(dense) = value else {
            return Err(ctx.invariant("checked linear algebra expects a dense array"));
        };
        let &[rows, cols] = dense.shape() else {
            return Err(Fault::BadShape("linear algebra expects a rank-two matrix"));
        };
        Ok(Self { dense, rows, cols })
    }

    fn square(&self) -> Result<usize> {
        if self.rows == self.cols {
            Ok(self.rows)
        } else {
            Err(Fault::BadShape("linear algebra expects a square matrix"))
        }
    }

    fn get(&self, row: usize, col: usize) -> V {
        self.dense.value(row * self.cols + col)
    }
}

struct Vector<'a> {
    dense: &'a Dense,
    len: usize,
}

impl<'a> Vector<'a> {
    fn read(value: &'a V, ctx: &Ctx<'_, '_>) -> Result<Self> {
        let V::Dense(dense) = value else {
            return Err(ctx.invariant("checked linear algebra expects a dense array"));
        };
        let &[len] = dense.shape() else {
            return Err(Fault::BadShape("linear algebra expects a rank-one vector"));
        };
        Ok(Self { dense, len })
    }

    fn get(&self, index: usize) -> V {
        self.dense.value(index)
    }
}

/// Rank-preserving view of the right-hand side of a linear system.
struct Rhs<'a> {
    dense: &'a Dense,
    cols: usize,
    vector: bool,
}

impl<'a> Rhs<'a> {
    fn read(value: &'a V, rows: usize, ctx: &Ctx<'_, '_>) -> Result<Self> {
        let V::Dense(dense) = value else {
            return Err(ctx.invariant("checked linear algebra expects a dense right-hand side"));
        };
        let (n, cols, vector) = match *dense.shape() {
            [n] => (n, 1, true),
            [n, cols] => (n, cols, false),
            _ => {
                return Err(Fault::BadShape(
                    "the right-hand side must be a vector or rank-two matrix",
                ))
            }
        };
        if n != rows {
            return Err(Fault::BadShape(
                "the right-hand side row count must match the input matrix",
            ));
        }
        Ok(Self {
            dense,
            cols,
            vector,
        })
    }

    fn get(&self, row: usize, col: usize) -> V {
        self.dense.value(row * self.cols + col)
    }

    fn out_shape(&self, rows: usize) -> ([usize; 2], usize) {
        ([rows, self.cols], if self.vector { 1 } else { 2 })
    }
}

fn natural(n: usize) -> V {
    exact_i128(n as i128, Domain::Natural)
}

fn record(fields: impl IntoIterator<Item = (&'static str, V)>) -> V {
    V::Record(
        fields
            .into_iter()
            .map(|(name, value)| (name.into(), value))
            .collect(),
    )
}

fn bounded(real: f64) -> Result<f64> {
    if real.is_finite() {
        Ok(real)
    } else {
        Err(Fault::Nonfinite(
            "linear algebra produced a nonfinite real result",
        ))
    }
}

fn real(value: V, ctx: &Ctx<'_, '_>) -> Result<f64> {
    match value {
        V::Real(x) => bounded(x),
        _ => Err(ctx.invariant("checked real linear algebra requires real elements")),
    }
}

fn common(op: CoreOp, args: &[V], element: &Type, ctx: &mut Ctx<'_, '_>) -> Result<V> {
    match (op, args) {
        (CoreOp::LinEye, [n]) => {
            let n = exact_index(n).map_err(|msg| ctx.invariant(msg))?;
            let shape = [n, n];
            let len = ctx.volume(&shape)?;
            ctx.charge(len)?;
            if resources::optimized() {
                let (mut cells, lease) = ctx.zeros(len, 0_i64)?;
                for i in 0..n {
                    cells[i * n + i] = 1;
                }
                return Dense::pack_int_reserved(shape.into(), cells, lease, ctx.span)
                    .map_err(Into::into);
            }
            let (mut cells, lease) = ctx.zeros(len, exact_i128(0, Domain::Integer))?;
            for i in 0..n {
                cells[i * n + i] = exact_i128(1, Domain::Integer);
            }
            ctx.array_reserved(&shape, cells, lease)
        }
        (CoreOp::LinDiag, [V::Dense(dense)]) => match *dense.shape() {
            [n] => {
                let shape = [n, n];
                let len = ctx.volume(&shape)?;
                ctx.charge(len)?;
                let zero = ctx.numeric(numeric_identity(element, false))?;
                let (mut cells, lease) = ctx.zeros(len, zero)?;
                for i in 0..n {
                    cells[i * n + i] = dense.value(i);
                }
                ctx.array_reserved(&shape, cells, lease)
            }
            [rows, cols] => {
                let n = rows.min(cols);
                ctx.charge(n)?;
                let (mut cells, lease) = ctx.zeros(n, V::Unit)?;
                for (i, cell) in cells.iter_mut().enumerate() {
                    *cell = dense.value(i * cols + i);
                }
                ctx.array_reserved(&[n], cells, lease)
            }
            _ => Err(Fault::BadShape(
                "lin.diag expects a vector or rank-two matrix",
            )),
        },
        (CoreOp::LinTrace, [value]) => {
            let matrix = Matrix::read(value, ctx)?;
            let n = matrix.square()?;
            ctx.charge(n)?;
            let mut total = ctx.numeric(numeric_identity(element, false))?;
            for i in 0..n {
                total =
                    ctx.numeric(scalar_binary(CoreOp::Add, total, matrix.get(i, i), element))?;
            }
            Ok(total)
        }
        _ => Err(ctx.invariant("invalid operands for checked linear algebra")),
    }
}

pub(in crate::hir_eval) fn apply(
    op: CoreOp,
    args: &[V],
    inputs: &[Type],
    eval: &mut Eval<'_>,
    span: Span,
) -> R<V> {
    use CoreOp::*;
    let mut ctx = Ctx { eval, span };
    let element = inputs.first().and_then(Type::array_element);
    let outcome = match op {
        LinEye => {
            return common(op, args, &Type::Prim(Prim::Z), &mut ctx).map_err(|fault| match fault {
                Fault::Host(control) => control,
                _ => invariant(span, "lin.eye unexpectedly produced a recoverable error"),
            })
        }
        LinDiag | LinTrace => common(
            op,
            args,
            element.ok_or_else(|| invariant(span, "checked linear algebra lacks input type"))?,
            &mut ctx,
        ),
        LinLstsq => approximate::apply(op, args, &mut ctx),
        LinDet | LinRank | LinSolve | LinInv | LinPow | LinNorm | LinFrob | LinLu | LinQr
        | LinCholesky => {
            if element == Some(&Type::Prim(Prim::R)) {
                approximate::apply(op, args, &mut ctx)
            } else {
                exact::apply(op, args, inputs, &mut ctx)
            }
        }
        _ => Err(ctx.invariant("unknown checked linear algebra operation")),
    };
    match outcome {
        Ok(value) => Ok(succeeded(value)),
        Err(Fault::Host(control)) => Err(control),
        Err(fault) => {
            let (tag, detail) = match fault {
                Fault::BadShape(detail) => ("badshape", detail),
                Fault::Singular(detail) => ("singular", detail),
                Fault::RankDeficient(detail) => ("rankdef", detail),
                Fault::NotPositiveDefinite(detail) => ("nonposdef", detail),
                Fault::BadTolerance(detail) => ("badtol", detail),
                Fault::Nonfinite(detail) => ("nonfinite", detail),
                Fault::NoConvergence(detail) => ("noconverge", detail),
                Fault::Host(_) => unreachable!(),
            };
            let value = effects::variant(ctx.eval.p, "linerr", tag, vec![], span)?;
            Ok(effects::failure(value, span, Some(detail.to_owned())))
        }
    }
}
