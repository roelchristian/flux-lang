use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Source(PathBuf);

impl Source {
    fn new(text: &str) -> Self {
        loop {
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("flux-draft04-{}-{n}.flx", std::process::id()));
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut f) => {
                    f.write_all(text.as_bytes()).expect("write test source");
                    return Self(path);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("cannot create Flux source: {e}"),
            }
        }
    }

    fn invoke(&self, flags: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_flux"))
            .args(flags)
            .arg(&self.0)
            .output()
            .expect("run Flux")
    }
}

impl Drop for Source {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn run(source: &str, expected: &str) {
    let f = Source::new(source);
    let check = f.invoke(&["check"]);
    assert!(
        check.status.success(),
        "check: {}",
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(check.stdout.is_empty(), "checking must not run effects");
    let typed = f.invoke(&["hir"]);
    assert!(
        typed.status.success(),
        "typed lowering: {}",
        String::from_utf8_lossy(&typed.stderr)
    );
    assert!(String::from_utf8_lossy(&typed.stdout).contains("HirProgram"));
    let output = f.invoke(&["run-hir"]);
    assert!(
        output.status.success(),
        "run: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
}

#[test]
fn instantiates_each_use_of_inferred_generic() {
    run(
        r#"
<< "io";
id(x) := x;
!{
    ! io.print id 12;
    ! io.print id "twelve";
};
"#,
        "12\ntwelve\n",
    );
}

#[test]
fn infers_higher_order_composition() {
    run(
        r#"
<< "io";
cp(f,g,x) := @ f (@ g x);
inc(x:z):z := + x 1;
show(x:z):z := * x 2;
!{ ! io.print cp &show &inc 4; };
"#,
        "10\n",
    );
}

#[test]
fn infers_a_short_unannotated_lambda() {
    run(
        r#"
<< "io";
identity := \x -> x;
!{ ! io.print @ identity "value"; ! io.print @ identity 31; };
"#,
        "value\n31\n",
    );
}

#[test]
fn instantiates_signature_with_numeric_constraint() {
    run(
        r#"
<< "io";
sq(x:'a):'a | num 'a := * x x;
!{ ! io.print sq 2; ! io.print sq 0.5; };
"#,
        "4\n1/4\n",
    );
}

#[test]
fn infers_required_field_without_demanding_other_fields() {
    run(
        r#"
<< "io";
amount(x) := .amount x;
!{
    ! io.print amount .{ amount := 12; label := "test"; };
    ! io.print amount .{ ok := 1b; amount := 0.5; };
};
"#,
        "12\n1/2\n",
    );
}

#[test]
fn empty_aggregate_has_runtime_schema_for_csv() {
    let output =
        std::env::temp_dir().join(format!("flux-draft04-schema-{}.csv", std::process::id()));
    let _ = fs::remove_file(&output);
    // Flux expects an ASCII string. Encode any Unicode in the platform's temp path.
    let path = output
        .to_string_lossy()
        .chars()
        .map(|c| match c {
            '\\' => "\\\\".into(),
            '"' => "\\\"".into(),
            c if c.is_ascii_graphic() || c == ' ' => c.to_string(),
            c => format!("\\u{{{:x}}}", u32::from(c)),
        })
        .collect::<String>();
    let script = format!(
        r#"
<< "csv";
<< "df";
row :: {{ key:s; value:q; }};
mk(k:s,t:#row) := .{{ key := k; count := # t; mean := avg .value t; }};
summ(k:s,t:#row) := mk k t;
input:#row := #[];
res := df.agg &.key &summ input;
!{{ ! csv.write "{path}" res; }};
"#
    );
    let f = Source::new(&script);
    let check = f.invoke(&["check"]);
    assert!(
        check.status.success(),
        "check: {}",
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(!output.exists(), "check may not create CSV output");
    let result = f.invoke(&["run-hir"]);
    let csv = fs::read_to_string(&output);
    let _ = fs::remove_file(&output);
    assert!(
        result.status.success(),
        "run: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(csv.unwrap().replace("\r\n", "\n"), "key,count,mean\n");
}

#[test]
fn rejects_invalid_elaboration() {
    let mut accepted = Vec::new();
    for (label, source) in [
        ("infinite type", "omega(x) := @ x x;"),
        ("explicit signature mismatch", "bad(x:z):s := + x 1;"),
        ("function equality", "id(x:z):z := x; bad := == &id &id;"),
        (
            "missing field",
            "amount(x) := .amount x; bad := amount .{ other := 1; };",
        ),
        ("ambiguous empty", "empty := [];"),
        ("tuple spread", "id(x:z):z := x; bad := @@ &id 2;"),
        ("exact/approximate mix", "bad := + 1q 2r;"),
        ("cyclic values", "a := b; b := a;"),
        ("inconsistent recursion", "f(x) := f [x];"),
        (
            "experimental explicit open row",
            "bad(x:.{tag:z; | 'r}, y:'r):'r := y;",
        ),
        ("nested numeric lifting", "x := + [[1 2] [3 4]] 9;"),
        ("nested relational lifting", "x := = [[1 2] [3 4]] 1;"),
        ("nested logical lifting", "x := && [[1b]] 1b;"),
        ("vector range bounds", "x := .. [1 2] [3 4];"),
        ("boolean scalar ordering", "x := < 0b 1b;"),
        (
            "not executable decimal CSV",
            r#"<< "csv"; row :: {val:d;}; !{ x:#row := ! csv.read "data.csv"; };"#,
        ),
        ("undeclared nested Eq", ">> (f); f(x:['a]):b := == x x;"),
        (
            "nominal function equality",
            "id(x:z):z := x; r :: {f:z->z;}; a := r {f := &id;}; b := == a a;",
        ),
    ] {
        let output = Source::new(source).invoke(&["check"]);
        if output.status.success() || output.stderr.is_empty() {
            accepted.push(format!("{label}: {source}"));
        }
    }
    assert!(accepted.is_empty(), "invalid or silent cases: {accepted:?}");
}

#[test]
fn mutually_recursive_callables_and_forward_value_dependencies() {
    run(
        r#"
<< "io";
result := even base;
base := 4n;
even(x:n):b := ? (= x 0) 1b (odd (- x 1));
odd(x:n):b := ? (= x 0) 0b (even (- x 1));
!{ ! io.print result; };
"#,
        "1b\n",
    );
}

#[test]
fn nested_pipeline_captures_one_value_and_explicit_actions_do_not_run_early() {
    run(
        r#"
<< "io";
silent := io.print "must not execute";
twice(x:z):z := + x x;
a := |{ 4; |{ + $ 1; twice $ }; + $ 2 };
!{ ! io.print a; };
"#,
        "12\n",
    );
}

#[test]
fn generic_core_executes_through_a_typed_only_preparation() {
    use flux_lang::{parser, semantic};
    let source = r#"<< "io"; id(x) := x; !{ ! io.print id 4; ! io.print id "x"; };"#;
    let parsed = parser::parse(source).expect("parse");
    let prepared = semantic::prepare(&parsed, semantic::Mode::Script, semantic::Policy::TypedOnly)
        .expect("typed preparation without fallback");
    assert!(prepared.hir().is_some());
    assert!(prepared.migration_reason().is_none());
    assert_eq!(prepared.run().expect("HIR run").stdout, "4\nx\n");
}

#[test]
fn tuple_spread_handles_a_function_returned_by_the_first_call() {
    run(
        r#"<< "io"; f(x:z):z->z := \(y:z) -> + x y; !{ ! io.print @@ &f (1,2); };"#,
        "3\n",
    );
}

#[test]
fn exact_division_and_numeric_conversion_boundaries() {
    run(
        r#"<< "io"; !{ ! io.print / 0 -170141183460469231731687303715884105728; ! io.print / -170141183460469231731687303715884105728 -170141183460469231731687303715884105728; ! io.print :q 0.5r; ! io.print :r (/ 14949302760048185 17434334414104329); };"#,
        "0\n1\n1/2\n0.8574633481823224\n",
    );
}
