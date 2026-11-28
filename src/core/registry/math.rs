//! Mathematical primitive signatures and their catalogue entries.
use super::{Builtin, CoreOp};
use crate::types::{NominalId, Prim, Row, Type};

/// Runtime family and checked scalar signature are co-located with registry identity and arity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MathFamily {
    Elementary,
    Transcendental,
    NumberTheory,
    Polynomial,
    Numerical,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MathSignature {
    pub family: MathFamily,
    pub inputs: &'static [MathType],
    pub output: MathType,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MathType {
    N,
    Z,
    R,
    B,
    SeqN,
    SeqR,
    OptN,
    FnR,
    Options,
    SolverResult,
    T,
    SeqT,
    QuotientT,
    SeqQuotientT,
}

impl MathType {
    /// Instantiate the one homogeneous numeric parameter used by generic operations.
    pub fn instantiate(self, numeric: &Type) -> Option<Type> {
        let p = Type::Prim;
        let seq = |ty| Type::Seq(Box::new(ty));
        use MathType::*;
        Some(match self {
            N => p(Prim::N),
            Z => p(Prim::Z),
            R => p(Prim::R),
            B => p(Prim::B),
            SeqN => seq(p(Prim::N)),
            SeqR => seq(p(Prim::R)),
            OptN => Type::Optional(Box::new(p(Prim::N))),
            FnR => Type::Fun(Box::new(p(Prim::R)), Box::new(p(Prim::R))),
            Options => Type::Record(Row {
                fields: vec![("tol".into(), p(Prim::R)), ("max".into(), p(Prim::N))],
                tail: None,
            }),
            SolverResult => Type::Result(
                Box::new(p(Prim::R)),
                Box::new(Type::Nominal(NominalId {
                    name: "solvererr".into(),
                    id: crate::standard_errors::owner_id("solvererr")
                        .expect("standard solver error"),
                })),
            ),
            T => numeric.clone(),
            SeqT => seq(numeric.clone()),
            QuotientT | SeqQuotientT => {
                let domain = match numeric {
                    Type::Prim(Prim::R) => Prim::R,
                    Type::Prim(Prim::N | Prim::Z | Prim::D | Prim::Q) => Prim::Q,
                    _ => return None,
                };
                if self == SeqQuotientT {
                    seq(p(domain))
                } else {
                    p(domain)
                }
            }
        })
    }

    pub const fn numeric_parameter(self) -> bool {
        matches!(self, Self::T | Self::SeqT)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MathShape {
    Scalar,
    Sequence,
    Dense,
}

impl MathShape {
    pub fn wrap(self, ty: Type) -> Type {
        match self {
            Self::Scalar => ty,
            Self::Sequence => Type::Seq(Box::new(ty)),
            Self::Dense => Type::dense_array(ty),
        }
    }

    pub fn inner(self, ty: &Type) -> Option<&Type> {
        match (self, ty) {
            (Self::Scalar, t) => Some(t),
            (Self::Sequence, Type::Seq(elem)) => Some(elem.as_ref()),
            (Self::Dense, t) => t.array_element(),
            _ => None,
        }
    }

    pub fn of(ty: &Type) -> Self {
        if matches!(ty, Type::Seq(_)) {
            Self::Sequence
        } else if ty.array_element().is_some() {
            Self::Dense
        } else {
            Self::Scalar
        }
    }
}

macro_rules! with_math_catalog {
    ($emit:ident($($prefix:tt)*)) => {
        $emit! {
            $($prefix)*;
            MathFloor, "math.floor", Elementwise, Elementary, [T] -> Z;
            MathCeil, "math.ceil", Elementwise, Elementary, [T] -> Z;
            MathTrunc, "math.trunc", Elementwise, Elementary, [T] -> Z;
            MathRound, "math.round", Elementwise, Elementary, [T] -> Z;
            MathSign, "math.sign", Elementwise, Elementary, [T] -> Z;
            MathFrac, "math.frac", Elementwise, Elementary, [T] -> T;
            MathRecip, "math.recip", Elementwise, Elementary, [T] -> QuotientT;
            MathExp, "math.exp", Elementwise, Transcendental, [R] -> R;
            MathExpm1, "math.expm1", Elementwise, Transcendental, [R] -> R;
            MathLn, "math.ln", Elementwise, Transcendental, [R] -> R;
            MathLog10, "math.log10", Elementwise, Transcendental, [R] -> R;
            MathLog1p, "math.log1p", Elementwise, Transcendental, [R] -> R;
            MathSin, "math.sin", Elementwise, Transcendental, [R] -> R;
            MathCos, "math.cos", Elementwise, Transcendental, [R] -> R;
            MathTan, "math.tan", Elementwise, Transcendental, [R] -> R;
            MathAsin, "math.asin", Elementwise, Transcendental, [R] -> R;
            MathAcos, "math.acos", Elementwise, Transcendental, [R] -> R;
            MathAtan, "math.atan", Elementwise, Transcendental, [R] -> R;
            MathSinh, "math.sinh", Elementwise, Transcendental, [R] -> R;
            MathCosh, "math.cosh", Elementwise, Transcendental, [R] -> R;
            MathTanh, "math.tanh", Elementwise, Transcendental, [R] -> R;
            MathAsinh, "math.asinh", Elementwise, Transcendental, [R] -> R;
            MathAcosh, "math.acosh", Elementwise, Transcendental, [R] -> R;
            MathAtanh, "math.atanh", Elementwise, Transcendental, [R] -> R;
            MathRad, "math.rad", Elementwise, Transcendental, [R] -> R;
            MathDeg, "math.deg", Elementwise, Transcendental, [R] -> R;
            MathGamma, "math.gamma", Elementwise, Transcendental, [R] -> R;
            MathLgamma, "math.lgamma", Elementwise, Transcendental, [R] -> R;
            MathErf, "math.erf", Elementwise, Transcendental, [R] -> R;
            MathErfc, "math.erfc", Elementwise, Transcendental, [R] -> R;
            MathLog, "math.log", Elementwise, Transcendental, [R, R] -> R;
            MathPow, "math.pow", Elementwise, Transcendental, [R, R] -> R;
            MathHypot, "math.hypot", Elementwise, Transcendental, [R, R] -> R;
            MathAtan2, "math.atan2", Elementwise, Transcendental, [R, R] -> R;
            MathBeta, "math.beta", Elementwise, Transcendental, [R, R] -> R;
            MathRoot, "math.root", Elementwise, Transcendental, [N, R] -> R;
            MathPi, "math.pi", Scalar, Transcendental, [] -> R;
            MathE, "math.e", Scalar, Transcendental, [] -> R;
            MathTau, "math.tau", Scalar, Transcendental, [] -> R;
            MathGcd, "math.gcd", Elementwise, NumberTheory, [Z, Z] -> N;
            MathLcm, "math.lcm", Elementwise, NumberTheory, [Z, Z] -> N;
            MathFact, "math.fact", Elementwise, NumberTheory, [N] -> N;
            MathIsqrt, "math.isqrt", Elementwise, NumberTheory, [N] -> N;
            MathFib, "math.fib", Elementwise, NumberTheory, [N] -> N;
            MathChoose, "math.choose", Elementwise, NumberTheory, [N, N] -> N;
            MathPerm, "math.perm", Elementwise, NumberTheory, [N, N] -> N;
            MathModPow, "math.modpow", Scalar, NumberTheory, [N, N, N] -> N;
            MathPrime, "math.prime", Elementwise, NumberTheory, [N] -> B;
            MathFactor, "math.factor", Collection, NumberTheory, [N] -> SeqN;
            MathDivisors, "math.divisors", Collection, NumberTheory, [N] -> SeqN;
            PolyEval, "poly.eval", Collection, Polynomial, [SeqT, T] -> T;
            PolyAdd, "poly.add", Collection, Polynomial, [SeqT, SeqT] -> SeqT;
            PolySub, "poly.sub", Collection, Polynomial, [SeqT, SeqT] -> SeqT;
            PolyMul, "poly.mul", Collection, Polynomial, [SeqT, SeqT] -> SeqT;
            PolyDeriv, "poly.deriv", Collection, Polynomial, [SeqT] -> SeqT;
            PolyInteg, "poly.integ", Collection, Polynomial, [SeqT] -> SeqQuotientT;
            PolyDegree, "poly.degree", Collection, Polynomial, [SeqT] -> OptN;
            NumBisect, "num.bisect", Scalar, Numerical, [FnR, R, R, Options] -> SolverResult;
            NumNewton, "num.newton", Scalar, Numerical, [FnR, FnR, R, Options] -> SolverResult;
            NumIntegrate, "num.integrate", Scalar, Numerical, [FnR, R, R, Options] -> SolverResult;
            NumInterp, "num.interp", Scalar, Numerical, [SeqR, SeqR, R] -> SolverResult;
            NumDiff, "num.diff", Scalar, Numerical, [FnR, R, R] -> SolverResult;
        }
    };
}

macro_rules! math_metadata {
    ($actual:expr;
     $($op:ident, $name:literal, $shape:expr, $family:ident,
        [$($input:ident),*] -> $output:ident;)*) => {
        match $actual {
            $(CoreOp::$op => Some(MathSignature {
                family: MathFamily::$family,
                inputs: &[$(MathType::$input),*],
                output: MathType::$output,
            }),)*
            _ => None,
        }
    };
}

impl Builtin {
    pub(crate) fn math_signature(&self) -> Option<MathSignature> {
        with_math_catalog!(math_metadata(self.op?))
    }
}
