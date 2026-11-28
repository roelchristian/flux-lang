use super::repl::repl_output;
use crate::support::{flux, TempDir};
use std::{fs, path::PathBuf, process::Output};

struct Script(TempDir);

impl Script {
    fn new(source: &str) -> Self {
        let script = Self(TempDir::new("cli"));
        fs::write(script.path(), source).unwrap();
        script
    }

    fn path(&self) -> PathBuf {
        self.0.join("test.flx")
    }

    fn invoke(&self, args: &[&str]) -> Output {
        flux().args(args).arg(self.path()).output().unwrap()
    }
}

fn text(stdout: &[u8]) -> &str {
    std::str::from_utf8(stdout).unwrap()
}

#[test]
fn anonymous_entry_runs_once_instead_of_main() {
    let file = Script::new(
        r#"
<< "io" : io;
main:!() := !{ ^= ! io.print "do not run" { _ : (); }; };
!{ ^= ! io.print "run once" { _ : (); }; };
"#,
    );
    let checked = file.invoke(&["check"]);
    assert!(checked.status.success(), "{}", text(&checked.stderr));
    assert!(checked.stdout.is_empty(), "check unexpectedly ran effects");
    let run = file.invoke(&[]);
    assert!(run.status.success(), "{}", text(&run.stderr));
    assert_eq!(text(&run.stdout), "run once\n");
}

#[test]
fn natural_entry_results_and_delimiter_errors_are_distinct() {
    assert_eq!(Script::new("!{ 17 };").invoke(&[]).status.code(), Some(17));
    let out_of_range = Script::new("!{ 256 };").invoke(&[]);
    assert_eq!(out_of_range.status.code(), Some(1));
    assert!(!out_of_range.stderr.is_empty());
    for source in ["!{ }; x := 1;", "!{ }; !{ };", "!{ }"] {
        let output = Script::new(source).invoke(&["check"]);
        assert!(!output.status.success(), "unexpectedly accepted {source}");
    }
}

#[test]
fn tuple_spread_runs_and_single_application_can_take_a_tuple() {
    let file = Script::new(
        r#"
<< "io" : io;
add(x,y:q):q := + x y;
one(x:(q,q)):q := + .1 x .2 x;
!{
    args := (2q,3q);
    ^= ! io.print @@ &add args { _ : (); };
    ^= ! io.print @ &one (7q,8q) { _ : (); };
};
"#,
    );
    let output = file.invoke(&[]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "5\n15\n");
}

#[test]
fn non_ascii_error_reports_byte_position_via_cli() {
    let output = Script::new("x := 1;\n-- comment eé\n").invoke(&["check"]);
    assert!(!output.status.success());
    assert!(
        text(&output.stderr).contains(":2:13:"),
        "{}",
        text(&output.stderr)
    );
    assert!(text(&output.stderr).contains("ASCII"));
}

#[test]
fn check_rejects_retired_core_io_spelling() {
    let output = Script::new(r#"!{ !> "do not run"; };"#).invoke(&["check"]);
    assert!(!output.status.success());
    let stderr = text(&output.stderr);
    assert!(
        stderr.contains("qualified module action") && stderr.contains("retired"),
        "{stderr}"
    );
}

#[test]
fn repl_does_not_automatically_execute_suspended_action() {
    let output =
        repl_output("a := !{ ^= ! io.print \"marker-effect\" { _ : (); }; };\na\n! a\n:q\n");
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(output.stderr.is_empty(), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stdout).matches("marker-effect").count(),
        1,
        "{}",
        text(&output.stdout)
    );
}

#[test]
fn floor_quotient_is_not_a_comment_and_calls_stay_prefix() {
    let file = Script::new(
        r#"
<< "io" : io;
-- floor quotient with a separate adjacent-sign literal
!{ ^= ! io.print // -7 2 { _ : (); }; };
"#,
    );
    let output = file.invoke(&[]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "-4\n");
    let infix = Script::new("!{ 2 + 3 };").invoke(&["check"]);
    assert!(!infix.status.success());
}

#[test]
fn fmt_normalizes_whitespace_without_editing_input() {
    let file = Script::new("-- comment  \r\n!{ 7 }; \r\n");
    let output = file.invoke(&["fmt"]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "-- comment\n!{ 7 };\n");
    assert_eq!(
        fs::read_to_string(file.path()).unwrap(),
        "-- comment  \r\n!{ 7 }; \r\n"
    );
}
