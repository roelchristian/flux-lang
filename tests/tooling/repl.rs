use crate::support::{flux, TempDir};
use std::{
    fs,
    io::Write as _,
    process::{Output, Stdio},
};

pub(super) fn repl_output(input: &str) -> Output {
    let mut child = flux()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn repl(input: &str) -> (String, String) {
    let output = repl_output(input);
    assert!(output.status.success());
    (
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

#[test]
fn type_does_not_run_effects_and_conversion_is_not_quit() {
    let (out, err) = repl(":type ! io.print \"not executed\"\n:q 5\n- 5n 2n\n:q\n+ 10 20\n");
    assert!(err.is_empty(), "{err}");
    assert_eq!(out, "()^ioerr\n5 : q\n3 : n\n");
}

#[test]
fn help_history_replay_and_timing() {
    let (out, err) = repl(":help\n+ 2n 3n\n:type 8n\n:time * 2n 3n\n:history\n:replay 1\n:quit\n");
    assert!(err.is_empty(), "{err}");
    assert!(out.contains(":schema EXPR"));
    assert!(out.contains("5 : n\nn\n6 : n\nelapsed: "));
    assert!(out.contains("1  + 2n 3n\n2  * 2n 3n\n5 : n\n"));
}

#[test]
fn load_is_atomic_and_does_not_dispatch_a_script_entry() {
    let fixture = TempDir::new("repl");
    let declarations = fixture.join("declarations with spaces.flx");
    let script = fixture.join("script with spaces.flx");
    fs::write(&declarations, "seed:n := 8;\n").unwrap();
    fs::write(
        &script,
        "poison:n := 2;\n!{ ^= ! io.print \"not executed\" { _ : (); }; };\n",
    )
    .unwrap();
    let (out, err) = repl(&format!(
        ":load \"{}\"\nseed\n:load {}\npoison\nseed\n:quit\n",
        declarations.display(),
        script.display(),
    ));
    assert!(out.contains("loaded "));
    assert_eq!(out.matches("8 : n").count(), 2);
    assert!(!out.contains("not executed"));
    assert!(
        err.contains("declarations only") && err.contains("poison"),
        "{err}"
    );
}

#[test]
fn schema_reports_rows_and_rejects_scalar_expressions() {
    let (out, err) = repl(concat!(
        "observation :: { name:s; amount:q; };\n",
        "table:#observation := #[observation { name:=\"one\"; amount:=2q; }];\n",
        ":schema table\ntable\n:schema 2n\n:quit\n"
    ));
    assert!(
        out.contains("name") && out.contains("amount") && out.contains("#observation"),
        "{out}"
    );
    assert!(
        out.contains("name | amount\n") && out.contains("one  | 2"),
        "{out}"
    );
    assert!(err.contains(":schema expects a table; found n"), "{err}");
}

#[test]
fn inline_comments_preserve_submission_boundaries() {
    let (out, err) = repl("k := 7n; -- declaration\n+ k 1n; -- expression\n:quit\n");
    assert!(err.is_empty(), "{err}");
    assert_eq!(out, "8 : n\n");
}

#[test]
fn load_resolves_imports_relative_to_each_fragment_without_executing_actions() {
    let root = TempDir::new("repl-modules");
    let a = root.join("a");
    let b = root.join("b");
    fs::create_dir(&a).unwrap();
    fs::create_dir(&b).unwrap();
    for (dir, n, alias, name) in [(&a, 3, "liba", "ax"), (&b, 8, "libb", "bx")] {
        fs::write(
            dir.join("lib.flx"),
            format!("<< \"io\";\n>> (x);\nx:n := {n};\nheld := io.print \"not executed\";\n"),
        )
        .unwrap();
        fs::write(
            dir.join("session.flx"),
            format!("<< \"lib.flx\" : {alias};\n{name}:n := {alias}.x;\n"),
        )
        .unwrap();
    }
    let (out, err) = repl(&format!(
        ":load \"{}\"\n:load \"{}\"\n+ ax bx\n:quit\n",
        a.join("session.flx").display(),
        b.join("session.flx").display(),
    ));
    assert!(err.is_empty(), "{err}");
    assert!(out.ends_with("11 : n\n"), "{out}");
    assert!(!out.contains("not executed"), "{out}");
}
