//! Public canonical types and the inference context.
mod context;
mod display;

pub use context::InferCtx;

use std::collections::{BTreeMap, BTreeSet};
use std::ops::ControlFlow::{self, Break, Continue};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Prim {
    N,
    Z,
    Q,
    D,
    R,
    B,
    S,
    C,
    Unit,
    Cx,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Type {
    Var(u32),
    Prim(Prim),
    Seq(Box<Type>),
    Tuple(Vec<Type>),
    Dict(Box<Type>, Box<Type>),
    Optional(Box<Type>),
    Result(Box<Type>, Box<Type>),
    /// Internal type for an expression that does not return normally.
    Never,
    Action(Box<Type>),
    Fun(Box<Type>, Box<Type>),
    Table(Box<Type>),
    Nominal(NominalId),
    /// A saturated application of a parameterized nominal sum or built-in array.
    Applied(NominalId, Vec<Type>),
    Record(Row),
}

/// Identity and user-facing spelling of a lexical schema declaration.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NominalId {
    pub id: usize,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Row {
    pub fields: Vec<(String, Type)>,
    pub tail: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Class {
    Num,
    Eq,
    Ord,
    Hash,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Constraint {
    pub class: Class,
    pub ty: Type,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scheme {
    pub vars: Vec<u32>,
    pub ty: Type,
    pub constraints: Vec<Constraint>,
    /// Quantified row IDs, with the field labels excluded from each tail.
    pub row_lacks: BTreeMap<u32, BTreeSet<String>>,
}

impl Scheme {
    pub fn mono(ty: Type) -> Self {
        Self {
            vars: vec![],
            ty,
            constraints: vec![],
            row_lacks: BTreeMap::new(),
        }
    }
}

impl Type {
    /// Canonical rank-independent dense array type.
    pub fn dense_array(element: Type) -> Self {
        Self::Applied(
            NominalId {
                id: usize::MAX,
                name: "array".into(),
            },
            vec![element],
        )
    }

    pub fn array_element(&self) -> Option<&Type> {
        match self {
            Self::Applied(id, args)
                if id.id == usize::MAX && id.name == "array" && args.len() == 1 =>
            {
                args.first()
            }
            _ => None,
        }
    }

    /// Select the first matching node in preorder, including record fields.
    pub(crate) fn find_map<'a, B>(
        &'a self,
        mut select: impl FnMut(&'a Type) -> Option<B>,
    ) -> Option<B> {
        self.walk(&mut |t| select(t).map_or(Continue(()), Break))
            .break_value()
    }

    fn walk<'a, B>(&'a self, visit: &mut impl FnMut(&'a Type) -> ControlFlow<B>) -> ControlFlow<B> {
        visit(self)?;
        match self {
            Self::Seq(t) | Self::Optional(t) | Self::Action(t) | Self::Table(t) => t.walk(visit),
            Self::Dict(a, b) | Self::Fun(a, b) | Self::Result(a, b) => {
                a.walk(visit)?;
                b.walk(visit)
            }
            Self::Tuple(ts) | Self::Applied(_, ts) => ts.iter().try_for_each(|t| t.walk(visit)),
            Self::Record(row) => row.fields.iter().try_for_each(|(_, t)| t.walk(visit)),
            Self::Var(_) | Self::Prim(_) | Self::Nominal(_) | Self::Never => Continue(()),
        }
    }

    fn visit(&self, mut visit: impl FnMut(&Type)) {
        let _: ControlFlow<()> = self.walk(&mut |t| {
            visit(t);
            Continue(())
        });
    }

    fn try_visit<E>(&self, mut visit: impl FnMut(&Type) -> Result<(), E>) -> Result<(), E> {
        self.walk(&mut |t| match visit(t) {
            Ok(()) => Continue(()),
            Err(error) => Break(error),
        })
        .break_value()
        .map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod tests;
