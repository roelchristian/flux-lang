use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        loop {
            let root = std::env::temp_dir().join(format!(
                "flux-draft07-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            match fs::create_dir(&root) {
                Ok(()) => return Self(root),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("create fixture directory: {e}"),
            }
        }
    }

    fn write(&self, path: &str, data: &str) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).expect("module directory");
        fs::write(path, data).expect("write module fixture");
    }

    fn cli(&self, from: &str, command: &str, root: impl AsRef<Path>) -> Output {
        let mut app = Command::new(env!("CARGO_BIN_EXE_flux"));
        app.current_dir(self.0.join(from));
        if !command.is_empty() {
            app.arg(command);
        }
        app.arg(root.as_ref()).output().expect("invoke Flux CLI")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn accept(f: &Fixture, from: &str, path: &str, expected: &str) {
    for cmd in ["check", "hir", "run-hir", ""] {
        let out = f.cli(from, cmd, path);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{cmd} {path}: {stderr}");
        match cmd {
            "check" => assert!(stdout.is_empty(), "check executed effects"),
            "hir" => assert!(stdout.contains("HirProgram"), "missing typed HIR"),
            _ => assert_eq!(stdout, expected, "{cmd} {path}"),
        }
    }
}

fn reject(f: &Fixture, path: &str, fragments: &[&str]) -> String {
    let mut primary = String::new();
    for cmd in ["check", "run-hir", ""] {
        let out = f.cli("", cmd, path);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !out.status.success() && out.stdout.is_empty(),
            "{cmd}: {out:?}"
        );
        assert!(
            fragments.iter().all(|part| stderr.contains(part)),
            "{cmd}: expected {fragments:?}, got {stderr}",
        );
        if cmd == "check" {
            primary = stderr.into_owned();
        }
    }
    primary
}

#[test]
fn examples_run_in_both_modes() {
    let f = Fixture::new();
    f.write("tacit.flx", include_str!("fixtures/pre-corpus/tacit.flx"));
    f.write(
        "finance.flx",
        include_str!("fixtures/pre-corpus/tacit-finance.flx"),
    );
    accept(&f, "", "tacit.flx", "(10, 13, 12, identity)\n");
    accept(&f, "", "finance.flx", "([C], [0.00 0.00 0.02])\n");
}

#[test]
fn local_module_exports_can_carry_typed_tacit_values() {
    let f = Fixture::new();
    f.write(
        "app/lib.flx",
        r#">> (id, add_cents);
id(x:'a):'a := @ &. x;
levy(x:d):d := + x 0.005d;
tenfold(x:d):d := * x 10d;
add_cents:d->d := &{ @. levy tenfold };"#,
    );
    f.write(
        "app/main.flx",
        r#"<< "lib.flx" : ext;
!{ ^= ! print (ext.id "salve",ext.id 12z,@ ext.add_cents 9.995d) { _ : (); }; };"#,
    );
    accept(&f, "", "app/main.flx", "(salve, 12, 99.955)\n");
}

#[test]
fn generic_identity_open_rows_and_checked_numeric_lifting() {
    let f = Fixture::new();
    f.write(
        "script.flx",
        r#"
sale :: { revenue:d; cost:d; tax:d; };
gain := &{ @* - .revenue (@* + .cost .tax) };
gain_sale:sale->d := &{ @* - .revenue (@* + .cost .tax) };
identity := &{ . };
square_sum:[d]->d := &{ @. +/ (@* * . .) };
!{
    ^= ! print (@ identity 4z,
                @ identity "salve",
                @ gain_sale sale { revenue := 9.00d; cost := 1.20d; tax := 0.500d; },
                @ gain .{ revenue := 9z; cost := 1z; tax := 2z; extra := "open row"; },
                @ square_sum [1.20d 2.0d]) { _ : (); };
};
"#,
    );
    accept(&f, "", "script.flx", "(4, salve, 7.300, 6, 5.4400)\n");
}

#[test]
fn strict_prefix_grammar_does_not_admit_application_or_trains_in_tacit_form() {
    let f = Fixture::new();
    for source in [
        "value := .;",
        "value := @. &+ &-;",
        "value := @* &+ &. &.;",
        "value := @? &+ &.;",
        "value := &{ + . . };",
        "value := &{ . . };",
        "value := &{ @. . };",
        "value := &{ @* + . };",
        "value := &{ @? + };",
        "value := &{ @ &. . };",
        "value := &{ &. };",
        "value := &{ &+ };",
        "value := &{ ! };",
        "value := &{ $ };",
        "value := &{ 12z };",
    ] {
        f.write("script.flx", &format!("{source}\n!{{ () }};"));
        reject(&f, "script.flx", &["script.flx:1:"]);
    }
}

#[test]
fn compact_ordinary_application_keeps_existing_selectors_and_guards() {
    let f = Fixture::new();
    f.write(
        "script.flx",
        r#"
inc(x:z):z := + x 1;
dec(x:z):z := - x 1;
box := .{ fn := &inc; };
a := @.fn box 4z;
b := @?{ 1b : &inc; _ : &dec; } 4z;
c := @?! (?1 &inc) 4z;
!{ ^= ! print (a,b,c) { _ : (); }; };
"#,
    );
    accept(&f, "", "script.flx", "(5, 5, 5)\n");
}

#[test]
fn malformed_tacit_nodes_have_targeted_diagnostics() {
    let f = Fixture::new();
    for (expression, expected) in [
        ("&{ @. +/ }", "composition requires two function operands"),
        ("&{ @* + . }", "fork requires three function operands"),
        ("&{ @? + }", "hook requires two function operands"),
        ("&{ +/ # }", "unexpected additional tacit operand"),
        ("&{ ! }", "effect execution is not a tacit function"),
        ("&{ $ }", "pipeline placeholder is not tacit identity"),
    ] {
        f.write("script.flx", &format!("value := {expression};"));
        reject(&f, "script.flx", &[expected]);
    }
}

#[test]
fn fork_fails_at_the_left_branch_before_evaluating_the_right() {
    let f = Fixture::new();
    f.write(
        "script.flx",
        r#"left(x:z):q := / x 0z;
right(x:z):q := / x 0z;
fork := &{ @* + left right };
!{ ^= ! print @ fork 1z { _ : (); }; };
"#,
    );
    let checked = f.cli("", "check", "script.flx");
    assert!(checked.status.success(), "{checked:?}");
    for command in ["", "run-hir"] {
        let out = f.cli("", command, "script.flx");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success() && out.stdout.is_empty(), "{out:?}");
        assert!(
            stderr.contains("script.flx:1:") && !stderr.contains("script.flx:2:"),
            "{command}: {stderr}"
        );
    }
}

#[test]
fn a_tacit_action_constructor_remains_suspended_and_result_provenance_survives() {
    let f = Fixture::new();
    f.write(
        "script.flx",
        r#"<< "math";
twice(x:d):d := * x 2d;
zero(x:d):d := 0.00d;
checked_div := &{ @* math.div twice zero };
make_action:d->!(()^ioerr) := &{ @. print twice };
pending := @ make_action 7.0d;
!{
    ^= ! @ make_action 2.0d { _ : (); };
    ^{ result := @ checked_div 9.00d; ^? result; () }
};
"#,
    );
    for command in ["check", "hir"] {
        let out = f.cli("", command, "script.flx");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{command}: {stderr}");
        assert!(!String::from_utf8_lossy(&out.stdout).contains("14.0\n"));
    }
    for command in ["", "run-hir"] {
        let out = f.cli("", command, "script.flx");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{command}: {stderr}");
        assert_eq!(out.stdout, b"4.0\n", "{command}: {stderr}");
        assert!(
            stderr.contains("script.flx:4:")
                && stderr.contains("divzero")
                && stderr.contains("propagated here"),
            "{command}: {stderr}"
        );
    }
}

#[test]
fn exact_tacit_and_explicit_forms_agree_on_a_deterministic_sample() {
    let f = Fixture::new();
    let mut state = 0x1234_5678_u32;
    let mut numbers = vec!["-9999999999999999999999999999999999999999999999.00d".to_owned()];
    for i in 0..64 {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let amount = state % 100_000;
        let sign = if i % 3 == 0 { "-" } else { "" };
        numbers.push(format!("{sign}{}.{:02}d", amount / 100, amount % 100));
    }
    let input = numbers.join(" ");
    let template = r#"<< "math";
sq(x:d):d := * x x;
double(x:d):d := + x x;
round(x:d):d := math.quantize 2n x;
op:d->d := IMPLEMENTATION;
!{ ^= ! print (@: op [INPUT]) { _ : (); }; };"#;
    for (file, expression) in [
        ("explicit.flx", r"\x -> round (- (sq x) (double x))"),
        ("tacit.flx", "&{ @. round (@* - sq double) }"),
    ] {
        f.write(
            file,
            &template
                .replace("INPUT", &input)
                .replace("IMPLEMENTATION", expression),
        );
    }
    let reference = f.cli("", "run-hir", "explicit.flx");
    assert!(reference.status.success(), "{reference:?}");
    assert!(String::from_utf8_lossy(&reference.stdout).contains(".00"));
    for command in ["", "run-hir"] {
        for _ in 0..2 {
            let out = f.cli("", command, "tacit.flx");
            assert!(out.status.success() && out.stderr.is_empty(), "{out:?}");
            assert_eq!(out.stdout, reference.stdout, "{command}");
        }
    }
}

#[test]
fn repl_and_formatter_handle_multiline_tacit_delimiters() {
    let f = Fixture::new();
    let source = "sq:z->z := &{\n    -- Keep this comment and both signs.\n    @* * . .\n};\n";
    f.write("multiline.flx", source);
    let formatted = f.cli("", "fmt", "multiline.flx");
    assert!(formatted.status.success(), "{formatted:?}");
    let formatted = String::from_utf8(formatted.stdout).unwrap();
    let token_text = |s: &str| {
        flux_lang::lexer::lex(s)
            .unwrap()
            .into_iter()
            .map(|t| t.text)
            .collect::<Vec<_>>()
    };
    assert_eq!(token_text(source), token_text(&formatted));
    assert!(formatted.contains("-- Keep this comment and both signs."));
    let mut repl = Command::new(env!("CARGO_BIN_EXE_flux"))
        .current_dir(&f.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    write!(
        repl.stdin.take().unwrap(),
        "{source}:type @ sq 6;\n@ sq 6;\n:schema @ (&{{ . }}) (#: .{{ x := [1 2]; }});\n:quit\n"
    )
    .unwrap();
    let out = repl.wait_with_output().unwrap();
    assert!(out.status.success() && out.stderr.is_empty(), "{out:?}");
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        text.contains("36 : z") && text.lines().any(|line| line == "z"),
        "{text}"
    );
    assert!(text.contains(".{ x:z }"), "{text}");
}
