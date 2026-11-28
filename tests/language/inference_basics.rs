use crate::support::{flux, TempDir};
use std::{fs, process::Output};

struct Source(TempDir);

impl Source {
    fn new(text: &str) -> Self {
        let dir = TempDir::new("flux-inference");
        fs::write(dir.join("test.flx"), text).expect("write test source");
        Self(dir)
    }

    fn invoke(&self, flags: &[&str]) -> Output {
        flux()
            .args(flags)
            .arg(self.0.join("test.flx"))
            .output()
            .expect("run Flux")
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
    ^= ! io.print id 12 { _ : (); };
    ^= ! io.print id "twelve" { _ : (); };
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
!{ ^= ! io.print cp &show &inc 4 { _ : (); }; };
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
!{ ^= ! io.print @ identity "value" { _ : (); }; ^= ! io.print @ identity 31 { _ : (); }; };
"#,
        "value\n31\n",
    );
}

#[test]
fn instantiates_signature_with_numeric_constraint() {
    run(
        r#"
<< "io";
sq(x:~a):~a | num ~a := * x x;
!{ ^= ! io.print sq 2 { _ : (); }; ^= ! io.print sq 0.5 { _ : (); }; };
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
    ^= ! io.print amount .{ amount := 12; label := "test"; } { _ : (); };
    ^= ! io.print amount .{ ok := 1b; amount := 0.5; } { _ : (); };
};
"#,
        "12\n1/2\n",
    );
}

#[test]
fn empty_aggregate_has_runtime_schema_for_csv() {
    let dir = TempDir::new("flux-inference-schema");
    let output = dir.join("schema.csv");
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
!{{ ^{{ ^? ! csv.write "{path}" res; () }} }};
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
            "bad(x:.{tag:z; | ~r}, y:~r):~r := y;",
        ),
        ("nested numeric lifting", "x := + [[1 2] [3 4]] 9;"),
        ("nested relational lifting", "x := = [[1 2] [3 4]] 1;"),
        ("nested logical lifting", "x := && [[1b]] 1b;"),
        ("vector range bounds", "x := .. [1 2] [3 4];"),
        ("boolean scalar ordering", "x := < 0b 1b;"),
        (
            "non-scalar CSV field",
            r#"<< "csv"; row :: {val:[d];}; !{ ^{ x:#row := ^? ! csv.read "data.csv"; () } };"#,
        ),
        ("undeclared nested Eq", ">> (f); f(x:[~a]):b := == x x;"),
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
!{ ^= ! io.print result { _ : (); }; };
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
!{ ^= ! io.print a { _ : (); }; };
"#,
        "12\n",
    );
}

#[test]
fn generic_core_executes_through_a_typed_only_preparation() {
    use flux_lang::{parser, semantic};
    let source = r#"<< "io"; id(x) := x; !{ ^= ! io.print id 4 { _ : (); }; ^= ! io.print id "x" { _ : (); }; };"#;
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
        r#"<< "io"; f(x:z):z->z := \(y:z) -> + x y; !{ ^= ! io.print @@ &f (1,2) { _ : (); }; };"#,
        "3\n",
    );
}

#[test]
fn exact_division_and_numeric_conversion_boundaries() {
    run(
        r#"<< "io"; !{ ^= ! io.print / 0 -170141183460469231731687303715884105728 { _ : (); }; ^= ! io.print / -170141183460469231731687303715884105728 -170141183460469231731687303715884105728 { _ : (); }; ^= ! io.print :q 0.5r { _ : (); }; ^= ! io.print :r (/ 14949302760048185 17434334414104329) { _ : (); }; };"#,
        "0\n1\n1/2\n0.8574633481823224\n",
    );
}
