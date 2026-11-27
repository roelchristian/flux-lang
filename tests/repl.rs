use std::fs;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

fn repl(input: &str) -> (String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_flux"))
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
    let output = child.wait_with_output().unwrap();
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
    assert_eq!(out, "()\n5 : q\n3 : n\n");
}

#[test]
fn help_history_replay_and_timing() {
    let (out, err) = repl(":help\n+ 2n 3n\n:type 8n\n:time * 2n 3n\n:history\n:replay 1\n:quit\n");
    assert!(err.is_empty(), "{err}");
    assert!(out.contains(":schema EXPR"));
    assert!(out.contains("5 : n\nn\n6 : n\nelapsed: "));
    assert!(out.contains("1  + 2n 3n\n2  * 2n 3n\n5 : n\n"));
}

static SERIAL: AtomicUsize = AtomicUsize::new(0);
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
fn file(src: &str) -> Temporary {
    loop {
        let i = SERIAL.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("flux repl {} {i}.flx", std::process::id()));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                f.write_all(src.as_bytes()).unwrap();
                return Temporary(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => panic!("{e}"),
        }
    }
}

#[test]
fn load_is_atomic_and_does_not_dispatch_a_script_entry() {
    let declarations = file("seed:n := 8;\n");
    let script = file("poison:n := 2;\n!{ ! io.print \"not executed\"; };\n");
    let (out, err) = repl(&format!(
        ":load \"{}\"\nseed\n:load {}\npoison\nseed\n:quit\n",
        declarations.0.display(),
        script.0.display(),
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
