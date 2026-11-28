//! Static operation signatures and schema witnesses.
use super::{builtins, invalid, CoreOp, Error, HirProgram, Prim, Span, TableSchema, Type};
use std::rc::Rc;

pub(super) fn mathesis_signature(
    spec: &builtins::Builtin,
    inputs: &[&Type],
    output: &Type,
    span: Span,
) -> Result<(), Error> {
    use builtins::MathType;
    let signature = spec
        .math_signature()
        .expect("checked Mathesis registry entry");
    if *output == Type::Never && inputs.contains(&&Type::Never) {
        return Ok(());
    }
    let (shape, elements) = spec
        .math_operands(inputs)
        .map_err(|reason| invalid(span, reason))?;
    let numeric = elements
        .iter()
        .zip(signature.inputs)
        .find_map(|(&ty, param)| match (param, ty) {
            (MathType::T, ty) => Some(ty),
            (MathType::SeqT, Type::Seq(item)) => Some(item.as_ref()),
            _ => None,
        })
        .unwrap_or(&Type::Prim(Prim::Unit));
    let bad = || {
        invalid(
            span,
            format!(
                "HIR {} signature disagrees with checked types",
                spec.canonical
            ),
        )
    };
    if signature.inputs.iter().any(|p| p.numeric_parameter())
        && !matches!(
            numeric,
            Type::Var(_) | Type::Prim(Prim::N | Prim::Z | Prim::Q | Prim::D | Prim::R)
        )
    {
        return Err(bad());
    }
    for (&actual, &param) in elements.iter().zip(signature.inputs) {
        let expected = param.instantiate(numeric).ok_or_else(bad)?;
        if param == MathType::Options {
            let (Type::Record(actual), Type::Record(expected)) = (actual, &expected) else {
                return Err(bad());
            };
            if actual.tail.is_some()
                || actual.fields.len() != expected.fields.len()
                || !expected
                    .fields
                    .iter()
                    .all(|field| actual.fields.contains(field))
            {
                return Err(bad());
            }
        } else if *actual != expected {
            return Err(bad());
        }
    }
    if signature
        .output
        .instantiate(numeric)
        .map(|ty| shape.wrap(ty))
        .as_ref()
        != Some(output)
    {
        return Err(bad());
    }
    Ok(())
}

pub(super) fn layout_matches(p: &HirProgram, mut ty: &Type, witness: Option<&TableSchema>) -> bool {
    while let Type::Action(out) | Type::Result(out, _) | Type::Fun(_, out) = ty {
        ty = out;
    }
    if let Type::Table(row) = ty {
        match row.as_ref() {
            Type::Record(row) if row.tail.is_none() => {
                return matches!(witness, Some(TableSchema::Named(fields)) if fields == &row.fields);
            }
            Type::Tuple(row) => {
                return matches!(witness, Some(TableSchema::Positional(fields)) if fields == row);
            }
            Type::Nominal(id) => return p.nominal_schemas.get(id).map(Rc::as_ref) == witness,
            _ => {}
        }
    }
    witness.is_none()
}

pub(super) fn is_new_dense_operation(op: CoreOp) -> bool {
    use CoreOp::*;
    matches!(
        op,
        ArrayForm
            | ArrayAxes
            | ArraySection
            | ArrayReduce
            | ArrayFold
            | ArrayScanAxis
            | ArrayDot
            | ArrayMatMul
            | ArrayDiag
    )
}

// Check the collection kinds used for both direct and first-class dispatch.
// Ranks, dimensions and promotion have their own runtime and inference rules.
pub(super) fn dense_signature(
    op: CoreOp,
    inputs: &[&Type],
    output: &Type,
    at: Span,
) -> Result<(), Error> {
    use CoreOp::*;
    if *output == Type::Never {
        return Ok(());
    }
    let natural = |ty: &Type| *ty == Type::Prim(Prim::N);
    let dimensions = |ty: &Type| matches!(ty, Type::Seq(n) if natural(n));
    let numeric = |ty: &Type| {
        matches!(
            ty,
            Type::Var(_) | Type::Prim(Prim::N | Prim::Z | Prim::Q | Prim::D | Prim::R)
        )
    };
    let array = |ty: &Type| ty.array_element().is_some();
    let checked = match (op, inputs) {
        (ArrayForm, [shape, items]) => {
            dimensions(shape) && (matches!(items, Type::Seq(_)) || array(items)) && array(output)
        }
        (ArrayAxes, [order, input]) => dimensions(order) && array(input) && array(output),
        (ArraySection, [starts, counts, input]) => {
            dimensions(starts) && dimensions(counts) && array(input) && array(output)
        }
        (ArrayReduce, [axis, _, input]) => {
            natural(axis)
                && array(input)
                && matches!(output.array_element(), Some(Type::Optional(_)))
        }
        (ArrayFold | ArrayScanAxis, [axis, _, _, input]) => {
            natural(axis) && array(input) && array(output)
        }
        (ArrayDot | ArrayMatMul, [left, right]) => {
            left.array_element().is_some_and(numeric)
                && right.array_element().is_some_and(numeric)
                && if op == ArrayDot {
                    numeric(output)
                } else {
                    output.array_element().is_some_and(numeric)
                }
        }
        (ArrayDiag, [input]) => array(input) && array(output),
        _ => false,
    };
    if !checked {
        return Err(invalid(
            at,
            "HIR dense-array operation disagrees with its checked signature",
        ));
    }
    Ok(())
}
