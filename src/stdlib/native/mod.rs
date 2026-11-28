//! Dispatch typed text, calendar and host operations to their native domains.
use super::{effects, Action, CoreOp, Eval, HirProgram, Span, R, V};
use std::rc::Rc;

mod calendar;
mod host;
mod regex;
mod text;

const TEXT_LIMIT: usize = 8 * 1024 * 1024;

fn string(s: impl AsRef<str>) -> V {
    V::String(Rc::from(s.as_ref()))
}

fn optional(v: Option<V>) -> V {
    V::Optional(v.map(Rc::new))
}

fn sequence(items: Vec<V>) -> V {
    V::Sequence(items.into())
}

fn typed_failure(
    program: &HirProgram,
    owner: &str,
    tag: &str,
    payload: &str,
    detail: impl Into<String>,
    span: Span,
) -> R<V> {
    Ok(effects::failure(
        effects::variant(program, owner, tag, vec![string(payload)], span)?,
        span,
        Some(detail.into()),
    ))
}

pub(super) fn apply_core(op: CoreOp, args: &[V], span: Span, program: &HirProgram) -> Option<R<V>> {
    use CoreOp::*;
    Some(match op {
        ReadLine | FsList | FsMkdir | SysArgs | SysPid | SysEnv | FsBase | FsJoin => {
            host::construct(op, args, span)
        }
        StrUpper | StrLower | StrTrim | StrSplit | StrJoin => text::apply(op, args, span),
        RxFind => regex::apply(args, program, span),
        DtDays | DtDate => calendar::apply(op, args, program, span),
        _ => return None,
    })
}

pub(super) fn apply_action(action: &Action, eval: &mut Eval<'_>, span: Span) -> Option<R<V>> {
    let Action::Library(op, args) = action else {
        return None;
    };
    Some(host::execute(
        *op, args, eval.p, eval.stdin, eval.args, span,
    ))
}
