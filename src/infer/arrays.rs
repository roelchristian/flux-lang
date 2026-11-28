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
    ("arr.red", 1),
    ("arr.fold", 1),
    ("arr.scana", 1),
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
            "arr.axes" => (vec![dims, aa.clone()], aa),
            "arr.sect" => (vec![dims.clone(), dims, aa.clone()], aa),
            "arr.red" => (
                vec![n, f(a.clone(), f(a.clone(), a.clone())), aa],
                array(o(a)),
            ),
            "arr.fold" | "arr.scana" => (
                vec![n, f(b.clone(), f(a, b.clone())), b.clone(), aa],
                array(b),
            ),
            "arr.diag" => (vec![aa.clone()], aa),
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
        if op == "form" {
            let shape = self.expr(env, &args[0], Some(&s(p(Prim::N))), effect)?;
            let target = want.map(|w| self.cx.resolve(w));
            let item_type = target.as_ref().and_then(Type::array_element).cloned();
            let items = if matches!(&args[1], Expr::Array(_)) {
                let expected = item_type.map(s);
                self.expr(env, &args[1], expected.as_ref(), effect)?
            } else {
                self.expr(env, &args[1], None, effect)?
            };
            if shape == Type::Never || items == Type::Never {
                return Ok(Type::Never);
            }
            return self.form_type(&items);
        }
        if matches!(op, "arr.dot" | "arr.mul") {
            let args = args
                .iter()
                .map(|arg| self.expr(env, arg, None, effect))
                .collect::<R<Vec<_>>>()?;
            if args.contains(&Type::Never) {
                return Ok(Type::Never);
            }
            return self.contraction_type(op, &args, want);
        }
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

    pub(super) fn array_types(&mut self, op: &str, args: &[Type], want: Option<&Type>) -> R<Type> {
        match op {
            "form" => {
                self.unify(&args[0], &s(p(Prim::N)))?;
                return self.form_type(&args[1]);
            }
            "arr.dot" | "arr.mul" => return self.contraction_type(op, args, want),
            _ => {}
        }
        let (inputs, output, order_key) = self.array_signature(op);
        for (got, expected) in args.iter().zip(&inputs) {
            self.unify(got, expected)?;
        }
        if let Some(key) = order_key {
            self.req(Class::Ord, &key)?;
        }
        Ok(output)
    }

    /// Split dense shape from element domains before applying ordinary scalar
    /// typing rules. Once chosen, this path cannot recursively lift elements.
    pub(super) fn array_elementwise(
        &mut self,
        op: &str,
        operands: &[Type],
        want: Option<&Type>,
    ) -> R<Option<Type>> {
        if !operands
            .iter()
            .any(|ty| matches!(ty, Type::Applied(..) | Type::Var(_)))
        {
            return Ok(None);
        }
        let args = operands
            .iter()
            .map(|ty| self.cx.resolve(ty))
            .collect::<Vec<_>>();
        if !args.iter().any(|ty| ty.array_element().is_some()) {
            return Ok(None);
        }
        if args.iter().any(|ty| matches!(ty, Type::Seq(_))) {
            return Err(self.err(format!("{op} cannot mix a dense array and a sequence")));
        }
        let elems = args
            .iter()
            .map(|ty| ty.array_element().unwrap_or(ty).clone())
            .collect::<Vec<_>>();
        if elems
            .iter()
            .any(|ty| matches!(ty, Type::Seq(_)) || ty.array_element().is_some())
        {
            return Err(self.err(format!(
                "{op} does not lift recursively into array elements"
            )));
        }
        let expected = want.map(|ty| self.cx.resolve(ty));
        let expected = expected.as_ref().and_then(Type::array_element);
        Ok(Some(Type::dense_array(
            self.builtin_types(op, &elems, expected)?,
        )))
    }

    fn form_type(&self, items: &Type) -> R<Type> {
        let ty = self.cx.resolve(items);
        match ty {
            Type::Seq(elem) => Ok(Type::dense_array(*elem)),
            t if t.array_element().is_some() => Ok(t),
            Type::Var(_) => Err(self.err("form needs an annotated sequence or dense array input")),
            t => Err(self.err(format!("form expects a sequence or dense array, found {t}"))),
        }
    }

    fn contraction_type(&mut self, op: &str, args: &[Type], want: Option<&Type>) -> R<Type> {
        let a = self.cx.fresh();
        let b = self.cx.fresh();
        self.unify(&args[0], &Type::dense_array(a.clone()))?;
        self.unify(&args[1], &Type::dense_array(b.clone()))?;
        let desired = want.map(|t| self.cx.resolve(t));
        let want = if op == "arr.mul" {
            desired.as_ref().and_then(Type::array_element)
        } else {
            desired.as_ref()
        };
        let out = self.numeric_binary("*", &a, &b, want)?;
        // Contraction requires one numerical domain, not nested array or sequence elements.
        self.req(Class::Num, &out)?;
        Ok(if op == "arr.mul" {
            Type::dense_array(out)
        } else {
            out
        })
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
    #[test]
    fn dense_array_calculus_keeps_shape_separate_from_element_domains() {
        check(
            r#"
            << "arr";
            add(a:q,b:q):q := + a b;
            rat(x:d):q := :q x;
            row:array[d] := form [2] [1.20d 2.35d];
            other:array[n] := form [2] [2n 3n];
            exact:array[d] := + row other;
            lifted:array[r] := ^/ exact;
            from_scalar:array[z] := ^ 2z other;
            from_arrays:array[d] := ^ exact other;
            converted:array[q] := :q row;
            compared:array[b] := >= converted 0q;
            whole:b := == exact converted;
            negated:array[b] := ~ compared;
            n0:n := # row;
            dims:[n] := dim row;
            copy:array[d] := form [1 2] row;
            cell:?d := .: [1 2] copy;
            values:[d] := ,. copy;
            tr:array[d] := |: copy;
            mapped:array[q] := @: &rat copy;
            red:array[?q] := @/ &add mapped;
            total:array[q] := +/ mapped;
            folded:array[q] := /: &add 0 mapped;
            scanned:array[q] := arr.scana 1 &add 0 mapped;
            dot:d := arr.dot row other;
            product:array[q] := arr.mul mapped (form [2 1] [2q 3q]);
            diag:array[q] := arr.diag product;
            reshape := &arr.reshape;
            strings:array[s] := @@ reshape ([1], ["x"]);
            ref_form:[n]->array[d]->array[d] := &form;
            same:array[d] := @@ ref_form ([2],copy);
            compare:array[d]->d->array[b] := &>;
            mask:array[b] := @@ compare (same,1.20d);
        "#,
        )
        .unwrap();
    }

    #[test]
    fn dense_array_type_errors_are_not_runtime_shape_choices() {
        for (source, diagnostic) in [
            ("a := form [1] [1q]; bad := + a [2q];", "cannot mix"),
            (
                "a := form [1] [1q]; bad := + a 2r;",
                "does not implicitly mix",
            ),
            ("bad := &form;", "needs an annotated"),
            ("bad := .: 1n (form [1] [1q]);", "cannot unify"),
            (
                "bad := + (form [1] [[1q]]) 2q;",
                "does not lift recursively",
            ),
            (
                r#"bad := == (form [1] [(\x -> x)]) (form [1] [(\x -> x)]);"#,
                "does not satisfy eq",
            ),
            (
                "key(x:~a):~a | hash ~a := x; bad := key (form [1] [1q]);",
                "does not satisfy hash",
            ),
        ] {
            let error = check(source).unwrap_err();
            assert!(error.message.contains(diagnostic), "{source}: {error}");
        }
    }
}
