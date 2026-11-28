//! Static numeric promotion used by the compiler and runtime.
use crate::builtins::Rule;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Numeric {
    N,
    Z,
    Q,
    D,
    R,
}

/// Ordinary implicit numeric joins. Approximation never silently consumes an exact operand.
pub const fn promote(a: Numeric, b: Numeric) -> Option<Numeric> {
    use Numeric::*;
    match (a, b) {
        (R, R) => Some(R),
        (R, _) | (_, R) => None,
        (Q, _) | (_, Q) => Some(Q),
        (D, _) | (_, D) => Some(D),
        (Z, _) | (_, Z) => Some(Z),
        (N, N) => Some(N),
    }
}

/// Domain rule after collection shape and operand constraints have been established.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BinaryDomain {
    Common,
    Integral,
    Division,
    Power,
}

impl BinaryDomain {
    pub(crate) const fn from_rule(rule: Rule) -> Option<Self> {
        match rule {
            Rule::Arithmetic | Rule::Order | Rule::Equality | Rule::StructuralEquality => {
                Some(Self::Common)
            }
            Rule::IntegralArithmetic | Rule::Range => Some(Self::Integral),
            Rule::Division => Some(Self::Division),
            Rule::Power => Some(Self::Power),
            _ => None,
        }
    }

    pub(crate) fn result(self, operator: &str, common: Numeric) -> Result<Numeric, String> {
        use Numeric::*;
        match (self, common) {
            (Self::Integral, N | Z) => Ok(common),
            (Self::Integral, _) => Err(format!("{operator} requires integral operands")),
            (Self::Division, R) => Ok(R),
            (Self::Division, _) => Ok(Q),
            (Self::Common | Self::Power, _) => Ok(common),
        }
    }
}
