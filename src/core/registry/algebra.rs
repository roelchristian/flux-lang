//! Linear algebra's canonical operation names, arities and checked signatures.
use super::{Builtin, CoreOp};
use crate::types::{NominalId, Prim, Row, Type};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AlgebraInput {
    Natural,
    Real,
    Complex,
    DenseNumber,
    DenseReal,
}

impl AlgebraInput {
    pub(crate) fn instantiate(self, numeric: &Type) -> Type {
        match self {
            Self::Natural => Type::Prim(Prim::N),
            Self::Real => Type::Prim(Prim::R),
            Self::Complex => Type::Prim(Prim::Cx),
            Self::DenseNumber => Type::dense_array(numeric.clone()),
            Self::DenseReal => Type::dense_array(Type::Prim(Prim::R)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AlgebraOutput {
    Eye,
    Same,
    SameArray,
    Real,
    Rank,
    Determinant,
    QuotientArray,
    RealArray,
    Lu,
    Qr,
    Complex,
    ComplexComponent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AlgebraSignature {
    pub(crate) inputs: &'static [AlgebraInput],
    output: AlgebraOutput,
}

impl AlgebraSignature {
    pub(crate) fn fallible(self) -> bool {
        !matches!(self.output, AlgebraOutput::Eye) && !self.complex()
    }

    fn complex(self) -> bool {
        matches!(
            self.output,
            AlgebraOutput::Complex | AlgebraOutput::ComplexComponent
        )
    }

    pub(crate) fn domain_dependent(self) -> bool {
        matches!(
            self.output,
            AlgebraOutput::Determinant | AlgebraOutput::QuotientArray
        )
    }

    pub(crate) fn contextual_numeric(self, target: &Type) -> Option<&Type> {
        let Type::Result(value, _) = target else {
            return None;
        };
        match self.output {
            AlgebraOutput::Same => Some(value.as_ref()),
            AlgebraOutput::SameArray => value.array_element(),
            _ => None,
        }
    }

    pub(crate) fn output(self, numeric: &Type) -> Option<Type> {
        use AlgebraOutput::*;
        let p = Type::Prim;
        let array = Type::dense_array;
        let real_array = || array(p(Prim::R));
        let record = |fields: Vec<(&str, Type)>| {
            Type::Record(Row {
                fields: fields
                    .into_iter()
                    .map(|(name, t)| (name.to_owned(), t))
                    .collect(),
                tail: None,
            })
        };
        let value = match self.output {
            Eye => return Some(array(p(Prim::Z))),
            Complex => return Some(p(Prim::Cx)),
            ComplexComponent => return Some(p(Prim::R)),
            Same => numeric.clone(),
            SameArray => array(numeric.clone()),
            Real => p(Prim::R),
            Rank => p(Prim::N),
            Determinant => p(match numeric {
                Type::Prim(Prim::N | Prim::Z) => Prim::Z,
                Type::Prim(Prim::Q | Prim::D) => Prim::Q,
                Type::Prim(Prim::R) => Prim::R,
                _ => return None,
            }),
            QuotientArray => array(p(match numeric {
                Type::Prim(Prim::N | Prim::Z | Prim::Q | Prim::D) => Prim::Q,
                Type::Prim(Prim::R) => Prim::R,
                _ => return None,
            })),
            RealArray => real_array(),
            Lu => record(vec![
                ("p", Type::Seq(Box::new(p(Prim::N)))),
                ("l", real_array()),
                ("u", real_array()),
            ]),
            Qr => record(vec![("q", real_array()), ("r", real_array())]),
        };
        Some(Type::Result(
            Box::new(value),
            Box::new(Type::Nominal(NominalId {
                name: "linerr".to_owned(),
                id: crate::standard_errors::owner_id("linerr").expect("standard linear error"),
            })),
        ))
    }
}

macro_rules! with_algebra_catalog {
    ($emit:ident($($prefix:tt)*)) => {
        $emit! {
            $($prefix)*;
            LinEye, "lin.eye", Collection, [Natural] -> Eye;
            LinDiag, "lin.diag", Collection, [DenseNumber] -> SameArray;
            LinTrace, "lin.trace", Reduction, [DenseNumber] -> Same;
            LinNorm, "lin.norm", Reduction, [DenseReal] -> Real;
            LinFrob, "lin.frob", Reduction, [DenseReal] -> Real;
            LinDet, "lin.det", Reduction, [DenseNumber] -> Determinant;
            LinRank, "lin.rank", Reduction, [DenseNumber] -> Rank;
            LinSolve, "lin.solve", Collection, [DenseNumber, DenseNumber] -> QuotientArray;
            LinInv, "lin.inv", Collection, [DenseNumber] -> QuotientArray;
            LinPow, "lin.pow", Collection, [DenseNumber, Natural] -> SameArray;
            LinLu, "lin.lu", Collection, [DenseReal] -> Lu;
            LinQr, "lin.qr", Collection, [DenseReal] -> Qr;
            LinCholesky, "lin.cholesky", Collection, [DenseReal] -> RealArray;
            LinLstsq, "lin.lstsq", Collection, [Real, DenseReal, DenseReal] -> RealArray;
            CxMake, "cx.make", Scalar, [Real, Real] -> Complex;
            CxRe, "cx.re", Scalar, [Complex] -> ComplexComponent;
            CxIm, "cx.im", Scalar, [Complex] -> ComplexComponent;
            CxConj, "cx.conj", Scalar, [Complex] -> Complex;
            CxAbs, "cx.abs", Scalar, [Complex] -> ComplexComponent;
            CxArg, "cx.arg", Scalar, [Complex] -> ComplexComponent;
        }
    };
}

macro_rules! algebra_metadata {
    ($actual:expr;
     $($op:ident, $name:literal, $shape:expr, [$($input:ident),*] -> $output:ident;)*) => {
        match $actual {
            $(CoreOp::$op => Some(AlgebraSignature {
                inputs: &[$(AlgebraInput::$input),*],
                output: AlgebraOutput::$output,
            }),)*
            _ => None,
        }
    }
}

impl CoreOp {
    fn algebra_signature(self) -> Option<AlgebraSignature> {
        with_algebra_catalog!(algebra_metadata(self))
    }

    pub(crate) fn is_linear_algebra(self) -> bool {
        self.algebra_signature().is_some_and(|sig| !sig.complex())
    }
}

impl Builtin {
    pub(crate) fn algebra_signature(&self) -> Option<AlgebraSignature> {
        self.op?.algebra_signature()
    }
}
