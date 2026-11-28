//! Real gamma, beta and error functions in the checked binary64 domain.
use super::super::{finite, CoreOp, NumericResult, Type, V};

// Stirling's correction through B14. For positive x >= 8 its error is
// bounded by the first omitted term (< 9e-16 in log space).
// NIST DLMF 5.11.1 and §5.11(ii): https://dlmf.nist.gov/5.11
fn stirling(inv_x: f64) -> f64 {
    let z = inv_x * inv_x;
    inv_x
        * (1.0 / 12.0
            + z * (-1.0 / 360.0
                + z * (1.0 / 1260.0
                    + z * (-1.0 / 1680.0
                        + z * (1.0 / 1188.0 + z * (-691.0 / 360360.0 + z / 156.0))))))
}

fn beta(x: f64, y: f64) -> NumericResult<f64> {
    if x <= 0.0 || y <= 0.0 {
        return Err("math.beta requires positive arguments");
    }
    let (a, b) = (x.min(y), x.max(y));
    if a == 1.0 {
        return Ok(b.recip());
    }
    if b == 1.0 {
        return Ok(a.recip());
    }
    // B(a,b) = Γ(a)Γ(b)/Γ(a+b), NIST DLMF 5.12.1. Direct subtraction
    // of log-gamma values loses accuracy for strongly unequal arguments
    // and overflows even where the final answer can be represented.
    if b < 8.0 {
        return Ok(libm::exp(
            libm::lgamma(a) + libm::lgamma(b) - libm::lgamma(a + b),
        ));
    }
    let ratio = a / b;
    let shift = libm::log1p(ratio);
    let inv_b = b.recip();
    let sum_log = libm::log(b) + shift;
    let corrections = stirling(inv_b) - stirling(inv_b / (1.0 + ratio));
    let log_beta = if a < 8.0 {
        // Stable difference of the two large log-gammas. Compute the
        // product b*log1p(a/b) without losing a tiny a/b to underflow.
        let product = if ratio == 0.0 { a } else { a * (shift / ratio) };
        libm::lgamma(a) - a * sum_log + (a - product) + 0.5 * shift + corrections
    } else {
        const HALF_LOG_TWO_PI: f64 = 0.918_938_533_204_672_8;
        // Total a+b may overflow, but its logarithm and reciprocal need not.
        HALF_LOG_TWO_PI + (a - 0.5) * (libm::log(ratio) - shift) - (b - 0.5) * shift - 0.5 * sum_log
            + stirling(a.recip())
            + corrections
    };
    Ok(libm::exp(log_beta))
}

pub(super) fn apply(op: CoreOp, args: &[V], _ty: &Type) -> NumericResult<V> {
    use CoreOp::*;
    finite(match (op, args) {
        (MathGamma | MathLgamma, [V::Real(x)]) if *x <= 0.0 && x.fract() == 0.0 => {
            return Err("gamma has a pole at every nonpositive integer");
        }
        (MathGamma, [V::Real(x)]) => libm::tgamma(*x),
        (MathLgamma, [V::Real(x)]) => libm::lgamma(*x),
        (MathErf, [V::Real(x)]) => libm::erf(*x),
        (MathErfc, [V::Real(x)]) => libm::erfc(*x),
        (MathBeta, [V::Real(x), V::Real(y)]) => beta(*x, *y)?,
        _ => return Err("invalid checked special-function operands"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: f64, expected: f64) {
        assert!(
            (actual / expected - 1.0).abs() < 1e-12,
            "{actual:e} != {expected:e}",
        );
    }

    #[test]
    fn beta_handles_exact_identities_underflow_and_imbalanced_arguments() {
        close(beta(0.5, 0.5).unwrap(), std::f64::consts::PI);
        close(beta(2.0, 3.0).unwrap(), 1.0 / 12.0);
        close(beta(2.0, 1e150).unwrap(), 1e-300);
        close(
            beta(1e250, 0.5).unwrap(),
            1e-125 * std::f64::consts::PI.sqrt(),
        );
        close(beta(1e-100, 1e300).unwrap(), 1e100);
        assert_eq!(beta(1e300, 1e300).unwrap(), 0.0);
        assert!(beta(0.0, 1.0).is_err());
    }
}
