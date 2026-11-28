use super::fixture::{rejects, Fixture};
use crate::support::flux;

#[test]
fn independent_projections_preserve_heterogeneous_arguments_and_generalize() {
    Fixture::new(
        r#"
first/2 := &{2: $1 };
second := &{2: $2 };
!{
    ^{ ^? ! print (first 7 "unused",first "x" 9,@ (@ second 0) 1b,@ (@ second "a") [9]); () }
};
"#,
    )
    .accepts("", "(7, x, 1b, [9])\n");
}

#[test]
fn constants_capture_lexically_and_do_not_run_suspended_actions() {
    Fixture::new(
        r#"
k := 10;
f := &{1: @* + $1 (@= k) };
action := &{1: @= print "action" };
!{ ^{
    k := 20;
    ^? ! print @ f 2;
    a := @ action 99;
    ^? ! a;
    ()
} };
"#,
    )
    .accepts("", "12\naction\n");
}

#[test]
fn expansive_constants_are_eager_not_generalized_and_body_waits_for_saturation() {
    let early = Fixture::new(
        r#"
f := &{2: @* + $1 (@= // 1 0) };
!{ ^{ ^? ! print ( @ f 1 ); () } };
"#,
    );
    for mode in ["", "run-hir"] {
        let o = early.cli(mode, "");
        assert!(!o.status.success() && o.stdout.is_empty());
        assert!(String::from_utf8_lossy(&o.stderr).contains("zero"));
    }
    Fixture::new(
        r#"
bad(x:z):z := // x 0;
f := &{2: @. bad $2 };
!{ ^{ p := @ f 1; ^? ! print "partial"; () } };
"#,
    )
    .accepts("", "partial\n");
    rejects(
        r#"
choose(x) := x;
f := &{1: @= @ &choose (\x -> x) };
g := @ f 0;
one := @ g 1;
two := @ g "text";
"#,
        &["cannot unify"],
    );
}

#[test]
fn nested_composition_and_generalized_forks_match_named_functions() {
    Fixture::new(
        r#"
sq/1 := &{ @* * . . };
s(a,b,c) := + a + b c;
t/3 := &{3: @{ s; @. sq $1; @. sq $2; @. sq $3 } };
sub2/2 := &{2: @* - $2 $1 };
after/2 := &{2: @. sq (@* - $1 $2) };
named(a,b,c:q):q := + (sq a) + (sq b) (sq c);
!{ ^{ ^? ! print (== (t 2q 3q 4q) (named 2q 3q 4q),sub2 10 3,after 8 3); () } };
"#,
    )
    .accepts("", "(1b, -7, 25)\n");
}

#[test]
fn named_arity_is_independent_of_tacit_arity() {
    Fixture::new(
        r#"
add/1:z->z->z := &{2: @* + $1 $2 };
neg_add/2:z->z->z := &{2: @. &{ -: } (@* + $1 $2) };
!{ ^{ ^? ! print (@ (add 12) 8,neg_add 2 3); () } };
"#,
    )
    .accepts("", "(20, -5)\n");
}

#[test]
fn local_module_exports_report_direct_arity() {
    let f = Fixture::new(
        r#"
<< "./f.flx" : f;
!{ ^{ ^? ! print (f.second "unused" 3); () } };
"#,
    );
    f.write("f.flx", ">> (second); second/2 : ~a->~b->~b := &{2: $2 };");
    f.accepts("", "3\n");
}

#[test]
fn branches_execute_left_to_right_before_the_combiner() {
    let f = Fixture::new(
        r#"
lhs(x:z):z := // x 0z;
rhs(x:z):z := ?! (?0);
combine(x,y:z):z := // x 0z;
fold/2 := &{2: @{ combine; @. lhs $1; @. rhs $2 } };
!{ ^{ ^? ! print (fold 1 2); () } };
"#,
    );
    for mode in ["", "run-hir"] {
        let out = f.cli(mode, "");
        assert!(!out.status.success());
        assert!(out.stdout.is_empty());
        let diag = String::from_utf8_lossy(&out.stderr);
        assert!(diag.contains("division by zero"), "{diag}");
        assert!(diag.contains(":2:"), "{diag}");
    }
}

#[test]
fn invalid_contextual_tokens_have_static_diagnostics_and_pipeline_still_works() {
    for s in [
        "f := &{0: . };",
        "f := &{2z: $1 };",
        "f := &{2: $0 };",
        "f := &{2: $3 };",
        "f := &{2: $1z };",
        "f := &{2: $ 1 };",
        "f := &{2: . };",
        "f := &{2: @? + $1 };",
        "f := &{2: @{ + } };",
        "f := &{1: @= ! print 3 };",
    ] {
        let f = Fixture::new(s);
        let out = f.cli("check", "");
        assert!(!out.status.success(), "unexpected acceptance: {s}");
        assert!(!out.stderr.is_empty(), "{s}");
    }
    Fixture::new(
        r#"
f/2 := &{2: @* + $1 $2 };
!{ ^{ ^? ! print |{ 1; f $ 2 }; () } };
"#,
    )
    .accepts("", "3\n");
}

#[test]
fn formatter_keeps_contextual_adjacency_and_ordinary_compact_tokens() {
    let f = Fixture::new(
        r#"
tri(a,b,c) := + a + b c;
p/2 := &{2: @{ tri; $1; $2; @= 3 } };
!{ ^{ ^? ! print p 1 2; () } };
"#,
    );
    let out = f.cli("fmt", "");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    for token in ["$1", "$2", "@=", "@{"] {
        assert!(text.contains(token), "{text}");
    }
    f.write("main.flx", &text);
    let result = flux()
        .args(["fmt", "--check", "main.flx"])
        .current_dir(f.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    f.accepts("", "6\n");
    let ordinary = Fixture::new("f := @{ &. } 7; !{ ^{ ^? ! print f; () } };");
    ordinary.accepts("", "7\n");
}
