//! Recoverable outcomes, lexical transfers and fatal runtime diagnostics.
use crate::hir_eval::{repr, V};
use crate::{
    diagnostic::{Category, Diagnostic, Related},
    hir::{BoundaryId, HirProgram, Span},
    types::Type,
    Error,
};
use std::rc::Rc;

pub(in crate::hir_eval) type R<T> = Result<T, Control>;
#[derive(Clone)]
pub(in crate::hir_eval) struct Failure {
    pub(in crate::hir_eval) payload: Rc<V>,
    pub(in crate::hir_eval) origin: Span,
    pub(in crate::hir_eval) propagations: Vec<Span>,
    pub(in crate::hir_eval) diagnostic: Option<String>,
}
impl Failure {
    pub(in crate::hir_eval) fn new(
        payload: Rc<V>,
        origin: Span,
        diagnostic: Option<String>,
    ) -> Rc<Self> {
        Rc::new(Self {
            payload,
            origin,
            propagations: Vec::new(),
            diagnostic,
        })
    }
}
pub(in crate::hir_eval) enum Control {
    Fatal(Error),
    Raise {
        boundary: BoundaryId,
        failure: Rc<Failure>,
    },
}
impl Control {
    pub(in crate::hir_eval) fn into_error(self) -> Error {
        match self {
            Self::Fatal(e) => e,
            Self::Raise { failure, .. } => Diagnostic::new(
                Category::RuntimeInvariant,
                failure.origin,
                "checked caret transfer escaped its lexical boundary",
            )
            .into_error(),
        }
    }
}
pub(in crate::hir_eval) fn succeeded(value: V) -> V {
    V::Outcome(Ok(Rc::new(value)))
}

pub(in crate::hir_eval) fn error(span: Span, message: impl Into<String>) -> Control {
    Control::Fatal(Diagnostic::new(Category::RuntimeDomain, span, message).into_error())
}
pub(in crate::hir_eval) fn invariant(span: Span, message: impl Into<String>) -> Control {
    Control::Fatal(Diagnostic::new(Category::RuntimeInvariant, span, message).into_error())
}
pub(in crate::hir_eval) fn uncaught(program: &HirProgram, f: &Failure) -> Error {
    let declared = program.entry.and_then(|id| {
        let ty = &program.node(id).ty;
        let Type::Action(value) = ty else { return None };
        let Type::Result(_, err) = value.as_ref() else {
            return None;
        };
        Some(err.as_ref())
    });
    let shown = declared
        .map(|t| format!("{t}: {}", repr(&f.payload)))
        .unwrap_or_else(|| repr(&f.payload));
    let mut d = Diagnostic::new(
        Category::RuntimeOperational,
        f.origin,
        format!("uncaught operational failure: {shown}"),
    );
    if let Some(detail) = &f.diagnostic {
        d.related.push(Related {
            span: f.origin,
            label: detail.clone(),
        });
    }
    for &at in &f.propagations {
        d.related.push(Related {
            span: at,
            label: "propagated here".into(),
        });
    }
    d.into_error()
}
