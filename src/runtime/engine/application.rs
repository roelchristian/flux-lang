//! Checked application, partial application and environment specialization.
use super::{
    evidence::{callable_lift, dynamic_schema_operation, fit_instance, fun_arg, result_type},
    primitives::{operation_arity, witnessed_op},
    Eval,
};
use crate::{
    hir::Span,
    hir_eval::{error, projection, wide, Function, R, V},
    types::Type,
};
use std::{collections::HashMap, rc::Rc};

impl Eval<'_> {
    pub(in crate::hir_eval) fn apply(&mut self, f: V, args: Vec<V>, span: Span) -> R<V> {
        self.apply_checked(f, args, None, None, span)
    }
    pub(in crate::hir_eval) fn apply_unary(
        &mut self,
        f: V,
        arg: V,
        input: Type,
        output: Type,
        span: Span,
    ) -> R<V> {
        self.apply_checked(f, vec![arg], Some(vec![input]), Some(output), span)
    }

    /// Apply both stages of a curried callback, in order, without losing
    /// the checked intermediate function type at the first application.
    pub(in crate::hir_eval) fn apply_dyadic(
        &mut self,
        f: &V,
        a: (V, &Type),
        b: (V, &Type),
        out: &Type,
        span: Span,
    ) -> R<V> {
        let residual = Type::Fun(Box::new(b.1.clone()), Box::new(out.clone()));
        let partial = self.apply_unary(f.clone(), a.0, a.1.clone(), residual, span)?;
        self.apply_unary(partial, b.0, b.1.clone(), out.clone(), span)
    }

    pub(in crate::hir_eval) fn apply_checked(
        &mut self,
        f: V,
        args: Vec<V>,
        supplied_types: Option<Vec<Type>>,
        out_type: Option<Type>,
        span: Span,
    ) -> R<V> {
        self.tick(span)?;
        let V::Fun(f) = f else {
            return Err(error(span, "expected a callable function value"));
        };
        let mut f = Rc::unwrap_or_clone(f);
        let n = f.bound.len();
        let sig = match &f.code {
            Function::User { declared, .. } | Function::Core(_, declared, _) => Some(declared),
            Function::Projection(_) | Function::Constructor(..) => None,
        };
        let suggested = f.instance.as_ref();
        let out = out_type.or_else(|| suggested.map(|sig| result_type(sig, args.len()).clone()));
        for (i, actual) in supplied_types
            .unwrap_or_else(|| vec![Type::Var(u32::MAX); args.len()])
            .into_iter()
            .enumerate()
        {
            // A contextual formal expectation can be more informative than a
            // polymorphic call's supplied expression (e.g., a higher-order ref).
            let expected = suggested
                .and_then(|sig| fun_arg(sig, i))
                .or_else(|| sig.and_then(|sig| fun_arg(sig, n + i)));
            let t = if matches!(actual, Type::Var(_)) {
                expected.cloned().unwrap_or(actual)
            } else {
                actual
            };
            f.bound_types.push(t);
        }
        f.bound.extend(args);
        let arity = match &f.code {
            Function::User { formals, .. } => formals.len(),
            Function::Core(op, ..) => operation_arity(*op, span)?,
            Function::Projection(_) => 1,
            Function::Constructor(_, arity) => *arity,
        };
        if f.bound.len() < arity {
            f.instance = out;
            return Ok(V::Fun(Rc::new(f)));
        }
        if f.bound.len() > arity {
            return Err(error(span, "too many arguments for function"));
        }
        match f.code {
            Function::User {
                formals,
                body,
                env,
                declared,
                ..
            } => {
                let mut substs = HashMap::new();
                let mut row_substs = HashMap::new();
                for (i, arg) in f.bound_types.iter().enumerate() {
                    if let Some(p) = fun_arg(&declared, i) {
                        fit_instance(p, arg, &mut substs, &mut row_substs);
                    }
                }
                if let Some(out) = out.as_ref() {
                    fit_instance(
                        result_type(&declared, arity),
                        out,
                        &mut substs,
                        &mut row_substs,
                    );
                }
                let scope = self.frame(env);
                if !substs.is_empty() || !row_substs.is_empty() {
                    self.frames[scope].substs = substs;
                    self.frames[scope].row_substs =
                        (!row_substs.is_empty()).then(|| Box::new(row_substs));
                    self.frames[scope].type_scope = Some(scope);
                }
                for (s, v) in formals.iter().zip(f.bound) {
                    self.put(scope, *s, v)
                }
                self.eval(body, scope)
            }
            Function::Core(op, ty, schema) => {
                let out = out.as_ref().unwrap_or_else(|| result_type(&ty, arity));
                let value = if let Some(op) = witnessed_op(op) {
                    self.witnessed_core(op, f.bound, &f.bound_types, out, schema.as_ref(), span)?
                } else {
                    let mode = callable_lift(op, &f.bound_types).map_err(|m| error(span, m))?;
                    let instance_schema = if dynamic_schema_operation(op) {
                        self.p.instance_schema(out).or(schema)
                    } else {
                        schema
                    };
                    self.core(op, f.bound, out, instance_schema.as_ref(), mode, span)?
                };
                wide::guard_embedded(value).map_err(|m| error(span, m))
            }
            Function::Constructor(id, _) => self.make_variant(id, f.bound, span),
            Function::Projection(field) => {
                projection(&field, f.bound.pop().expect("checked arity"), span)
            }
        }
    }
}
