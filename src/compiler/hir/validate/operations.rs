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

pub(super) fn algebra_signature(
    spec: &builtins::Builtin,
    inputs: &[&Type],
    output: &Type,
    span: Span,
) -> Result<(), Error> {
    use builtins::AlgebraInput;
    if *output == Type::Never && inputs.contains(&&Type::Never) {
        return Ok(());
    }
    let signature = spec
        .algebra_signature()
        .expect("checked Algebra registry entry");
    let bad = || {
        invalid(
            span,
            format!(
                "HIR {} signature disagrees with checked types",
                spec.canonical
            ),
        )
    };
    let numeric = signature
        .inputs
        .iter()
        .zip(inputs)
        .find(|(param, _)| **param == AlgebraInput::DenseNumber)
        .map(|(_, ty)| ty.array_element().ok_or_else(bad))
        .transpose()?
        .unwrap_or(&Type::Prim(Prim::Unit));
    if signature.inputs.contains(&AlgebraInput::DenseNumber)
        && !matches!(
            numeric,
            Type::Var(_) | Type::Prim(Prim::N | Prim::Z | Prim::Q | Prim::D | Prim::R)
        )
    {
        return Err(bad());
    }
    if signature
        .inputs
        .iter()
        .zip(inputs)
        .any(|(&param, &actual)| param.instantiate(numeric) != *actual)
    {
        return Err(bad());
    }
    let expected = signature.output(numeric).ok_or_else(bad)?;
    match (&expected, output) {
        // The runtime returns factors in a fixed field order, but may be
        // referenced through a closed structural annotation with reordered fields.
        (Type::Result(value, e), Type::Result(actual, a)) if e == a => {
            if let (Type::Record(expected), Type::Record(actual)) = (&**value, &**actual) {
                if expected.tail == actual.tail
                    && expected.fields.len() == actual.fields.len()
                    && expected
                        .fields
                        .iter()
                        .all(|field| actual.fields.contains(field))
                {
                    return Ok(());
                }
            }
        }
        _ => {}
    }
    if expected != *output {
        return Err(bad());
    }
    Ok(())
}

pub(super) fn is_complex_arithmetic(op: CoreOp) -> bool {
    matches!(op, CoreOp::Add | CoreOp::Sub | CoreOp::Mul | CoreOp::Div)
}

pub(super) fn complex_arithmetic_signature(
    input: [&Type; 2],
    output: &Type,
    span: Span,
) -> Result<(), Error> {
    use crate::builtins::MathShape;
    use crate::hir::LiftMode;

    let parts = input.map(|ty| MathShape::of(ty).inner(ty).expect("one-layer shape"));
    let cx = Type::Prim(Prim::Cx);
    if !parts.contains(&&cx) && MathShape::of(output).inner(output) != Some(&cx) {
        return Ok(());
    }
    if *output == Type::Never && parts.contains(&&Type::Never) {
        return Ok(());
    }
    let bad = || {
        invalid(
            span,
            "HIR complex arithmetic requires homogeneous complex operands",
        )
    };
    if parts != [&cx, &cx] {
        return Err(bad());
    }
    let shape = match LiftMode::for_operands(input[0], Some(input[1])).map_err(|_| bad())? {
        LiftMode::Scalar => MathShape::Scalar,
        LiftMode::LeftSequence | LiftMode::RightSequence | LiftMode::Pairwise => {
            MathShape::Sequence
        }
        LiftMode::LeftArray | LiftMode::RightArray | LiftMode::ArrayPair => MathShape::Dense,
    };
    if *output != shape.wrap(cx) {
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
