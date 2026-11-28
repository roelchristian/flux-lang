use super::fixture::{rejects, Fixture};
use crate::support::flux;

#[test]
fn calls_tacit_reference_and_lambda_declarations_directly_or_as_values() {
    Fixture::new(
        r#"
square/1 := &{ @* * . . };
sum/2:z->z->z := &+;
shift/2:z->z->z := (\x -> \y -> + (* 2z x) y);
same/2:z->z->z := &sum;
ordinary(x,y:z):z := + (* 2z x) y;
!{ ^{
    ^? ! print (square 6z,@ &square 7z,sum 2z 3z,@@ &sum (4z,5z),/ 12z 3z);
    ^? ! print (shift 4z 3z,ordinary 4z 3z,same 1z 5z);
    ()
} };
"#,
    )
    .accepts("", "(36, 49, 5, 9, 4)\n(11, 11, 6)\n");
}

#[test]
fn full_signature_resolves_declared_arity_and_keeps_residual_arrows() {
    Fixture::new(
        r#"
increment/1:z->z->z := &+;
combine/2:s->z->z->(s,z) := \label -> \a -> \b -> (label,+ a b);
!{ ^{
    ^? ! print (@ (increment 2z) 3z, @ (combine "sum" 4z) 5z);
    ()
} };
"#,
    )
    .accepts("", "(5, (sum, 9))\n");
}

#[test]
fn calls_forward_and_recursive_declarations_in_lexical_scopes() {
    Fixture::new(
        r#"
first := later 3z 4z;
later/2:z->z->z := &+;
tree[~a] :: { | leaf ~a; | branch tree[~a] tree[~a]; };
total/1:tree[z]->z := \t -> ?= t {
    leaf x : x;
    branch l r : + (total l) (total r);
};
local(x:z):z := {
    advance/1:z->z := \y -> later x y;
    before := advance 1z;
    advance before
};
!{ ^{ ^? ! print (first,total (branch (leaf 2z) (leaf 3z)),local 8z); () } };
"#,
    )
    .accepts("", "(7, 5, 17)\n");
}

#[test]
fn numeric_qualifiers_and_open_rows_reinstantiate_per_call() {
    Fixture::new(
        r#"
square/1:~a->~a | num ~a := &{ @* * . . };
get/1:.{ value:~a; | ~r }->~a := &.value;
!{ ^{
    ^? ! print (square 3n,square 0.5q,square 1.20d,
        get .{value:=7n;label:="one";},get .{flag:=1b;value:="two";});
    ()
} };
"#,
    )
    .accepts("", "(9, 1/4, 1.4400, 7, two)\n");
}

#[test]
fn qualified_export_keeps_arity_in_direct_and_first_class_calls() {
    let fixture = Fixture::new(
        r#"
<< "ops.flx" : m;
!{ ^{ ^? ! print (m.add 3z 4z,@ (m.curried 5z) 6z,@@ &m.add (2z,9z)); () } };
"#,
    );
    fixture.write(
        "ops.flx",
        r#"
>> (add,curried);
add/2:z->z->z := &+;
curried/1:z->z->z := &+;
"#,
    );
    fixture.accepts("", "(7, 11, 11)\n");
    let out = flux()
        .current_dir(fixture.path())
        .args(["interface", "ops.flx"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let interface = String::from_utf8_lossy(&out.stdout);
    assert!(
        interface.contains("callable/2 add") && interface.contains("callable/1 curried"),
        "{interface}"
    );
}

#[test]
fn effects_in_call_operands_run_once_and_in_source_order() {
    Fixture::new(
        r#"
<< "io";
pair/2:?s->?s->(?s,?s) := \a -> \b -> (a,b);
!{ ^{
    p := pair (^? ! io.readln) (^? ! io.readln);
    rest := ^? ! io.readln;
    ^? ! print (p,rest);
    ()
} };
"#,
    )
    .accepts("left\nright\nafter\n", "((?1 left, ?1 right), ?1 after)\n");
}

#[test]
fn cannot_infer_an_exported_signature_or_use_an_unlisted_rhs_shape() {
    rejects("bad/1 := 1z;", &["right-hand side"]);
    rejects("id(x:z):z := x; bad/1 := @ &id &id;", &["right-hand side"]);
    let fixture = Fixture::new("<< \"ops.flx\" : m; !{ () };");
    fixture.write("ops.flx", ">> (add); add/2 := &+;");
    let out = fixture.cli("check", "");
    let error = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success() && error.contains("signature"),
        "{error}"
    );
}

#[test]
fn validates_adjacency_arity_and_signature_boundary() {
    for source in [
        "f /1 := &+;",
        "f/ 1 := &+;",
        "f/0 := &+;",
        "f/9999999999999999999999999 := &+;",
        "f/1n := &+;",
    ] {
        rejects(source, &["main.flx:1:"]);
    }
    rejects("f/2:z->z := &+;", &["argument domains"]);
    rejects("f/3 := &+;", &["main.flx:1:"]);
    rejects("f/2:z->z->z := &+; x := f 1z;", &["operand"]);
}

#[test]
fn diagnostic_uses_the_real_file_and_rhs_span() {
    let fixture = Fixture::new(
        r#"
<< "ops.flx" : m;
!{ () };
"#,
    );
    fixture.write("ops.flx", ">> (bad);\n\nbad/1:z->s := &-:;\n");
    let out = fixture.cli("check", "");
    let error = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success()
            && error.contains("ops.flx:3:")
            && error.contains("&-:")
            && error.contains('^'),
        "{error}"
    );
}

#[test]
fn formatting_preserves_adjacent_callable_headers_and_execution() {
    let fixture = Fixture::new("plus/2:z->z->z:=&+;\n!{ ^{ ^? ! print plus 3z 4z; () } };\n");
    let formatted = fixture.cli("fmt", "");
    assert!(
        formatted.status.success(),
        "{}",
        String::from_utf8_lossy(&formatted.stderr)
    );
    let source = String::from_utf8(formatted.stdout).unwrap();
    assert!(source.starts_with("plus/2"), "{source}");
    Fixture::new(&source).accepts("", "7\n");
}

#[test]
fn all_direct_operands_finish_before_the_rhs_is_applied() {
    let fixture = Fixture::new(
        r#"
force/2:z->()->z := \a -> {
    absent:?z := ?0;
    bad := ?! absent;
    \b -> + a bad
};
!{ ^{
    unused := force 1z (^? ! print "second operand ran");
    ()
} };
"#,
    );
    assert!(fixture.cli("check", "").status.success());
    for mode in ["run-hir", ""] {
        let out = fixture.cli(mode, "");
        let error = String::from_utf8_lossy(&out.stderr);
        assert!(
            !out.status.success() && error.contains("unwrap ?0"),
            "{error}"
        );
        assert_eq!(out.stdout, b"second operand ran\n", "{mode}");
    }
}
