use crate::support::{flux, TempDir};
use std::{fs, process::Output};

struct Script(TempDir);

impl Script {
    fn new(source: &str) -> Self {
        let dir = TempDir::new("flux-typed-errors");
        fs::write(dir.join("test.flx"), source).expect("write isolated script");
        Self(dir)
    }

    fn cli(&self, mode: &str) -> Output {
        flux()
            .current_dir(self.0.path())
            .args([mode, "test.flx"])
            .output()
            .expect("invoke public CLI")
    }
}

fn strict(script: &Script, expected: &str) {
    for mode in ["check", "hir", "run-hir"] {
        let out = script.cli(mode);
        assert!(
            out.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        match mode {
            "check" => assert!(out.stdout.is_empty(), "checking ran effects"),
            "hir" => assert!(String::from_utf8_lossy(&out.stdout).contains("HirProgram")),
            _ => assert_eq!(String::from_utf8_lossy(&out.stdout), expected),
        }
    }
}

fn rejected(source: &str, expected: &[&str]) {
    let script = Script::new(source);
    for mode in ["check", "hir"] {
        let out = script.cli(mode);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !out.status.success()
                && !stderr.contains("E0400")
                && expected.iter().any(|fragment| {
                    stderr
                        .to_ascii_lowercase()
                        .contains(&fragment.to_ascii_lowercase())
                }),
            "{mode}: {stderr}\n{source}"
        );
    }
}

const DIV: &str = r#"
<< "io";
diverr :: { | zero; };
div(x,y:q):q^diverr := ^{
    ?{ = y 0 : ^: zero; _ : / x y; }
};
emit(x) := !{ ^= ! io.print x { _ : (); } };
"#;

#[test]
fn success_bypasses_recovery_and_caret_arithmetic_is_unchanged() {
    let source = format!(
        r#"{DIV}
!{{
    ! emit ^= div 12 3 {{ zero : / 1q 0q; }};
    ! emit ^= div 12 0 {{ zero : 9q; }};
    ! emit ^ 2 3;
    ! emit ^/ 81;
}};"#
    );
    strict(&Script::new(&source), "4\n9\n8\n9\n");
}

#[test]
fn nested_results_and_lexically_nested_boundaries_are_distinct() {
    let source = format!(
        r#"{DIV}
inner(x:q):q^diverr := ^{{ ^? div 12 x }};
outer(x:q):(q^diverr)^diverr := ^{{ inner x }};
!{{
    first := ^= outer 0 {{ zero : div 99 3; }};
    ! emit ^= first {{ zero : 7q; }};
    ! emit ^= ^= outer 6 {{ zero : div 99 3; }} {{ zero : 7q; }};
}};"#
    );
    strict(&Script::new(&source), "7\n2\n");
}

#[test]
fn nominal_recovery_matches_constructor_and_binds_payload_once() {
    let source = r#"<< "io";
err :: { | plain; | detail s; };
make(x:s):s^err := ^{ ^: detail x };
emit(x) := !{ ^= ! io.print x { _ : (); } };
!{ ! emit ^= make "original" { plain : "wrong"; detail msg : msg; }; };"#;
    strict(&Script::new(source), "original\n");
}

#[test]
fn unmatched_lexical_targets_and_discarded_results_are_static_errors() {
    rejected("x := ^: 1z;", &["boundary", "outside"]);
    rejected("x := ^? ^{ 1z };", &["boundary", "outside"]);
    rejected(
        "f(x:z):z^z := ^{ g(y:z):z := ^: y; g x };",
        &["boundary", "outside"],
    );
    rejected(
        "f(x:z):z^z := ^{ g := \\y -> ^: y; @ g x };",
        &["boundary", "outside"],
    );
    rejected("f(x:z):(!z)^z := ^{ !{ ^: x } };", &["boundary", "outside"]);
    rejected(
        &format!("{DIV}!{{ div 12 3; () }};"),
        &["discard", "unhandled", "unused"],
    );
}

#[test]
fn invalid_recovery_arms_cannot_be_accepted_by_fallback() {
    let base = r#"err :: { | plain; | detail s; };
mk(x:s):s^err := ^{ ^: detail x };"#;
    for (arms, expected) in [
        ("plain : \"a\";", &["exhaust", "missing"][..]),
        (
            "_ : \"a\"; detail msg : msg;",
            &["unreach", "last", "catch-all"][..],
        ),
        (
            "detail : \"a\"; _ : \"b\";",
            &["arity", "payload", "bind", "expected identifier"][..],
        ),
        (
            "plain : \"a\"; plain : \"b\"; _ : \"c\";",
            &["duplicate", "unreach"][..],
        ),
    ] {
        rejected(&format!("{base}\nx := ^= mk \"x\" {{ {arms} }};"), expected);
    }
}

#[test]
fn propagation_keeps_completed_output_and_original_failure_context() {
    let source = format!(
        r#"{DIV}
work(x:q):!(q^diverr) := !{{
    ^{{
        printed := ! emit "before";
        ^? div 12 x
    }}
}};
!{{
    ^{{
        value := ^? ! work 0;
        printed := ! emit value;
        ()
    }}
}};"#
    );
    let fixture = Script::new(&source);
    let check = fixture.cli("check");
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(check.stdout.is_empty());
    let out = fixture.cli("run-hir");
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "before\n");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("diverr") && stderr.contains("zero"),
        "{stderr}"
    );
    assert!(
        stderr.contains("test.flx:5:"),
        "missing original raise locus: {stderr}"
    );
    assert_eq!(stderr.matches("propagated here").count(), 2, "{stderr}");
}

#[test]
fn fallible_typed_csv_can_recover_and_preserves_empty_headers() {
    let source = r#"
<< "csv";
row :: { name:s; value:q; };
empty:#row := #[];
!{
    ^{
        rows:#row := ^= ! csv.read "not-present.csv" { _ : empty; };
        written := ^? ! csv.write "out.csv" rows;
        ()
    }
};
"#;
    let fixture = Script::new(source);
    strict(&fixture, "");
    let text = fs::read_to_string(fixture.0.join("out.csv")).expect("output CSV");
    assert_eq!(text.replace("\r\n", "\n"), "name,value\n");
}

#[test]
fn mapping_fallible_elements_does_not_short_circuit() {
    let source = format!(
        r#"{DIV}
each(x:q):q^diverr := div 12 x;
recover(x:q^diverr):q := ^= x {{ zero : 0q; }};
!{{ ! emit @: &recover (@: &each [3q 0q 6q]); }};"#
    );
    strict(&Script::new(&source), "[4 0 2]\n");
}

#[test]
fn ordinary_domain_error_is_not_caught_by_typed_recovery() {
    let source = format!(
        r#"{DIV}
fatal(x:q):q^diverr := ^{{ / 1q x }};
!{{ ! emit ^= fatal 0 {{ zero : 7q; }}; }};"#
    );
    let f = Script::new(&source);
    let check = f.cli("check");
    assert!(
        check.status.success(),
        "check: {}",
        String::from_utf8_lossy(&check.stderr)
    );
    let out = f.cli("run-hir");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success() && out.stdout.is_empty(), "{stderr}");
    assert!(
        stderr.contains("division") && !stderr.contains("R0101") && !stderr.contains("E0400"),
        "{stderr}"
    );
}

#[test]
fn checked_exact_division_recovers_zero_without_catching_fatal_errors() {
    let source = r#"<< "math";
<< "io";
emit(x) := !{ ^= ! io.print x { _ : (); } };
!{
    ! emit ^= math.div 1q 2q { divzero : 0q; range : 0q; };
    ! emit ^= math.div 1q 0q { divzero : 7q; range : 0q; };
};"#;
    strict(&Script::new(source), "1/2\n7\n");
}

const CSV_WORKFLOW: &str = r#"
<< "csv";
obs :: { name:s; value:q; };
err :: { | source csverr; | empty; };
load(p:s):!(#obs^err) := !{
    ^{
        t:#obs := ^= ! csv.read p { e : ^: source e; };
        ?{ = (# t) 0 : ^: empty; _ : t; }
    }
};
!{
    ^{
        t := ^? ! load "input.csv";
        avg .value t;
        ()
    }
};
"#;

#[test]
fn requested_workflow_checks_runs_and_translates_csv_failure() {
    let valid = Script::new(CSV_WORKFLOW);
    fs::write(valid.0.join("input.csv"), "name,value\nexample,0.5\n").expect("valid CSV");
    strict(&valid, "");

    for (file, constructor) in [(None, "source"), (Some("name,value\n"), "empty")] {
        let f = Script::new(CSV_WORKFLOW);
        if let Some(contents) = file {
            fs::write(f.0.join("input.csv"), contents).expect("CSV fixture");
        }
        assert!(f.cli("check").status.success());
        let out = f.cli("run-hir");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{stderr}");
        assert!(
            stderr.contains(constructor) && !stderr.contains("E0400"),
            "{stderr}"
        );
    }
}

#[test]
fn malformed_csv_binds_cell_payload_and_does_not_become_optional_absence() {
    let source = r#"
<< "csv";
<< "io";
row :: { name:s; value:?q; };
empty:#row := #[];
!{
    parsed:#row := ^= ! csv.read "cells.csv" {
        cellerr r col bad : ?{
            && (= r 3n) (= col "value") : #[row { name := bad; value := ?0; }];
            _ : empty;
        };
        _ : empty;
    };
    ^= ! io.print .name parsed { _ : (); };
};
"#;
    let f = Script::new(source);
    fs::write(
        f.0.join("cells.csv"),
        "name,value\n\"missing\nname\",\ninvalid,oops\n",
    )
    .expect("CSV fixture");
    strict(&f, "[oops]\n");
}

#[test]
fn creating_fallible_actions_is_inert_and_read_error_is_typed() {
    let source = r#"
<< "io";
pending := io.write "never.txt" "not executed";
!{
    text := ^= ! io.read "missing.txt" { missing path : path; _ : "other"; };
    ^= ! io.print text { _ : (); };
    ()
};
"#;
    let f = Script::new(source);
    strict(&f, "missing.txt\n");
    assert!(!f.0.join("never.txt").exists());
}

#[test]
fn source_using_new_carets_never_falls_back_to_legacy() {
    let f = Script::new(&format!("{DIV}\n!{{ var [1q 2q]; () }};"));
    let typed = f.cli("hir");
    let expected = String::from_utf8_lossy(&typed.stderr);
    if !typed.status.success() {
        assert!(expected.contains("E0400"), "{expected}");
        let compatible = f.cli("check");
        assert!(
            !compatible.status.success()
                && String::from_utf8_lossy(&compatible.stderr).contains("E0400"),
            "check incorrectly accepted a typed-only program through legacy fallback"
        );
    }
}

#[test]
fn result_type_grouping_is_not_implicitly_reassociated() {
    rejected("x:q^s^z := ^{ 1q };", &["caret", "parenthes", "result"]);
    rejected(
        "bad(x:q):q^s := ^{ ^? ^{ ^: x } };",
        &["unify", "incompatible", "error"],
    );
}

#[test]
fn legacy_handler_spelling_is_not_reintroduced() {
    rejected("!{ ! !> 1; };", &["retired", "not supported", "unexpected"]);
}

#[test]
fn a_pipeline_must_use_or_explicitly_handle_a_result_stage() {
    rejected(
        &format!("{DIV}x := |{{ div 12 3; () }};"),
        &["discard", "unused", "unhandled", "result"],
    );
    let source = format!(
        r#"{DIV}
via(x:q):q^diverr := ^{{ |{{ div 12 x; ^? $; + $ 1q }} }};
!{{
    ! emit ^= via 3 {{ zero : 0q; }};
    ! emit ^= via 0 {{ zero : 0q; }};
}};"#
    );
    strict(&Script::new(&source), "5\n0\n");
}

#[test]
fn io_csv_aliases_expose_the_same_errors_and_preserve_no_clobber() {
    let source = r#"
<< "io";
row :: { name:s; };
load(p:s):!(#row^csverr) := io.read_csv p;
tbl:#row := #[row { name := "should-not-replace"; }];
!{
    absent := ^= ! io.exists "missing.csv" { _ : 1b; };
    ^= ! io.print absent { _ : (); };
    selected:#row := ^= ! load "missing.csv" {
        fileerr _ : tbl;
        _ : #[];
    };
    ^= ! io.write_csv "existing.csv" selected {
        fileerr error : ^= ! io.print error { _ : (); };
        _ : ();
    };
};
"#;
    let f = Script::new(source);
    let original = "name\noriginal\n";
    fs::write(f.0.join("existing.csv"), original).expect("existing CSV");
    strict(&f, "0b\nexists existing.csv\n");
    assert_eq!(
        fs::read_to_string(f.0.join("existing.csv")).expect("original CSV"),
        original
    );
}

#[test]
fn typed_csv_error_precedence_is_structural_then_schema_order() {
    let source = r#"
<< "csv";
row :: { left:q; right:q; };
!{ ^{ parsed:#row := ^? ! csv.read "input.csv"; () } };
"#;
    for (data, expected) in [
        ("right,left\nbad,bad\nshort\n", "rowerr 3"),
        ("right,left\nrightbad,leftbad\n", "cellerr 2 left leftbad"),
    ] {
        let f = Script::new(source);
        fs::write(f.0.join("input.csv"), data).expect("CSV records");
        assert!(f.cli("check").status.success());
        let out = f.cli("run-hir");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{stderr}");
        assert!(stderr.contains(expected), "{stderr}");
    }
}

#[test]
fn old_csv_assignment_gets_an_explicit_migration_diagnostic() {
    let source = r#"
<< "csv";
row :: { value:z; };
!{ t:#row := ! csv.read "input.csv"; };
"#;
    rejected(source, &["E0401"]);
    let out = Script::new(source).cli("check");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("without handling its error"));
    assert!(stderr.contains("use ^= to recover or ^? inside a fallible boundary"));
}
