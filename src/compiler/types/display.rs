//! Canonical display of inferred types and qualified schemes.
use super::{Class, Constraint, Prim, Row, Scheme, Type};
use std::fmt;

impl fmt::Display for Prim {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::N => "n",
            Self::Z => "z",
            Self::Q => "q",
            Self::D => "d",
            Self::R => "r",
            Self::B => "b",
            Self::S => "s",
            Self::C => "c",
            Self::Unit => "()",
            Self::Cx => "cx",
        })
    }
}
impl fmt::Display for Class {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Num => "num",
            Self::Eq => "eq",
            Self::Ord => "ord",
            Self::Hash => "hash",
        })
    }
}
impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn operand(t: &Type, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            if matches!(t, Type::Fun(..)) {
                write!(f, "({t})")
            } else {
                write!(f, "{t}")
            }
        }
        fn result_operand(t: &Type, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            if matches!(t, Type::Fun(..) | Type::Result(..)) {
                write!(f, "({t})")
            } else {
                write!(f, "{t}")
            }
        }
        match self {
            Self::Var(v) => write!(f, "~t{v}"),
            Self::Prim(p) => write!(f, "{p}"),
            Self::Seq(t) => write!(f, "[{t}]"),
            Self::Tuple(ts) => {
                f.write_str("(")?;
                for (i, t) in ts.iter().enumerate() {
                    if i != 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{t}")?;
                }
                if ts.len() == 1 {
                    f.write_str(",")?;
                }
                f.write_str(")")
            }
            Self::Dict(k, v) => {
                f.write_str("[")?;
                operand(k, f)?;
                f.write_str(":")?;
                operand(v, f)?;
                f.write_str("]")
            }
            Self::Optional(t) => {
                f.write_str("?")?;
                result_operand(t, f)
            }
            Self::Result(ok, err) => {
                result_operand(ok, f)?;
                f.write_str("^")?;
                result_operand(err, f)
            }
            Self::Never => f.write_str("<never>"),
            Self::Action(t) => {
                f.write_str("!")?;
                result_operand(t, f)
            }
            Self::Fun(a, b) => {
                operand(a, f)?;
                write!(f, "->{b}")
            }
            Self::Table(t) => {
                f.write_str("#")?;
                result_operand(t, f)
            }
            Self::Nominal(n) => f.write_str(crate::visible_symbol(&n.name)),
            Self::Applied(n, args) => {
                write!(f, "{}[", crate::visible_symbol(&n.name))?;
                for (i, t) in args.iter().enumerate() {
                    if i != 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{t}")?;
                }
                f.write_str("]")
            }
            Self::Record(row) => write!(f, "{row}"),
        }
    }
}
impl fmt::Display for Row {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(".{")?;
        for (i, (n, t)) in self.fields.iter().enumerate() {
            if i != 0 {
                f.write_str(" ")?;
            }
            write!(f, "{n}:{t};")?;
        }
        if let Some(id) = self.tail {
            if !self.fields.is_empty() {
                f.write_str(" ")?;
            }
            write!(f, "| ~r{id}")?;
        }
        f.write_str("}")
    }
}
impl fmt::Display for Constraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.class, self.ty)
    }
}
impl fmt::Display for Scheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.ty)?;
        for (i, c) in self.constraints.iter().enumerate() {
            if i == 0 {
                f.write_str(" | ")?;
            } else {
                f.write_str(", ")?;
            }
            write!(f, "{c}")?;
        }
        Ok(())
    }
}
