//! End-to-end checks of inference contracts.
use super::{f, infer, InferErrorKind, InferResult, Operation, R};
use crate::parser::{Expr, Stmt};
use crate::types::{Prim, Type};
use std::collections::HashSet;

fn check(source: &str) -> R<InferResult> {
    let p = crate::parser::parse(source).unwrap();
    let names = crate::resolve::resolve(&p).unwrap();
    infer(&p, &names)
}
#[test]
fn independent_generic_instantiations_and_recursive_scc() {
    check(
        r#"id(x) := x; a := id 1; b := id "x";
          even(x:n):b := ? (= x 0) 1b (odd (- x 1));
          odd(x:n):b := ? (= x 0) 0b (even (- x 1));
          c := even 2;"#,
    )
    .unwrap();
    assert_eq!(
        check("loop(x) := loop [x];").unwrap_err().kind,
        InferErrorKind::TypeError
    );
}
#[test]
fn higher_order_and_open_rows() {
    check(
        r#"
          apply(g,x) := @ g x;
          id(v) := v;
          value(row) := .val row;
          a := apply &id 3;
          b := value .{ label := "a"; val := 1; };
          c := value .{ val := 1.25; flag := 1b; };
        "#,
    )
    .unwrap();
    assert_eq!(
        check("value(r) := .val r; a := value .{ nope := 0; };")
            .unwrap_err()
            .kind,
        InferErrorKind::TypeError
    );
}
#[test]
fn annotated_signature_constraints_and_nominal_evidence() {
    check("f(x:[~a]):b | eq ~a := == x x; >> (f);").unwrap();
    let e = check("f(x:[~a]):b := == x x; >> (f);").unwrap_err();
    assert_eq!(e.kind, InferErrorKind::TypeError);
    assert!(e.message.contains("lacks eq ~a"), "{e}");
    let e=check("schema :: { callback:z->z; }; id(x) := x; same(x) := == x x; data := schema { callback:=&id; }; b := same data;").unwrap_err();
    assert_eq!(e.kind, InferErrorKind::TypeError);
    assert!(e.message.contains("callback"), "{e:?}");
    check(
        "schema :: { count:z; }; same(x) := == x x; data := schema { count:=2; }; b := same data;",
    )
    .unwrap();
}
#[test]
fn value_restriction_and_empty_ambiguity() {
    assert!(check(r#"id(x) := x; x := ? 1b &id &id; a := @ x 1; b := @ x "s";"#).is_err());
    assert!(check("empty := [];").is_err());
    assert!(check("bad(x) := == [] [];")
        .unwrap_err()
        .message
        .contains("ambiguous"));
    assert!(check("bad(x) := { y := ([],x); x };")
        .unwrap_err()
        .message
        .contains("ambiguous"));
    check("empty:[z] := [];").unwrap();
    check("f(x:[~a]):b | eq ~a := == x [];").unwrap();
}
#[test]
fn local_functions_generalize_after_acyclic_and_recursive_groups() {
    check(
        r#"run(u:()):() := {
            id(x) := x;
            a := id 1;
            b := id "one";
            first(x) := second x;
            second(y) := y;
            c := first 1;
            d := first "two";
            ()
        };"#,
    )
    .unwrap();
}
#[test]
fn diagnostics_have_the_expression_span_when_resolved() {
    let source = "good := 1;\ninvalid := * 2 0b;";
    let e = check(source).unwrap_err();
    assert_eq!(e.start, source.find('*').unwrap());
    assert_eq!(e.end, source.len() - 1);
}
#[test]
fn schemas_keep_lexical_identity_and_fields() {
    let inferred = check(
        r#"
            rr :: { x:z; };
            take_outer(v:rr):z := .x v;
            outer:rr := rr { x:=7; };
            val:s := {
                rr :: { y:s; };
                inner:rr := rr { y:="inside"; };
                seen:z := take_outer outer;
                .y inner
            };
            after:rr := rr { x:=8; };
            next:z := take_outer after;
        "#,
    )
    .unwrap();
    let shapes = inferred
        .schemas
        .iter()
        .filter(|(id, _)| id.name == "rr")
        .collect::<Vec<_>>();
    assert_eq!(shapes.len(), 2);
    assert_ne!(shapes[0].0.id, shapes[1].0.id);
    let e = check("rr :: { v:z; }; bad:rr := { rr :: { v:z; }; rr { v:=1; } };").unwrap_err();
    assert_eq!(e.kind, InferErrorKind::TypeError);
}

#[test]
fn rejects_skolem_escape_from_nested_signature() {
    let e = check(
        r#"
            outer(f) := {
                pretend(x:~a):~a := @ f x;
                pretend "s"
            };
        "#,
    )
    .unwrap_err();
    assert!(e.message.contains("escapes"), "{e:?}");
    check(r#"id(x:~a):~a := x; i := id 7; s := id "s";"#).unwrap();
    check("sq(x:~a):~a | num ~a := * x x; a := sq 4;").unwrap();
}
#[test]
fn closed_default_integer_terms_preserve_context_and_evidence() {
    let p=crate::parser::parse(
        "u := * (+ 2 3) 4; n:n := + 2 3; q:q := + 2 3; r := + n 4; fnn(x:q):q := + x 1; x := fnn 5;"
    ).unwrap();
    let names = crate::resolve::resolve(&p).unwrap();
    let r = infer(&p, &names).unwrap();
    let body = |i: usize| match &p.defs[i] {
        Stmt::Def { body, .. } => body,
        _ => unreachable!(),
    };
    let z = Type::Prim(Prim::Z);
    assert_eq!(r.type_of(body(0)), Some(&z));
    assert_eq!(
        r.operation_of(body(0)),
        Some(&Operation::Numeric {
            operator: "*".into(),
            result: z.clone()
        })
    );
    let Expr::Call(_, xs) = body(0) else {
        unreachable!()
    };
    assert_eq!(r.type_of(&xs[0]), Some(&z));
    assert_eq!(r.type_of(&xs[1]), Some(&z));
    assert_eq!(r.type_of(body(1)), Some(&Type::Prim(Prim::N)));
    assert_eq!(r.type_of(body(2)), Some(&Type::Prim(Prim::Q)));
    assert_eq!(r.type_of(body(3)), Some(&Type::Prim(Prim::N)));
    assert_eq!(
        r.callee_type(body(5)),
        Some(&f(Type::Prim(Prim::Q), Type::Prim(Prim::Q)))
    );
}
#[test]
fn result_boundaries_skips_and_handler_types() {
    let src = r#"
            fail :: { | code z; | other; };
            div(x:z):z^fail := ^{ ? 1b (^: code x) x };
            value := ^= div 7 { code x : x; other : 1; };
            !{ ^{ ^? div 0; () } };
        "#;
    let p = crate::parser::parse(src).unwrap();
    let names = crate::resolve::resolve(&p).unwrap();
    let r = infer(&p, &names).unwrap();
    assert_eq!(r.boundaries.len(), 4);
    assert_eq!(
        r.boundaries.values().copied().collect::<HashSet<_>>().len(),
        2
    );
    assert_eq!(
        r.constructors
            .values()
            .filter(|c| c.owner.name == "fail")
            .count(),
        2
    );
    let err = check("fail :: { | a; | b; }; f(x:z):z^fail := ^{ ^: a }; s := ^= f 0 { a : 0; };")
        .unwrap_err();
    assert!(err.message.contains("nonexhaustive"), "{err:?}");
    let err = check("fail :: { | a; }; f(x:z):z^fail := ^{ ^: a }; s := ^= f 0 { a : 0; _ : 1; };")
        .unwrap_err();
    assert!(err.message.contains("unreachable"), "{err:?}");
}

#[test]
fn rejects_executable_result_ambiguity_and_unconsumed_pipeline() {
    check("make(x) := ^{ x };").unwrap();
    let e = check("make(x) := ^{ x }; !{ n := make 1; () };").unwrap_err();
    assert!(e.message.contains("ambiguous"), "{e:?}");
    let e = check("err :: { | bad; }; fail(x:z):z^err := ^{ ^: bad }; !{ |{ fail 0z; () } };")
        .unwrap_err();
    assert!(e.message.contains("discard"), "{e:?}");
    check("err :: { | bad; }; fail(x:z):z^err := ^{ ^: bad }; !{ |{ fail 0z; ^= $ { bad : 0z; }; () } };").unwrap();
}

#[test]
fn resets_lexical_error_boundary_at_closures_functions_and_actions() {
    for source in [
        r#"e :: { | wrong; }; f(x:z):z^e := ^{ f2 := \u -> ^: wrong; x };"#,
        "e :: { | wrong; }; f(x:z):z^e := ^{ f2(u:()):z := ^: wrong; x };",
        "e :: { | wrong; }; f(x:z):z^e := ^{ suspended := !{ ^: wrong }; x };",
    ] {
        let err = check(source).unwrap_err();
        assert!(err.message.contains("boundary"), "{err:?}");
    }
    let err = check("f(x:z):z^s := ^{ ^: x };").unwrap_err();
    assert_eq!(err.kind, InferErrorKind::IncompatibleError);
}
#[test]
fn staged_builtin_references_preserve_arity_and_collection_context() {
    let src = r#"
            sum(x,y:z):z := + x y;
            foldsum := @ &/: &sum;
            total:z := @ (@ foldsum 0) [1 2 3];
            accs:[z] := \: &sum 0 [];
            joined:[q] := +: [] [];
            paired:[(z,s)] := &: [] [];
            crossed:[(z,s)] := *: [1] ["x"];
            idx:n->[z]->?z := &.:;
            found:?z := @ (@ idx 1) [7];
            measure:[q]->?q := &var;
            stat:?q := @ measure [1 3];
        "#;
    check(src).unwrap();
}

#[test]
fn higher_order_actions_receive_schema_and_filter_uses_the_actual_mask_type() {
    check(
        r#"
            << "csv";
            row :: { value:q; };
            typed(action:!(#row^csverr)):!(#row^csverr) := action;
            load := @ &typed csv.read "nothing.csv";
            flags(u:()):[b] := [1b 0b];
            kept:[z] := ?: flags () [2 3];
        "#,
    )
    .unwrap();
}

#[test]
fn annotation_failure_points_at_the_unresolved_named_type() {
    let src = "f(x:?absent):z := 1;";
    let err = check(src).unwrap_err();
    assert_eq!(&src[err.start..err.end], "absent");
    assert!(err.message.contains("unknown type"));
}
#[test]
fn exact_decimal_promotions_quantize_and_wide_literal_context() {
    check(
        r#"
            << "math";
            identity(x:~a):~a := x;
            magnitude:n := 12345678901234567890123456789012345678901234567890;
            fractional:q := 12345678901234567890123456789012345678901234567890;
            amount:d := identity 12.340d;
            sum:d := + 2z amount;
            quotient:q := / 1.20d 3n;
            checked:q^numerr := math.div 1.20d 3n;
            round:n->q->d := &math.quantize;
            result:d := @ (@ round 3) (/ 11q 4q);
            finite:d := :d 0.125q;
            batches:[d] := + [1.00d 2.00d] 3n;
        "#,
    )
    .unwrap();
    for failure in [
        "bad:n := -12345678901234567890123456789012345678901234567890;",
        "bad := + 1.0d 1.0r;",
        "bad := // 1.0d 1z;",
        "bad := % 1.0d 1z;",
        r#"<< "math"; bad := math.quantize -1 2.0d;"#,
        r#"<< "math"; bad := math.quantize 2 0.125r;"#,
    ] {
        assert_eq!(
            check(failure).unwrap_err().kind,
            InferErrorKind::TypeError,
            "{failure}"
        );
    }
}
#[test]
fn decimal_schema_supported_in_checked_csv_actions() {
    check(
        r#"
            << "csv";
            row :: { amount:d; discount:?d; };
            reader(path:s):!(#row^csverr) := csv.read path;
            writer(path:s,t:#row):!(()^csverr) := csv.write path t;
        "#,
    )
    .unwrap();
}
#[test]
fn optional_and_result_combinators_check_context_and_callbacks() {
    check(
        r#"
            << "opt";
            << "res";
            add(x:z):z := + x 1;
            lift(x:z):?z := ?1 x;
            good:z^s := opt.result "missing" (?1 2z);
            bad:z^s := opt.result "missing" ?0;
            mapped:z^s := res.map &add good;
            transformed:z^n := res.maperr (\tag -> # tag) bad;
            may:?z := opt.bind &lift (opt.map &add (?1 1z));
            collected:[z]^s := res.collect [good bad];
            parts:([z],[s]) := res.partition [];
            indicator:b := res.ok bad;
            optional:?z := res.optional mapped;
            indirect:(z->z)->(z^s)->z^s := &res.map;
        "#,
    )
    .unwrap();
    let bad = check(r#"<< "opt"; out := opt.bind (\x -> + x 1) (?1 2z);"#).unwrap_err();
    assert_eq!(bad.kind, InferErrorKind::TypeError);
}

#[test]
fn standard_print_requires_no_module_and_chronological_json_context_is_checked() {
    check(
        r#"
            action:!(()^ioerr) := print 1;
            !{ ^{ ^? ! action; () } };
        "#,
    )
    .unwrap();
    check(
        r#"
            << "io";
            << "fs";
            << "sys";
            << "str";
            << "rx";
            << "dt";
            << "json";
            read:!(?s^ioerr) := io.readln;
            path:s := fs.join "a" "b";
            exists:!(b^ioerr) := fs.exists path;
            pid:!n := sys.pid;
            args:![s] := sys.args;
            cols:[s] := str.split "," "a,b";
            match:?s^rxerr := rx.find "a+" "aaa";
            days:z^dterr := dt.days "1970-01-01";
            date:s^dterr := dt.date 0;
            row :: { value:z; text:?s; };
            decoded:row^jsonerr := json.decode "{}";
            decoder:s->row^jsonerr := &json.decode;
        "#,
    )
    .unwrap();
    for source in [
        r#"<< "json"; decoded := json.decode "{}";"#,
        r#"<< "json"; decoded:(z->z)^jsonerr := json.decode "{}";"#,
        r#"<< "json"; decoded:(?())^jsonerr := json.decode "null";"#,
    ] {
        assert_eq!(check(source).unwrap_err().kind, InferErrorKind::TypeError);
    }
}
#[test]
fn relational_joins_require_equal_hashable_keys_and_produce_typed_rows() {
    check(r#"
            << "df";
            leftrow :: { code:n; slot:?s; };
            rightrow :: { code:n; slot:?s; };
            leftkey(x:leftrow):(n,?s) := (.code x,.slot x);
            rightkey(x:rightrow):(n,?s) := (.code x,.slot x);
            l:#leftrow := #[leftrow { code:=1; slot:=?0; }];
            r:#rightrow := #[rightrow { code:=1; slot:=?0; }];
            inner:#(leftrow,rightrow) := *= &leftkey &rightkey l r;
            pairs:#(leftrow,?rightrow) := *? &leftkey &rightkey l r;
            semi:#leftrow := df.semi &leftkey &rightkey l r;
            anti:#leftrow := df.anti &leftkey &rightkey l r;
            refjoin:(leftrow->(n,?s))->(rightrow->(n,?s))->#leftrow->#rightrow->#(leftrow,rightrow) := &*=;
            indirect:#(leftrow,rightrow) := @@ refjoin (&leftkey,&rightkey,l,r);
        "#).unwrap();
    for source in [
        "a:#[(n,n)] := #[(1n,2n)]; b:#[(z,n)] := #[(1z,2n)]; j := *= &.1 &.1 a b;",
        "id(x:n):n := x; a := #[.{ fnn:=&id; }]; j := *= &.fnn &.fnn a a;",
    ] {
        assert_eq!(
            check(source).unwrap_err().kind,
            InferErrorKind::TypeError,
            "{source}"
        );
    }
}

#[test]
fn relational_projection_and_optional_transforms_get_context() {
    check(
        r#"
            << "df";
            row :: { val:?q; note:s; };
            table:#row := #[row { val:=?0; note:="x"; }];
            union:#row := df.union #[] table;
            dedup:#row := df.distinct union;
            picked:#.{ amount:?q; } := df.select (\r -> .{ amount:=.val r; }) table;
            rows:#row := df.dropna &.val table;
            filled:[q] := df.fill 0 .val rows;
            present:[q] := df.present [];
        "#,
    )
    .unwrap();
    assert_eq!(
        check(r#"<< "df"; t := #[(1z,2z)]; p := df.select &.1 t;"#)
            .unwrap_err()
            .kind,
        InferErrorKind::TypeError
    );
    assert_eq!(
        check(r#"<< "df"; t := #[(1z,2z)]; p := df.dropna &.1 t;"#)
            .unwrap_err()
            .kind,
        InferErrorKind::TypeError
    );
}
