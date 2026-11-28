use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new(source: &str) -> Self {
        loop {
            let path = std::env::temp_dir().join(format!(
                "flux-draft09-inference-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => {
                    fs::write(path.join("test.flx"), source).expect("write Flux fixture");
                    return Self(path);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("create fixture directory: {e}"),
            }
        }
    }

    fn cli(&self, mode: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_flux"))
            .current_dir(&self.0)
            .args([mode, "test.flx"])
            .output()
            .expect("run the Flux CLI")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn works(source: &str, expected: &str) {
    let fixture = Fixture::new(source);
    let checked = fixture.cli("check");
    assert!(
        checked.status.success() && checked.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let outcome = fixture.cli("run-hir");
    assert!(
        outcome.status.success() && outcome.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&outcome.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&outcome.stdout), expected);
}

fn rejected(source: &str, fragment: &str) -> String {
    let result = Fixture::new(source).cli("check");
    let stderr = String::from_utf8_lossy(&result.stderr).into_owned();
    assert!(
        !result.status.success() && result.stdout.is_empty(),
        "{stderr}\n{source}"
    );
    assert!(
        stderr.contains("E0200") && stderr.contains(fragment),
        "{stderr}\n{source}"
    );
    stderr
}

#[test]
fn generalizes_nonexpansive_closures_and_tacit_values_independently() {
    works(
        r#"
identity:~a->~a := \x -> x;
square := &{ @* * . . };
alias := square;
!{ ^{
    ^? ! print (@ identity 1z,@ identity "s",@ alias 3n,@ alias 0.5q,@ alias 1.20d);
    ()
} };
"#,
        "(1, s, 9, 1/4, 1.4400)\n",
    );
}

#[test]
fn expansive_bindings_keep_weak_types_and_cannot_claim_universal_signatures() {
    rejected(
        r#"
id(x) := x;
weak := @ &id &id;
i := @ weak 1z;
s := @ weak "s";
"#,
        "cannot unify",
    );
    let error = rejected(
        r#"
id(x) := x;
claimed:~a->~a := @ &id &id;
only_one := @ claimed 1z;
"#,
        "value restriction",
    );
    assert!(error.contains("test.flx:3:"), "{error}");
    works(
        r#"
id(x) := x;
specialized:z->z := @ &id &id;
!{ ^{ ^? ! print @ specialized 7z; () } };
"#,
        "7\n",
    );
}

#[test]
fn declared_schemes_check_rigidity_and_captured_variables() {
    rejected("specialized(x:~a):~a := + x 1z;", "annotated type variable");
    rejected(
        r#"
outer(f) := {
    pretend(x:~a):~a := @ f x;
    pretend "s"
};
"#,
        "escapes",
    );
    rejected("missing(x:~a):~a := * x x;", "lacks");
}

#[test]
fn recursive_headers_are_monomorphic_in_the_group_and_generalize_afterwards() {
    works(
        r#"
walk(k:n,x:~a):~a := ?{
    = k 0n : x;
    _ : walk (- k 1n) x;
};
!{ ^{ ^? ! print (walk 3n 7z,walk 2n "still s"); () } };
"#,
        "(7, still s)\n",
    );
    rejected("infinite(x) := infinite [x];", "occurs check");
}

#[test]
fn named_higher_order_and_tacit_numeric_composition_share_domains() {
    works(
        r#"
square(x:~a):~a | num ~a := * x x;
apply(f,x) := @ f x;
tacit := &{ @* * . . };
!{ ^{
    ^? ! print (square 1.20d,apply &square 1.20d,@ tacit 1.20d);
    ()
} };
"#,
        "(1.4400, 1.4400, 1.4400)\n",
    );
}

#[test]
fn an_expansive_export_cannot_be_specialized_by_its_consumer() {
    let fixture = Fixture::new(
        r#"<< "lib.flx" : lib;
value := @ lib.claimed 7z;
!{ () };
"#,
    );
    fs::write(
        fixture.0.join("lib.flx"),
        r#">> (claimed);
id(x) := x;
claimed:~a->~a := @ &id &id;
"#,
    )
    .unwrap();
    let out = fixture.cli("check");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success()
            && stderr.contains("lib.flx:3:")
            && stderr.contains("value restriction"),
        "{stderr}"
    );
}

#[test]
fn type_conflicts_show_the_expected_and_inferred_types_at_the_operand() {
    let stderr = rejected("ok := 0z;\nwrong:s := 7z;\n", "cannot unify");
    for detail in ["test.flx:2:12:", "expected: s", "actual: z"] {
        assert!(stderr.contains(detail), "{detail}: {stderr}");
    }
}
