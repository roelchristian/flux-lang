use super::fixture::{rejects, Fixture};
use crate::support::{flux, TempDir};
use std::{fs, io::Write, process::Stdio};

#[test]
fn implicit_unit_suspends_and_reuses_actions() {
    Fixture::new(
        r#"
a := !^{ ! print "run"; };
empty:!( ()^() ) := !^{ };
!^{
    ! a;
    ! a;
    ^= ! empty { _ : (); };
    ! !{ (); };
    ! print "done";
};
"#,
    )
    .accepts("", "run\nrun\ndone\n");
}

#[test]
fn constructing_an_action_does_not_execute_it_and_keeps_explicit_results() {
    Fixture::new(
        r#"
a := !^{ ! print "not run"; };
r := !^{ 2z };
!^{
    stored := ! r;
    ! print stored;
    ! print "done";
};
"#,
    )
    .accepts("", "ok 2\ndone\n");
}

#[test]
fn explicit_values_and_unconstrained_unit_error() {
    Fixture::new(
        r#"
empty := !^{ };
readtext:!(s^ioerr) := !^{ "text" };
r := !^{ ^= ! empty { _ : (); }; () };
!^{
    ! print (^= ! readtext { _ : "bad"; });
    ! print (^= ! r { _ : (); });
};
"#,
    )
    .accepts("", "text\n()\n");
}

#[test]
fn terminal_result_is_not_automatically_flattened() {
    Fixture::new(
        r#"
a:!((z^())^()) := !^{ ! !^{ 7z } };
!^{
    nested := ! a;
    ! print nested;
};
"#,
    )
    .accepts("", "ok ok 7\n");
}

#[test]
fn explicit_exit_code_is_preserved() {
    let fixture = Fixture::new("!^{ ! print \"ok\"; 7n };");
    for mode in ["", "run-hir"] {
        let output = fixture.cli(mode, "");
        assert_eq!(output.status.code(), Some(7));
        assert_eq!(output.stdout, b"ok\n");
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn first_failure_stops_execution_and_keeps_prior_output() {
    let fixture = Fixture::new(
        r#"
bad :: { | failed; };
f:!(()^bad) := !^{ ^: failed };
!^{
    ^= ! print "before" { _ : ^: failed; };
    ! f;
    ^= ! print "after" { _ : ^: failed; };
};
"#,
    );
    for mode in ["", "run-hir"] {
        let output = fixture.cli(mode, "");
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(output.stdout, b"before\n");
        assert!(String::from_utf8_lossy(&output.stderr).contains("failed"));
    }
}

#[test]
fn non_unit_success_cannot_be_discarded() {
    rejects(
        r#"<< "io"; !^{ ! io.read "notes.txt"; };"#,
        &["non-unit success", "bind"],
    );
    rejects("!^{ ! !{ 4z }; };", &["standalone execution", "bind"]);
}

#[test]
fn nested_ordinary_actions_do_not_inherit_automatic_propagation() {
    rejects(
        r#"a := !^{ ! !{ ! print "no"; }; };"#,
        &["result-valued statement would discard"],
    );
}

#[test]
fn error_mismatch_requires_explicit_conversion() {
    rejects(
        r#"bad :: { | failed; }; f:!(()^bad) := !^{ ^: failed }; !^{ ! print "not run"; ! f; };"#,
        &["ioerr", "bad"],
    );
}

#[test]
fn formatter_and_repl_keep_the_combined_opener() {
    let dir = TempDir::new("flux-action-layout");
    let file = dir.join("case.flx");
    fs::write(&file, "!^{ ! print \"formatted\"; };").unwrap();
    let fmt = flux().arg("fmt").arg(&file).output().unwrap();
    assert!(fmt.status.success());
    assert!(fmt.stdout.starts_with(b"!^{"));
    fs::write(&file, &fmt.stdout).unwrap();
    assert!(flux()
        .args(["fmt", "--check"])
        .arg(&file)
        .output()
        .unwrap()
        .status
        .success());
    let repl = crate::support::flux()
        .arg("run-hir")
        .arg(&file)
        .output()
        .unwrap();
    assert_eq!(repl.stdout, b"formatted\n");
    let mut session = flux()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    session
        .stdin
        .take()
        .unwrap()
        .write_all(b"!^{\n ! print \"repl\";\n};\n:quit\n")
        .unwrap();
    let output = session.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"repl\n");
    assert!(output.stderr.is_empty());
    rejects("! ^{ (); };", &["action execution"]);
}

#[test]
fn combined_actions_enforce_eager_initialization_and_keep_nested_boundaries() {
    let f = Fixture::new("!^{ x := value 0; y := 2; value(n) := y; ! print x; };");
    let o = f.cli("check", "");
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("initializer"));

    Fixture::new(
        r#"
bad :: { | failed; };
a:!(()^bad) := !^{ ^: failed };
!^{
    r := ! a;
    ^= r { failed : (); };
    ! print "recovered";
};
"#,
    )
    .accepts("", "recovered\n");
}

#[test]
fn default_only_after_contextual_error_constraints_have_resolved() {
    Fixture::new(
        r#"
a:!(()^ioerr) := !^{ ! !^{ }; };
empty/1:()->!(()^~e) := \x -> !^{ };
!^{
    ! a;
    ! !^{ };
    ! (? 1b !^{ } !^{ ! print "unreachable"; });
    value := ^= ! !^{ 3z } { _ : 0z; };
    ! empty ();
    ! print value;
};
"#,
    )
    .accepts("", "3\n");
}

#[test]
fn local_callable_constraints_and_closure_captures_precede_defaulting() {
    Fixture::new(
        r#"
wrap(cb) := !^{ a := ? 1b !^{ } cb; ! a; };
s:!(()^ioerr) := wrap (!^{ ! print "unused"; });
!^{
    aa(n) := ? 1b !^{ } (bb n);
    bb(n) := !^{ ! print n; };
    ! aa 1z;
    ! s;
    ! print "done";
};
"#,
    )
    .accepts("", "done\n");
}
