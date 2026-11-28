//! Bounded binary64 solvers. Checked callbacks remain ordinary Flux calls.
use super::super::{
    as_seq, effects, error, exact_index, invariant, resources, succeeded, Control, CoreOp, Eval,
    Prim, Signed, Span, Type, R, V,
};

const MAX_WORK: usize = 100_000;
const REAL: Type = Type::Prim(Prim::R);

enum SolverError {
    Callback(Control),
    Failure {
        tag: &'static str,
        detail: &'static str,
    },
}

type Solve<T> = Result<T, SolverError>;

impl From<Control> for SolverError {
    fn from(control: Control) -> Self {
        Self::Callback(control)
    }
}

fn failure(tag: &'static str, detail: &'static str) -> SolverError {
    SolverError::Failure { tag, detail }
}

fn badparams(detail: &'static str) -> SolverError {
    failure("badparams", detail)
}

fn bounded(value: f64, detail: &'static str) -> Solve<f64> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(failure("nonfinite", detail))
    }
}

fn real(value: &V, span: Span) -> Solve<f64> {
    let V::Real(number) = value else {
        return Err(invariant(span, "checked numerical method expects a real value").into());
    };
    bounded(*number, "numerical method received a nonfinite value")
}

struct Options {
    tol: f64,
    max: usize,
}

impl Options {
    fn read(value: &V, span: Span) -> Solve<Self> {
        let V::Record(fields) = value else {
            return Err(invariant(span, "checked numerical method expects solver options").into());
        };
        let field = |name| {
            fields
                .iter()
                .find(|(key, _)| key.as_ref() == name)
                .map(|(_, value)| value)
                .ok_or_else(|| invariant(span, format!("checked solver options lack {name}")))
        };
        let tol = real(field("tol")?, span)?;
        let max = exact_index(field("max")?).map_err(|detail| invariant(span, detail))?;
        if tol <= 0.0 || max == 0 {
            return Err(badparams(
                "solver tolerance and work limit must both be positive",
            ));
        }
        if max > MAX_WORK {
            return Err(failure("budget", "solver work limit exceeds 100000"));
        }
        Ok(Self { tol, max })
    }
}

struct Sampler<'session, 'eval> {
    eval: &'session mut Eval<'eval>,
    span: Span,
}

impl Sampler<'_, '_> {
    fn at(&mut self, callback: &V, x: f64) -> Solve<f64> {
        // In particular, a domain failure *in* the callback is fatal; unlike
        // failures of the method, it cannot be recovered as solvererr.
        let value = self
            .eval
            .apply_unary(callback.clone(), V::Real(x), REAL, REAL, self.span)?;
        real(&value, self.span)
    }
}

fn bisect(
    samples: &mut Sampler<'_, '_>,
    f: &V,
    mut lo: f64,
    mut hi: f64,
    opts: Options,
) -> Solve<f64> {
    if lo > hi {
        return Err(failure("badbracket", "bisection bounds are reversed"));
    }
    let mut low = samples.at(f, lo)?;
    if low == 0.0 {
        return Ok(lo);
    }
    if lo == hi {
        return Err(failure("badbracket", "singleton bracket is not a root"));
    }
    let high = samples.at(f, hi)?;
    if high == 0.0 {
        return Ok(hi);
    }
    if low.is_sign_negative() == high.is_sign_negative() {
        return Err(failure("badbracket", "bisection requires a sign change"));
    }
    for step in 0..=opts.max {
        let mid = lo.midpoint(hi);
        if (mid - lo).max(hi - mid) <= opts.tol {
            return Ok(mid);
        }
        if mid == lo || mid == hi {
            return Err(failure("noconverge", "bisection cannot refine its bracket"));
        }
        if step == opts.max {
            break;
        }
        resources::check_iterations(step + 1).map_err(|e| error(samples.span, e))?;
        let value = samples.at(f, mid)?;
        if value == 0.0 {
            return Ok(mid);
        }
        if value.is_sign_negative() == low.is_sign_negative() {
            lo = mid;
            low = value;
        } else {
            hi = mid;
        }
    }
    Err(failure("noconverge", "bisection exhausted its iterations"))
}

fn newton(
    samples: &mut Sampler<'_, '_>,
    f: &V,
    derivative: &V,
    mut x: f64,
    opts: Options,
) -> Solve<f64> {
    for step in 0..=opts.max {
        let value = samples.at(f, x)?;
        if value.abs() <= opts.tol {
            return Ok(x);
        }
        if step == opts.max {
            break;
        }
        resources::check_iterations(step + 1).map_err(|e| error(samples.span, e))?;
        let slope = samples.at(derivative, x)?;
        if slope == 0.0 {
            return Err(failure("singular", "Newton derivative is zero"));
        }
        let next = newton_update(x, value, slope)?;
        if next == x {
            return Err(failure("noconverge", "Newton iterate stopped changing"));
        }
        x = next;
    }
    Err(failure("noconverge", "Newton exhausted its iterations"))
}

fn newton_update(x: f64, value: f64, slope: f64) -> Solve<f64> {
    let delta = value / slope;
    let next = if delta.is_finite() {
        x - delta
    } else {
        // x - f/df can still be finite when the quotient alone overflows.
        // Cancel x*df against f before dividing, only on this fallback.
        x.mul_add(slope, -value) / slope
    };
    bounded(next, "Newton step is nonfinite")
}

fn mid_inside(lo: f64, hi: f64) -> Solve<f64> {
    let mid = lo.midpoint(hi);
    if lo < mid && mid < hi {
        Ok(mid)
    } else {
        Err(failure(
            "noconverge",
            "adaptive integration cannot refine an interval at real precision",
        ))
    }
}

#[derive(Clone, Copy)]
struct Sample {
    x: f64,
    y: f64,
}

fn simpson([a, m, b]: [Sample; 3]) -> Solve<f64> {
    let scale = a.y.abs().max(m.y.abs()).max(b.y.abs());
    if scale == 0.0 {
        return Ok(0.0);
    }
    let mean = ((a.y / scale + b.y / scale) + 4.0 * (m.y / scale)) / 6.0;
    let width = b.x - a.x;
    let (width, gain) = if width.is_finite() {
        (width, 1.0)
    } else {
        // Compute half the width before restoring its factor of two.
        (b.x * 0.5 - a.x * 0.5, 2.0)
    };
    // Enclosed estimates can be finite even when multiplying the width by
    // the sample scale first would overflow, or scaling the mean first
    // would underflow. Pair the smallest and largest magnitudes first.
    let mut factors = [width, scale, mean];
    factors.sort_by(|a, b| a.abs().total_cmp(&b.abs()));
    bounded(
        ((factors[0] * factors[2]) * factors[1]) * gain,
        "integral estimate is nonfinite",
    )
}

struct Panel {
    points: [Sample; 3],
    estimate: f64,
    tol: f64,
}

impl Panel {
    fn new(points: [Sample; 3], tol: f64) -> Solve<Self> {
        Ok(Self {
            points,
            estimate: simpson(points)?,
            tol,
        })
    }
}

fn budget(remaining: &mut usize, maximum: usize, want: usize, span: Span) -> Solve<()> {
    *remaining = remaining
        .checked_sub(want)
        .ok_or_else(|| failure("budget", "adaptive integration exhausted its evaluations"))?;
    resources::check_iterations(maximum - *remaining).map_err(|e| error(span, e))?;
    Ok(())
}

fn integrate(samples: &mut Sampler<'_, '_>, f: &V, lo: f64, hi: f64, opts: Options) -> Solve<f64> {
    if lo == hi {
        return Ok(0.0);
    }
    let (lo, hi, orientation) = if lo < hi {
        (lo, hi, 1.0)
    } else {
        (hi, lo, -1.0)
    };
    let mut remaining = opts.max;
    let span = samples.span;
    budget(&mut remaining, opts.max, 3, span)?;
    let mid = mid_inside(lo, hi)?;
    let mut probe = |x| -> Solve<Sample> {
        Ok(Sample {
            x,
            y: samples.at(f, x)?,
        })
    };
    let mut pending = vec![Panel::new([probe(lo)?, probe(mid)?, probe(hi)?], opts.tol)?];
    let mut total = 0.0;
    while let Some(Panel {
        points: [a, m, b],
        estimate,
        tol,
    }) = pending.pop()
    {
        budget(&mut remaining, opts.max, 2, span)?;
        let x = mid_inside(a.x, m.x)?;
        let y = mid_inside(m.x, b.x)?;
        let l = probe(x)?;
        let r = probe(y)?;
        let left = Panel::new([a, l, m], tol * 0.5)?;
        let right = Panel::new([m, r, b], tol * 0.5)?;
        let sum = bounded(
            left.estimate + right.estimate,
            "integral estimate is nonfinite",
        )?;
        let delta = sum - estimate;
        let correction = if delta.is_finite() {
            delta / 15.0
        } else {
            sum / 15.0 - estimate / 15.0
        };
        if correction.abs() <= tol {
            let refined = bounded(sum + correction, "refined integral is nonfinite")?;
            total = bounded(total + refined, "accumulated integral is nonfinite")?;
        } else {
            // LIFO worklist: left before right, reusing the 3 samples
            // already known for each half.
            pending.push(right);
            pending.push(left);
        }
    }
    Ok(orientation * total)
}

fn fraction(lo: f64, x: f64, hi: f64) -> f64 {
    let span = hi - lo;
    if span.is_finite() {
        (x - lo) / span
    } else {
        (x * 0.5 - lo * 0.5) / (hi * 0.5 - lo * 0.5)
    }
}

fn interpolate(xs: &[V], ys: &[V], x: f64, span: Span) -> Solve<f64> {
    if xs.len() != ys.len() || xs.len() < 2 {
        return Err(badparams("interpolation needs at least two paired samples"));
    }
    if xs.len() > MAX_WORK {
        return Err(failure("budget", "interpolation exceeds 100000 samples"));
    }
    resources::check_iterations(xs.len()).map_err(|e| error(span, e))?;
    let mut pair = None;
    let mut previous: Option<Sample> = None;
    for (sample, value) in xs.iter().zip(ys) {
        let current = Sample {
            x: real(sample, span)?,
            y: real(value, span)?,
        };
        if let Some(last) = previous {
            if last.x >= current.x {
                return Err(badparams(
                    "interpolation x samples must be strictly increasing",
                ));
            }
            if last.x <= x && x <= current.x {
                pair = Some((last, current));
            }
        }
        previous = Some(current);
    }
    let Some((lo, hi)) = pair else {
        return Err(badparams("interpolation does not extrapolate"));
    };
    if x == lo.x {
        return Ok(lo.y);
    }
    if x == hi.x {
        return Ok(hi.y);
    }
    let t = fraction(lo.x, x, hi.x);
    let interpolated = if lo.y.is_sign_negative() == hi.y.is_sign_negative() {
        lo.y + t * (hi.y - lo.y)
    } else {
        (1.0 - t) * lo.y + t * hi.y
    };
    bounded(interpolated, "interpolation result is nonfinite")
}

fn difference(samples: &mut Sampler<'_, '_>, f: &V, x: f64, step: f64) -> Solve<f64> {
    if step <= 0.0 {
        return Err(badparams("finite-difference step must be positive"));
    }
    let lo = bounded(x - step, "finite-difference left point is nonfinite")?;
    let hi = bounded(x + step, "finite-difference right point is nonfinite")?;
    if !(lo < x && x < hi) {
        return Err(badparams(
            "finite-difference step is too small at this point",
        ));
    }
    resources::check_iterations(2).map_err(|e| error(samples.span, e))?;
    let left = samples.at(f, lo)?;
    let right = samples.at(f, hi)?;
    let dx = hi - lo;
    let dy = right - left;
    let estimate = if dx.is_finite() && dy.is_finite() {
        dy / dx
    } else {
        (right * 0.5 - left * 0.5) / (hi * 0.5 - lo * 0.5)
    };
    bounded(estimate, "finite-difference result is nonfinite")
}

fn solve(op: CoreOp, args: &[V], samples: &mut Sampler<'_, '_>) -> Solve<f64> {
    let span = samples.span;
    match (op, args) {
        (CoreOp::NumBisect | CoreOp::NumIntegrate, [f, lo, hi, opts]) => {
            let (lo, hi, opts) = (real(lo, span)?, real(hi, span)?, Options::read(opts, span)?);
            if op == CoreOp::NumBisect {
                bisect(samples, f, lo, hi, opts)
            } else {
                integrate(samples, f, lo, hi, opts)
            }
        }
        (CoreOp::NumNewton, [f, derivative, x, opts]) => newton(
            samples,
            f,
            derivative,
            real(x, span)?,
            Options::read(opts, span)?,
        ),
        (CoreOp::NumInterp, [xs, ys, x]) => {
            let seq = |v| as_seq(v).map_err(|detail| invariant(span, detail));
            interpolate(seq(xs)?, seq(ys)?, real(x, span)?, span)
        }
        (CoreOp::NumDiff, [f, x, step]) => {
            difference(samples, f, real(x, span)?, real(step, span)?)
        }
        _ => Err(invariant(span, "invalid checked numerical method operands").into()),
    }
}

pub(super) fn apply(op: CoreOp, args: &[V], eval: &mut Eval<'_>, span: Span) -> R<V> {
    match solve(op, args, &mut Sampler { eval, span }) {
        Ok(value) => Ok(succeeded(V::Real(value))),
        Err(SolverError::Callback(control)) => Err(control),
        Err(SolverError::Failure { tag, detail }) => {
            let payload = effects::variant(eval.p, "solvererr", tag, vec![], span)?;
            Ok(effects::failure(payload, span, Some(detail.to_owned())))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newton_can_cancel_an_unrepresentable_correction() {
        let x = 1.5e308;
        let value = 0.5 * x + 2.5e307;
        let next = newton_update(x, value, 0.5).ok().unwrap();
        assert!((next / -5e307 - 1.0).abs() < 2e-15);
        assert!(matches!(
            newton_update(0.0, value, 0.5),
            Err(SolverError::Failure {
                tag: "nonfinite",
                ..
            })
        ));
    }

    #[test]
    fn equation_panels_avoid_intermediate_overflow() {
        let samples = [
            Sample {
                x: 0.0,
                y: f64::MAX,
            },
            Sample {
                x: 0.5,
                y: f64::MAX,
            },
            Sample {
                x: 1.0,
                y: f64::MAX,
            },
        ];
        assert_eq!(simpson(samples).ok(), Some(f64::MAX));
        let width = [-f64::MAX, 0.0, f64::MAX];
        let samples = width.map(|x| Sample {
            x,
            y: f64::MIN_POSITIVE,
        });
        assert!((simpson(samples).ok().unwrap() - 8.0).abs() < 1e-14);
    }

    #[test]
    fn interpolation_is_bounded_and_validates_all_samples() {
        let xs = [-f64::MAX, f64::MAX].map(V::Real);
        let ys = [-f64::MAX, f64::MAX].map(V::Real);
        let span = Span::default();
        assert_eq!(interpolate(&xs, &ys, 0.0, span).ok(), Some(0.0));
        assert_eq!(interpolate(&xs, &ys, f64::MAX, span).ok(), Some(f64::MAX));
        let out_of_order = [0.0, 1.0, 0.0].map(V::Real);
        assert!(matches!(
            interpolate(&out_of_order, &out_of_order, 0.0, span),
            Err(SolverError::Failure {
                tag: "badparams",
                ..
            })
        ));
    }
}
