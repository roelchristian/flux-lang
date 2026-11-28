//! Contextual inference for native optional and result combinators.
use super::super::{f, o, p, result, s, Engine, Env, R};
use crate::parser::Expr;
use crate::types::{Prim, Type};

impl Engine<'_> {
    pub(super) fn native_combinator(
        &mut self,
        env: &Env,
        op: &str,
        args: &[Expr],
        want: Option<&Type>,
        effect: bool,
    ) -> R<Type> {
        use Prim::*;
        let wanted = want.map(|w| self.cx.resolve(w));
        let result_parts = match &wanted {
            Some(Type::Result(ok, err)) => Some((ok.as_ref().clone(), err.as_ref().clone())),
            _ => None,
        };
        let optional_part = match &wanted {
            Some(Type::Optional(t)) => Some(t.as_ref().clone()),
            _ => None,
        };
        let fresh_pair = |this: &mut Self| (this.cx.fresh(), this.cx.fresh());
        let (input, output, callback, which) = match op {
            "opt.map" | "opt.bind" => {
                let item = self.cx.fresh();
                let value = optional_part.unwrap_or_else(|| self.cx.fresh());
                let mapped = if op == "opt.bind" {
                    o(value.clone())
                } else {
                    value.clone()
                };
                (o(item.clone()), o(value), Some(f(item, mapped)), 1)
            }
            "opt.result" => {
                let (ok, err) = result_parts.unwrap_or_else(|| fresh_pair(self));
                let er = self.expr(env, &args[0], Some(&err), effect)?;
                let value = self.expr(env, &args[1], Some(&o(ok.clone())), effect)?;
                return Ok(if er == Type::Never || value == Type::Never {
                    Type::Never
                } else {
                    result(ok, err)
                });
            }
            "res.map" => {
                let (mapped, err) = result_parts.unwrap_or_else(|| fresh_pair(self));
                let item = self.cx.fresh();
                (
                    result(item.clone(), err.clone()),
                    result(mapped.clone(), err),
                    Some(f(item, mapped)),
                    1,
                )
            }
            "res.maperr" => {
                let (item, mapped) = result_parts.unwrap_or_else(|| fresh_pair(self));
                let err = self.cx.fresh();
                (
                    result(item.clone(), err.clone()),
                    result(item, mapped.clone()),
                    Some(f(err, mapped)),
                    1,
                )
            }
            "res.collect" => {
                let (item, err) = match result_parts {
                    Some((Type::Seq(item), err)) => (*item, err),
                    _ => fresh_pair(self),
                };
                (
                    s(result(item.clone(), err.clone())),
                    result(s(item), err),
                    None,
                    0,
                )
            }
            "res.partition" => {
                let (item, err) = match wanted {
                    Some(Type::Tuple(parts)) => match parts.as_slice() {
                        [Type::Seq(a), Type::Seq(e)] => (a.as_ref().clone(), e.as_ref().clone()),
                        _ => fresh_pair(self),
                    },
                    _ => fresh_pair(self),
                };
                (
                    s(result(item.clone(), err.clone())),
                    Type::Tuple(vec![s(item), s(err)]),
                    None,
                    0,
                )
            }
            "res.ok" => {
                let (item, err) = fresh_pair(self);
                (result(item, err), p(B), None, 0)
            }
            "res.optional" => {
                let item = optional_part.unwrap_or_else(|| self.cx.fresh());
                let err = self.cx.fresh();
                (result(item.clone(), err), o(item), None, 0)
            }
            _ => unreachable!(),
        };
        let callback_first = callback.is_some()
            && (self.known_callee(env, &args[0])
                || matches!(&args[0],Expr::TypedLambda(_,ts,_) if ts.iter().all(Option::is_some)));
        let mut bottom = false;
        if callback_first {
            bottom |= self.expr(env, &args[0], callback.as_ref(), effect)? == Type::Never;
        }
        bottom |= self.expr(env, &args[which], Some(&input), effect)? == Type::Never;
        if callback.is_some() && !callback_first {
            bottom |= self.expr(env, &args[0], callback.as_ref(), effect)? == Type::Never;
        }
        Ok(if bottom { Type::Never } else { output })
    }
}
