//! Postorder source provenance and deterministic syntax identities.

use super::{Expr, Program, Stmt, SyntaxId};
use std::{
    hash::{DefaultHasher, Hash, Hasher},
    ops::Range,
};

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Origin {
    signature: u64,
    span: Option<Range<usize>>,
}

impl Origin {
    pub(super) fn new(expr: &Expr, span: Option<Range<usize>>) -> Self {
        Self {
            signature: expression_signature(expr),
            span,
        }
    }
}
fn expression_signature(expr: &Expr) -> u64 {
    let mut hash = DefaultHasher::new();
    std::mem::discriminant(expr).hash(&mut hash);
    match expr {
        Expr::Int(n) => n.hash(&mut hash),
        Expr::Char(c) => c.hash(&mut hash),
        Expr::Bool(b) => b.hash(&mut hash),
        Expr::Unit => {}
        Expr::Name(n) | Expr::Ref(n) | Expr::Str(n) | Expr::Num(n) => n.hash(&mut hash),
        Expr::Call(name, args) => {
            name.hash(&mut hash);
            args.len().hash(&mut hash);
        }
        Expr::Tuple(es) | Expr::Array(es) | Expr::Pipeline(es) => es.len().hash(&mut hash),
        Expr::Cases(arms, _) => arms.len().hash(&mut hash),
        Expr::Raise(_) | Expr::Propagate(_) | Expr::AutoPropagate(_) => {}
        Expr::Recover(_, arms) => {
            for arm in arms {
                arm.pattern.hash(&mut hash);
                arm.start.hash(&mut hash);
            }
        }

        Expr::Match(_, arms) => {
            for arm in arms {
                arm.pattern.hash(&mut hash);
                arm.start.hash(&mut hash);
            }
        }
        Expr::Lambda(ps, _) => ps.hash(&mut hash),
        Expr::TypedLambda(ps, ts, _) => {
            ps.hash(&mut hash);
            ts.hash(&mut hash);
        }
        Expr::NamedRecord(name, fs) => {
            name.hash(&mut hash);
            for (n, _) in fs {
                n.hash(&mut hash);
            }
        }
        Expr::Record(fs) => {
            for (n, _) in fs {
                n.hash(&mut hash);
            }
        }
        Expr::Block(stmts, last)
        | Expr::Action(stmts, last)
        | Expr::Fallible(stmts, last)
        | Expr::FallibleActionBody(stmts, last) => {
            stmts.len().hash(&mut hash);
            last.is_some().hash(&mut hash);
            for stmt in stmts {
                std::mem::discriminant(stmt).hash(&mut hash);
                match stmt {
                    Stmt::Def { name, start, .. }
                    | Stmt::Schema { name, start, .. }
                    | Stmt::Sum { name, start, .. } => {
                        name.hash(&mut hash);
                        start.hash(&mut hash);
                    }
                    Stmt::Effect { name, start, .. } => {
                        name.hash(&mut hash);
                        start.hash(&mut hash);
                    }
                }
            }
        }
    }
    hash.finish()
}

impl Program {
    fn expressions_postorder(&self) -> Vec<&Expr> {
        fn collect<'p>(e: &'p Expr, nodes: &mut Vec<&'p Expr>) {
            e.for_each_child(|child| collect(child, nodes));
            nodes.push(e);
        }

        let roots = self
            .defs
            .iter()
            .filter_map(|stmt| stmt.body_and_arity().map(|(body, _)| body))
            .chain(self.entry.iter());
        let mut nodes = Vec::with_capacity(self.origins.len());
        for e in roots {
            collect(e, &mut nodes);
        }
        nodes
    }

    /// The full deterministic syntax index, including generated expressions
    /// (such as tacit expansion nodes), independent of source span availability.
    pub fn identified_expressions(&self) -> impl Iterator<Item = (SyntaxId, &Expr)> {
        self.expressions_postorder()
            .into_iter()
            .enumerate()
            .map(|(i, e)| (SyntaxId(i), e))
    }

    #[cfg(test)]
    pub(crate) fn expression_origins(&self) -> Vec<(&Expr, Range<usize>)> {
        self.identified_origins()
            .into_iter()
            .map(|(_, expr, span)| (expr, span))
            .collect()
    }

    pub(crate) fn identified_origins(&self) -> Vec<(SyntaxId, &Expr, Range<usize>)> {
        let nodes = self.expressions_postorder();
        if nodes.len() != self.origins.len()
            || nodes
                .iter()
                .zip(&self.origins)
                .any(|(node, origin)| expression_signature(node) != origin.signature)
        {
            // A programmatic AST edit invalidates the original position map.
            return Vec::new();
        }
        nodes
            .into_iter()
            .zip(&self.origins)
            .enumerate()
            .filter_map(|(i, (node, origin))| {
                origin.span.clone().map(|span| (SyntaxId(i), node, span))
            })
            .collect()
    }
}

pub(super) fn originated(
    expr: Expr,
    span: Option<Range<usize>>,
    origins: &mut Vec<Origin>,
) -> Expr {
    origins.push(Origin::new(&expr, span));
    expr
}
