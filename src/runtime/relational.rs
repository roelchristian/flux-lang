//! Execution proofs subordinate to validated HIR. Never pushes a callback
//! across another eager table operation, even when both are pure.
use super::{resources, Eval, Function, Rc, Span, Table, TableSchema, R, V};
use crate::hir::{ExprKind, Target};

pub(super) mod grouping;

struct Projection<'a> {
    slots: Vec<&'a str>,
    target: Rc<TableSchema>,
    /// Number of original evaluator ticks per row.
    work: usize,
    depth: u16,
}

impl<'a> Projection<'a> {
    fn lower(
        eval: &Eval<'a>,
        callback: &V,
        table: &Table,
        target: &Rc<TableSchema>,
    ) -> Option<Self> {
        let V::Fun(fun) = callback else { return None };
        if !fun.bound.is_empty() {
            return None;
        }
        let Function::User { formals, body, .. } = &fun.code else {
            return None;
        };
        let [row] = formals.as_ref() else { return None };
        let (TableSchema::Named(source), TableSchema::Named(output)) =
            (table.schema().as_ref(), target.as_ref())
        else {
            return None;
        };
        let ExprKind::Record(fields) = &eval.p.node(*body).kind else {
            return None;
        };
        if fields.len() != output.len() {
            return None;
        }

        let mut slots = Vec::with_capacity(fields.len());
        for ((label, expression), (output_label, output_type)) in fields.iter().zip(output) {
            if label.as_ref() != output_label {
                return None;
            }
            let node = eval.p.node(*expression);
            let ExprKind::Call(Target::Projection(field), operands) = &node.kind else {
                return None;
            };
            let [operand] = operands.as_slice() else {
                return None;
            };
            if !matches!(&eval.p.node(*operand).kind, ExprKind::Load(id) if id == row) {
                return None;
            }
            // No conversions, captures, open-row lookups, or other
            // computations may be inferred from physical column storage.
            if node.ty != *output_type
                || !source
                    .iter()
                    .any(|(name, ty)| name.as_str() == field.as_ref() && ty == output_type)
            {
                return None;
            }
            slots.push(field.as_ref());
        }
        let work = fields.len().checked_mul(2)?.checked_add(2)?;
        Some(Self {
            slots,
            target: target.clone(),
            work,
            depth: if fields.is_empty() { 1 } else { 3 },
        })
    }

    fn run(self, eval: &mut Eval<'_>, table: &Table, span: Span) -> Option<R<V>> {
        let work = table.len().checked_mul(self.work)?;
        // Leave the original loop in charge of the exact first failing
        // source span when depth, work or physical layout cannot be admitted.
        if work > eval.fuel || eval.depth.checked_add(self.depth)? > eval.max_depth {
            return None;
        }
        let result = table.try_project_columns(self.target, &self.slots)?;
        Some(eval.charge_work(work, span).map(|()| result))
    }
}

pub(super) fn project(
    eval: &mut Eval<'_>,
    callback: &V,
    table: &Table,
    target: &Rc<TableSchema>,
    span: Span,
) -> Option<R<V>> {
    if !resources::optimized() || !table.columnar() {
        return None;
    }
    Projection::lower(eval, callback, table, target)?.run(eval, table, span)
}
