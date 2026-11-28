use super::{hir_validation::checked, run_script, script};
use crate::support::TempDir;
use std::fs;

fn accepts(source: &str, stdout: &str) -> TempDir {
    let f = script("compiler-regression", source);
    for command in ["check", "hir", "run-hir"] {
        let result = run_script(&f, &[command]);
        assert!(
            result.status.success(),
            "{command}: {}\n{source}",
            String::from_utf8_lossy(&result.stderr)
        );
        if command == "run-hir" {
            assert_eq!(String::from_utf8_lossy(&result.stdout), stdout);
        }
    }
    f
}

fn rejects(source: &str, diagnostic_fragments: &[&str]) {
    let executable = if source.contains("!{") {
        source.to_owned()
    } else {
        format!("{source}\n!{{ () }};")
    };
    let f = script("compiler-regression", &executable);
    for command in ["check", "run-hir"] {
        let result = run_script(&f, &[command]);
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(
            !result.status.success()
                && !stderr.contains("E0400")
                && diagnostic_fragments
                    .iter()
                    .any(|hint| stderr.contains(hint)),
            "{command}: {stderr}\n{source}"
        );
    }
}

#[test]
fn rejects_local_eager_initialization_cycles_through_calls() {
    rejects(
        "!{ a := f (); f(x:()):z := a; () };",
        &["initialization", "initializer", "initialized", "cycle"],
    );
    rejects(
        "!{ a := f (); f(x:()):z := g x; g(x:()):z := a; () };",
        &["initialization", "initializer", "initialized", "cycle"],
    );
}

#[test]
fn rejects_eager_calls_through_known_product_projections() {
    for source in [
        "!{ boxed := (&f,()); a := @ (.1 boxed) (); f(x:()):z := a; () };",
        "!{ r := .{ fn := &f; }; a := @ (.fn r) (); f(x:()):z := a; () };",
    ] {
        rejects(source, &["before its initializer"]);
    }
}

#[test]
fn local_forward_call_and_unforced_capture_are_legal() {
    accepts(
        r#"<< "io";
!{
    early := forward 1;
    forward(x:z):z := + x 1;
    a := &read;
    held := 23;
    read(x:()):z := held;
    ^= ! io.print early { _ : (); };
    ^= ! io.print @ a () { _ : (); };
};"#,
        "2\n23\n",
    );
}

#[test]
fn shadowed_nominal_ids_are_lexically_distinct() {
    accepts(
        r#"<< "io";
rr :: { a:z; };
outer(x:rr) := .a x;
!{
    inner := { rr :: { b:s; }; .b rr { b := "inside"; } };
    ^= ! io.print inner { _ : (); };
    ^= ! io.print outer rr { a := 3; } { _ : (); };
};"#,
        "inside\n3\n",
    );
    rejects(
        r#"rr :: { a:z; };
outer(x:rr) := .a x;
bad := { rr :: { a:z; }; outer rr { a := 3; } };"#,
        &["unify", "nominal", "schema mismatch", "expected"],
    );
}

#[test]
fn explicit_variables_cannot_specialize_or_escape_into_capture() {
    rejects(
        "outer(x) := { pretend(y:~a):~a := x; &pretend };",
        &["rigid", "specializ", "escape", "general"],
    );
    rejects("bad(x:~a):~a := 1;", &["rigid", "specializ", "general"]);
    accepts(
        r#"<< "io";
outer(x) := { identity(y:~a):~a := y; (identity 7, identity "s", x) };
!{ ^= ! io.print outer 42 { _ : (); }; };"#,
        "(7, s, 42)\n",
    );
}

#[test]
fn unconstrained_empty_occurrences_have_no_implicit_element_type() {
    rejects("bad(x) := == [] [];", &["empty", "ambiguous"]);
    rejects("bad(x) := { y := ([],x); x };", &["empty", "ambiguous"]);
    accepts(
        r#"<< "io";
f(xs:[z]) := == xs [];
empty(x:~a):[~a] := [];
!{ ^= ! io.print f [] { _ : (); }; ^= ! io.print f [1] { _ : (); }; ^= ! io.print empty "x" { _ : (); }; ^= ! io.print empty 2 { _ : (); }; };"#,
        "1b\n0b\n[]\n[]\n",
    );
}

#[test]
fn typed_empty_schema_is_retained_across_a_closure() {
    let f = accepts(
        r#"<< "csv";
<< "df";
row :: { key:s; n:z; };
summary(k:s,t:#row) := .{ key := k; count := # t; };
group(t:#row) := { summarizer := &summary; df.agg &.key (summarizer) t };
input:#row := #[];
!{ ^{ ^? ! csv.write "out.csv" (group input); () } };"#,
        "",
    );
    assert_eq!(
        fs::read_to_string(f.join("out.csv"))
            .expect("typed-only write")
            .replace("\r\n", "\n"),
        "key,count\n"
    );
}

#[test]
fn constrained_numeric_evidence_survives_escaping_closures() {
    accepts(
        r#"<< "io";
sq(x:~a):~a | num ~a := * x x;
factory(x) := { g(y) := sq y; &g };
!{ ^= ! io.print @ (factory ()) 2 { _ : (); }; ^= ! io.print @ (factory ()) 0.5 { _ : (); }; };"#,
        "4\n1/4\n",
    );
}

#[test]
fn indirect_numeric_operator_survives_partial_application() {
    accepts(
        r#"<< "io";
op := &+;
!{ addone := @ op 1; ^= ! io.print @ addone 2 { _ : (); }; };"#,
        "3\n",
    );
}

#[test]
fn aggregate_hir_carries_statically_ordered_schema() {
    use flux_lang::hir::{CoreOp, ExprKind, TableSchema, Target};

    let source = r#"
<< "df";
row :: { key:s; n:z; };
summary(k:s,t:#row) := .{ key := k; count := # t; };
group(t:#row) := { summarizer := &summary; df.agg &.key (summarizer) t };
res := group (#[row { key := "k"; n := 3; }]);
"#;
    let prepared = checked(source);
    let node = prepared
        .arena
        .iter()
        .find(|node| {
            matches!(
                node.kind,
                ExprKind::Call(Target::Builtin(CoreOp::Aggregate), _)
            )
        })
        .expect("elaborated aggregate");
    let Some(TableSchema::Named(fields)) = node.table_schema.as_deref() else {
        panic!("aggregate missing its checked named schema: {node:?}");
    };
    let names: Vec<_> = fields.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["key", "count"]);
}

#[test]
fn generic_named_calls_carry_distinct_checked_instances_in_hir() {
    use flux_lang::hir::{ExprKind, Target};
    use flux_lang::infer::Operation;
    use flux_lang::types::{Prim, Type};

    let prepared = checked("sq(x:~a):~a | num ~a := * x x; z := sq 2z; q := sq 0.5;");
    let arena = &prepared.arena;
    for prim in [Prim::Z, Prim::Q] {
        let want = Type::Fun(Box::new(Type::Prim(prim)), Box::new(Type::Prim(prim)));
        assert!(arena.iter().any(|node| {
            matches!(node.kind, ExprKind::Call(Target::Symbol(_), _))
                && node.callee_type.as_ref() == Some(&want)
        }));
    }
    assert!(arena.iter().any(|node| {
        matches!(
            &node.evidence,
            Some(Operation::Numeric { operator, result: Type::Var(_) }) if operator == "*"
        )
    }));
}
