//! Linear algebra type witnesses. Shape remains an evaluation-time property.
use super::super::{Engine, Env, R};
use crate::{
    builtins::{AlgebraInput, Builtin},
    parser::Expr,
    types::{Class, Type},
};

impl Engine<'_> {
    pub(super) fn algebra_call(
        &mut self,
        env: &Env,
        spec: &Builtin,
        args: &[Expr],
        want: Option<&Type>,
        effect: bool,
    ) -> R<Type> {
        let sig = spec
            .algebra_signature()
            .expect("registry algebra signature");
        let numeric = self.cx.fresh();
        // A shape/domain-preserving annotation can guide the common input
        // type. A many-to-one result domain does not determine input precision.
        if let Some(wanted) = want.map(|t| self.cx.resolve(t)) {
            if let Some(target) = sig.contextual_numeric(&wanted) {
                self.unify(&numeric, target)?;
            }
        }
        let mut inputs = Vec::with_capacity(args.len());
        for (&param, arg) in sig.inputs.iter().zip(args) {
            inputs.push(self.expr(env, arg, Some(&param.instantiate(&numeric)), effect)?);
        }
        if inputs.contains(&Type::Never) {
            return Ok(Type::Never);
        }
        self.algebra_types(spec, &inputs)
    }

    pub(super) fn algebra_types(&mut self, spec: &Builtin, inputs: &[Type]) -> R<Type> {
        let sig = spec
            .algebra_signature()
            .expect("registry algebra signature");
        let numeric = self.cx.fresh();
        for (&param, actual) in sig.inputs.iter().zip(inputs) {
            self.unify(actual, &param.instantiate(&numeric))?;
        }
        if sig.inputs.contains(&AlgebraInput::DenseNumber) {
            self.req(Class::Num, &numeric)?;
        }
        let mut domain = self.cx.resolve(&numeric);
        if sig.domain_dependent() && matches!(domain, Type::Var(_)) {
            // Default a literal's legal unresolved domain just as ordinary
            // exact division does. Do not default users' polymorphic types.
            self.require_exact_numeric(&numeric, spec.canonical)?;
            domain = self.cx.resolve(&numeric);
        }
        sig.output(&domain)
            .ok_or_else(|| self.err(format!("{} needs a known numeric domain", spec.canonical)))
    }
}

#[cfg(test)]
mod tests {
    use crate::semantic::{prepare, Mode, Policy};

    #[test]
    fn algebra_inference_carries_domain_evidence_across_call_styles() {
        let source = r#"
            << "lin" : la;
            eye:array[z] := la.eye 2;
            iz:array[z] := form [1 1] [2];
            in0:array[n] := form [1 1] [2];
            id0:array[d] := form [1 1] [2.50d];
            ir:array[r] := form [1 1] [2.5r];
            signed:z^linerr := la.det in0;
            rr:r^linerr := la.det ir;
            defaulted:z^linerr := la.det (form [1 1] [2]);
            exact:q^linerr := la.det id0;
            exact_inv:array[q]^linerr := la.inv iz;
            approximate_inv:array[r]^linerr := la.inv ir;
            tr := &la.trace;
            trd:d^linerr := @ tr id0;
            trr:r^linerr := @ tr ir;
            ranker := &{ la.rank };
            rankz:n^linerr := @ ranker iz;
            ranked:n^linerr := @ ranker ir;
            fa:array[d]->q^linerr := &la.det;
            fb:array[r]->array[r]->array[r]^linerr := &la.solve;
            solved:array[r]^linerr := @@ fb (ir,ir);
            factor:array[r]->.{r:array[r];q:array[r];}^linerr := &la.qr;
            grouped := @ factor ir;
            empty:q^linerr := la.trace (form [0 0] []);
            length:r^linerr := la.norm (form [2] [3 4]);
            powered:array[n]^linerr := la.pow in0 3;
            recovered:q := ^= exact {
                badshape : 1q;
                _ : 0q;
            };
        "#;
        let program = crate::parser::parse(source).unwrap();
        prepare(&program, Mode::Script, Policy::TypedOnly).unwrap();
    }

    #[test]
    fn algebra_and_solver_error_names_resolve_to_their_own_nominal_types() {
        for imports in [
            r#"<< "num" : nu; << "lin" : la;"#,
            r#"<< "lin" : la; << "num" : nu;"#,
        ] {
            let source = format!(
                r#"{imports}
                    m:array[r] := form [1 1] [0r];
                    inverse := la.inv m;
                    replaced := ^= inverse {{
                        la.singular : m;
                        _ : m;
                    }};
                    failed:r^solvererr := ^{{ ^: nu.singular }};
                    old:r := ^= failed {{
                        singular : 1r;
                        _ : 0r;
                    }};
                    new:z^linerr := ^{{ ^: la.nonfinite }};
                "#
            );
            let program = crate::parser::parse(&source).unwrap();
            prepare(&program, Mode::Script, Policy::TypedOnly).unwrap();
        }
    }

    #[test]
    fn algebra_rejects_forged_source_annotations_and_undefined_domains() {
        for (source, diagnostic) in [
            (
                r#"<< "lin"; m := form [1 1] [1q]; x := lin.norm m;"#,
                "cannot unify",
            ),
            (
                r#"<< "lin"; a := form [1 1] [1n]; b := form [1] [1q]; x := lin.solve a b;"#,
                "cannot unify",
            ),
            (
                r#"<< "lin"; m := form [1 1] [1r]; p := lin.pow m 1z;"#,
                "cannot unify",
            ),
            (r#"<< "lin"; t := &lin.det;"#, "ambiguous lin.det"),
            (
                r#"<< "lin"; d:array[d]->d^linerr := &lin.det;"#,
                "cannot unify",
            ),
            (
                r#"<< "lin"; x := lin.trace (form [1 1] ["non-numeric"]);"#,
                "does not satisfy num",
            ),
            (
                r#"<< "lin"; t:array[r]->r := &lin.frob;"#,
                "fallible result",
            ),
        ] {
            let program = crate::parser::parse(source).unwrap();
            let resolved = crate::resolve::resolve(&program).unwrap();
            let err = crate::infer::infer(&program, &resolved).unwrap_err();
            assert!(err.message.contains(diagnostic), "{source}: {err}");
        }
    }
    #[test]
    fn complex_requires_explicit_construction_and_has_only_equality() {
        let source = r#"
            << "cx";
            left:cx := cx.make 3 4;
            char:c := 'c';
            right := cx.conj left;
            re:r := cx.re left;
            im:r := cx.im left;
            abs:r := cx.abs left;
            arg:r := cx.arg left;
            scalar:cx := + left right;
            array0:array[cx] := form [2] [left right];
            array1:array[cx] := * array0 left;
            seq:[cx] := - [left right] right;
            inv:cx->cx->cx := &/;
            half:cx->cx := @ inv left;
            result:cx := @ half right;
            eq:b := == array0 array1;
            elts:array[b] := /= array0 array1;
            right_alias:cx->cx := &{ cx.conj };
            components:array[r] := @: &cx.re array1;
        "#;
        let program = crate::parser::parse(source).unwrap();
        prepare(&program, Mode::Script, Policy::TypedOnly).unwrap();

        for (fragment, diagnostic) in [
            ("mixed := + left 1r;", "cannot unify"),
            ("mixed := + left 1q;", "cannot unify"),
            ("badorder := < left left;", "does not satisfy ord"),
            (r"badkey := =: (\x -> x) [left];", "does not satisfy hash"),
            ("badnum := +/ [left];", "does not satisfy num"),
            ("badmake := cx.make 1q 2r;", "cannot unify"),
        ] {
            let source = format!(r#"<< "cx"; left := cx.make 1 2; {fragment}"#);
            let program = crate::parser::parse(&source).unwrap();
            let resolved = crate::resolve::resolve(&program).unwrap();
            let error = crate::infer::infer(&program, &resolved).unwrap_err();
            assert!(error.message.contains(diagnostic), "{source}: {error}");
        }
        let redecl = crate::parser::parse("cx :: { num:r; };").unwrap_err();
        assert!(redecl.message.contains("built-in type name"));
    }
}
