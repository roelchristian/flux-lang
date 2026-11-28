//! Checked signatures and bidirectional elaboration of the array primitives.
use super::*;

/// Callback operands are resolved after their data inputs, so concrete
/// element types can guide lambdas and instantiate polymorphic references.
/// This is inference order only; evaluation order is unchanged.
const CALLBACKS: &[(&str, usize)] = &[
    ("arr.map", 0),
    ("arr.zip", 0),
    ("arr.outer", 0),
    ("arr.rankmap", 1),
    ("arr.scan", 0),
    ("arr.grade", 0),
];

impl Engine<'_> {
    fn array_signature(&mut self, op: &str) -> (Vec<Type>, Type, Option<Type>) {
        let n = p(Prim::N);
        let dims = s(n.clone());
        let a = self.cx.fresh();
        let b = self.cx.fresh();
        let c = self.cx.fresh();
        let array = Type::dense_array;
        let aa = array(a.clone());
        let sa = s(a.clone());
        let order_key = (op == "arr.grade").then(|| b.clone());
        let (inputs, output) = match op {
            "arr.reshape" => (vec![dims, sa], aa),
            "arr.shape" => (vec![aa], dims),
            "arr.ravel" => (vec![aa], sa),
            "arr.at" => (vec![dims, aa], o(a)),
            "arr.map" => (vec![f(a, b.clone()), aa], array(b)),
            "arr.zip" | "arr.outer" => {
                (vec![f(a, f(b.clone(), c.clone())), aa, array(b)], array(c))
            }
            "arr.rankmap" => (vec![n, f(aa.clone(), b.clone()), aa], array(b)),
            "arr.scan" => (vec![f(b.clone(), f(a, b.clone())), b.clone(), aa], array(b)),
            "arr.windows" => (vec![n, sa.clone()], s(sa)),
            "arr.select" => (vec![dims, sa.clone()], o(sa)),
            "arr.grade" => (vec![f(a, b.clone()), sa], dims),
            "arr.permute" => (vec![dims, sa.clone()], sa),
            _ => unreachable!("unrecognized array primitive: {op}"),
        };
        (inputs, output, order_key)
    }

    pub(super) fn array_call(
        &mut self,
        env: &Env,
        op: &str,
        args: &[Expr],
        want: Option<&Type>,
        effect: bool,
    ) -> R<Type> {
        let (inputs, output, order_key) = self.array_signature(op);
        if let Some(w) = want {
            self.unify(&output, w)?;
        }
        let callback = CALLBACKS.iter().find_map(|&(n, i)| (n == op).then_some(i));
        let mut bottom = false;
        for (i, (arg, expected)) in args.iter().zip(&inputs).enumerate() {
            if callback != Some(i) {
                bottom |= self.expr(env, arg, Some(expected), effect)? == Type::Never;
            }
        }
        if let Some(i) = callback {
            bottom |= self.expr(env, &args[i], Some(&inputs[i]), effect)? == Type::Never;
        }
        if let Some(key) = order_key {
            self.req(Class::Ord, &key)?;
        }
        Ok(if bottom { Type::Never } else { output })
    }

    pub(super) fn array_types(&mut self, op: &str, args: &[Type]) -> R<Type> {
        let (inputs, output, order_key) = self.array_signature(op);
        for (got, expected) in args.iter().zip(&inputs) {
            self.unify(got, expected)?;
        }
        if let Some(key) = order_key {
            self.req(Class::Ord, &key)?;
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(source: &str) -> R<InferResult> {
        let program = crate::parser::parse(source).unwrap();
        let names = crate::resolve::resolve(&program).unwrap();
        infer(&program, &names)
    }

    #[test]
    fn signatures_guide_shapes_and_polymorphic_callbacks() {
        check(
            r#"
            << "arr";
            id(x) := x;
            x:array[z] := arr.reshape [2] [1 2];
            y:array[z] := arr.map &id x;
            z0:array[s] := arr.map (\k -> (.label k)) (arr.reshape [1] [.{ label := "a"; }]);
            dims:[n] := arr.shape y;
            at:?z := arr.at [1] x;
            scalar:array[z] := arr.reshape [] [1];
            empty:array[s] := arr.reshape [0] [];
            first:array[z] := arr.map &id scalar;
        "#,
        )
        .unwrap();
    }

    #[test]
    fn grade_needs_ordering_and_arrays_are_not_sequences() {
        let grade = check(r#"<< "arr"; bad := arr.grade (\x -> (\y -> x)) [1];"#).unwrap_err();
        assert!(grade.message.contains("ord"), "{grade}");
        let bad = check(r#"<< "arr"; bad := arr.ravel [1];"#).unwrap_err();
        assert!(bad.message.contains("array"), "{bad}");
    }

    #[test]
    fn builtin_references_preserve_linked_element_types() {
        check(
            r#"
            << "arr";
            reshape := &arr.reshape;
            x:array[n] := @@ reshape ([2], [2n 3n]);
            y:array[s] := @@ reshape ([1], ["a"]);
            grade := &arr.grade;
            order:[n] := @@ grade ((\x -> x), ["b" "a"]);
        "#,
        )
        .unwrap();
    }
}
