//! Shared numeric and shape inference for Mathesis calls and references.
use super::super::{p, s, Engine, Env, R};
use crate::parser::Expr;
use crate::types::{Class, Prim, Type};

use crate::builtins::{Builtin, MathShape, MathType, Shape};

impl Engine<'_> {
    pub(super) fn mathesis_call(
        &mut self,
        env: &Env,
        spec: &Builtin,
        args: &[Expr],
        want: Option<&Type>,
        effect: bool,
    ) -> R<Type> {
        let signature = spec.math_signature().expect("registry Mathesis signature");
        let numeric = self.cx.fresh();
        let mut types = Vec::with_capacity(args.len());
        let wanted = want.map(|ty| self.cx.resolve(ty));
        for (arg, &param) in args.iter().zip(signature.inputs) {
            let scalar = param
                .instantiate(&numeric)
                .expect("Mathesis inputs have a fully specified type");
            let expected =
                if spec.shape != Shape::Elementwise || matches!(arg, Expr::Int(_) | Expr::Num(_)) {
                    Some(scalar)
                } else if matches!(arg, Expr::Array(_)) {
                    Some(s(scalar))
                } else if args.len() == 1 {
                    wanted.as_ref().and_then(|out| match MathShape::of(out) {
                        MathShape::Scalar => None,
                        shape => Some(shape.wrap(scalar)),
                    })
                } else {
                    None
                };
            types.push(self.expr(env, arg, expected.as_ref(), effect)?);
        }
        if types.contains(&Type::Never) {
            return Ok(Type::Never);
        }
        self.mathesis_types(spec, &types, want)
    }

    pub(super) fn mathesis_types(
        &mut self,
        spec: &Builtin,
        types: &[Type],
        want: Option<&Type>,
    ) -> R<Type> {
        let signature = spec.math_signature().expect("registry Mathesis signature");
        let operands = types
            .iter()
            .map(|ty| self.cx.resolve(ty))
            .collect::<Vec<_>>();
        let inputs = operands.iter().collect::<Vec<_>>();
        let (shape, elements) = spec
            .math_operands(&inputs)
            .map_err(|error| self.err(format!("{}: {error}", spec.canonical)))?;
        let numeric = self.cx.fresh();
        for (actual, &param) in elements.iter().zip(signature.inputs) {
            let expected = param
                .instantiate(&numeric)
                .expect("Mathesis input signatures do not coerce");
            self.unify(actual, &expected)?;
        }
        if signature
            .inputs
            .iter()
            .any(|param| param.numeric_parameter())
        {
            self.req(Class::Num, &numeric)?;
        }

        let mut domain = self.cx.resolve(&numeric);
        if matches!(
            signature.output,
            MathType::QuotientT | MathType::SeqQuotientT
        ) && matches!(domain, Type::Var(_))
        {
            // A contextual approximate result determines r; all other
            // unresolved operands need explicit input precision, save the
            // established default for ordinary unsuffixed integer literals.
            let wanted = want.map(|ty| self.cx.resolve(ty));
            if signature
                .output
                .instantiate(&p(Prim::R))
                .map(|t| shape.wrap(t))
                == wanted
            {
                self.unify(&numeric, &p(Prim::R))?;
            } else {
                self.require_exact_numeric(&numeric, spec.canonical)?;
            }
            domain = self.cx.resolve(&numeric);
        }
        let output = signature
            .output
            .instantiate(&domain)
            .ok_or_else(|| self.err(format!("{} needs a known numeric domain", spec.canonical)))?;
        Ok(shape.wrap(output))
    }
}

#[cfg(test)]
mod tests {
    use super::R;
    use crate::infer::{infer, InferResult};

    fn prepare(source: &str) -> R<InferResult> {
        let parsed = crate::parser::parse(source).unwrap();
        let resolved = crate::resolve::resolve(&parsed).unwrap();
        infer(&parsed, &resolved)
    }

    #[test]
    fn shared_mathesis_signatures_infer_at_direct_and_first_class_calls() {
        let source = r#"
            << "math" : m;
            << "poly" : p;
            << "num" : n0;
            floor(x:~a):z | num ~a := m.floor x;
            ff:[q]->[z] := &m.floor;
            sine:array[r]->array[r] := &m.sin;
            r0:r := m.sin 0;
            r1:[r] := m.exp [0 1];
            recip:q := m.recip 3;
            pol:[d] := p.mul [1.20d] [2.30d];
            integral:[q] := p.integ pol;
            prin:[r] := p.integ r1;
            arr:array[r] := @ sine (form [2] r1);
            constants:(r,r,r) := (m.pi,m.e,m.tau);
            gc:[n] := m.gcd [-24 0] 12;
            gt:array[n] := m.gcd (form [2] [2z 4z]) 6;
            f(x:r):r := - x 3r;
            outcome:r^solvererr := n0.bisect &f 0 8 .{ max := 100; tol := 0.001r; };
            rb:(r->r)->r->r->.{tol:r;max:n;}->r^solvererr := &n0.bisect;
            recovered:r := ^= outcome { _ : 0r; };
        "#;
        prepare(source).unwrap();
        let program = crate::parser::parse(source).unwrap();
        crate::semantic::prepare(
            &program,
            crate::semantic::Mode::Script,
            crate::semantic::Policy::TypedOnly,
        )
        .unwrap();
    }

    #[test]
    fn mathesis_static_types_reject_precision_shape_and_callback_mismatches() {
        for (source, diagnostic) in [
            (r#"<< "math"; bad := math.sin 1q;"#, "cannot unify"),
            (
                r#"<< "math"; bad := math.hypot (form [1] [1r]) [1r];"#,
                "cannot mix",
            ),
            (r#"<< "math"; bad := math.exp [[1r]];"#, "cannot unify"),
            (r#"<< "poly"; bad := poly.eval [1q] 1r;"#, "cannot unify"),
            (
                r#"<< "num"; bad := num.diff (\x -> print x) 2r 0.01r;"#,
                "cannot unify",
            ),
        ] {
            let err = prepare(source).unwrap_err();
            assert!(err.message.contains(diagnostic), "{source}: {err}");
        }
    }
}
