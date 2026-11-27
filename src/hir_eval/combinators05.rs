//! Option and Result combinators. Callback evidence comes from checked call sites.
use super::*;

pub(super) fn handles(op: CoreOp) -> bool {
    use CoreOp::*;
    matches!(
        op,
        OptMap
            | OptBind
            | OptResult
            | ResMap
            | ResMapErr
            | ResCollect
            | ResPartition
            | ResOk
            | ResOptional
    )
}

pub(super) fn apply(
    op: CoreOp,
    operands: Vec<V>,
    types: &[Type],
    output: &Type,
    eval: &mut Eval<'_>,
    span: Span,
) -> R<V> {
    use CoreOp::*;
    eval.tick(span)?;
    if operands.len() != operation_arity(op, span)? {
        return Err(invariant(span, "invalid checked combinator arity"));
    }
    let wrong = || invariant(span, "invalid checked option/result combinator operands");
    let mut operands = operands.into_iter();
    let first = operands.next().ok_or_else(wrong)?;
    let optional = |operand| match operand {
        V::Optional(v) => Ok(v),
        _ => Err(wrong()),
    };
    let outcome = |operand| match operand {
        V::Outcome(v) => Ok(v),
        _ => Err(wrong()),
    };
    let failure = |payload| {
        V::Outcome(Err(Rc::new(Failure {
            payload,
            origin: span,
            propagations: Vec::new(),
            diagnostic: None,
        })))
    };
    let invoke = |eval: &mut Eval<'_>, f, arg: &Rc<V>| {
        // The lowered callback signature and the independently checked
        // container type normally agree. Prefer the container type when the
        // callback is still polymorphic (e.g. a passed builtin reference).
        let container_input = match (op, types.get(1)) {
            (OptMap | OptBind, Some(Type::Optional(input)))
            | (ResMap, Some(Type::Result(input, _)))
            | (ResMapErr, Some(Type::Result(_, input))) => Some(input.as_ref()),
            _ => None,
        };
        let callback_input = types.first().and_then(|sig| fun_arg(sig, 0));
        let input = container_input
            .filter(|t| !matches!(t, Type::Var(_)))
            .or(callback_input)
            .or(container_input)
            .cloned();
        let callback_output = types.first().and_then(|sig| match sig {
            Type::Fun(_, result) => Some(result.as_ref()),
            _ => None,
        });
        let result = match (op, output) {
            (OptMap, Type::Optional(result))
            | (ResMap, Type::Result(result, _))
            | (ResMapErr, Type::Result(_, result)) => Some(result.as_ref()),
            (OptBind, Type::Optional(_)) => Some(output),
            _ => None,
        };
        let result = result
            .filter(|t| !matches!(t, Type::Var(_)))
            .or(callback_output)
            .or(result);
        eval.apply_checked(
            f,
            vec![arg.as_ref().clone()],
            input.map(|t| vec![t]),
            result.cloned(),
            span,
        )
    };
    Ok(match op {
        OptMap | OptBind => {
            let Some(value) = optional(operands.next().ok_or_else(wrong)?)? else {
                return Ok(V::Optional(None));
            };
            let mapped = invoke(eval, first, &value)?;
            if op == OptMap {
                V::Optional(Some(Rc::new(mapped)))
            } else {
                V::Optional(optional(mapped)?)
            }
        }
        OptResult => match optional(operands.next().ok_or_else(wrong)?)? {
            Some(value) => V::Outcome(Ok(value)),
            None => failure(Rc::new(first)),
        },
        ResMap => match outcome(operands.next().ok_or_else(wrong)?)? {
            Ok(value) => succeeded(invoke(eval, first, &value)?),
            Err(failure) => V::Outcome(Err(failure)),
        },
        ResMapErr => match outcome(operands.next().ok_or_else(wrong)?)? {
            Ok(value) => V::Outcome(Ok(value)),
            Err(mut failure) => {
                let mapped = invoke(eval, first, &failure.payload)?;
                Rc::make_mut(&mut failure).payload = Rc::new(mapped);
                V::Outcome(Err(failure))
            }
        },
        ResCollect | ResPartition => {
            let V::Sequence(items) = first else {
                return Err(wrong());
            };
            let mut values = Vec::with_capacity(items.len());
            let mut failures = Vec::new();
            for item in items.iter() {
                eval.tick(span)?;
                match item {
                    V::Outcome(Ok(value)) => values.push(value.as_ref().clone()),
                    V::Outcome(Err(failure)) if op == ResCollect => {
                        return Ok(V::Outcome(Err(Rc::clone(failure))));
                    }
                    V::Outcome(Err(failure)) => failures.push(failure.payload.as_ref().clone()),
                    _ => return Err(wrong()),
                }
            }
            let values = V::Sequence(values.into());
            if op == ResCollect {
                succeeded(values)
            } else {
                V::Tuple(vec![values, V::Sequence(failures.into())].into())
            }
        }
        ResOk => V::Bool(outcome(first)?.is_ok()),
        ResOptional => V::Optional(outcome(first)?.ok()),
        _ => return Err(wrong()),
    })
}
