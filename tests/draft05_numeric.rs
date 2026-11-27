use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
const WIDE: &str = "340282366920938463463374607431768211457";
const INLINE_MAX: &str = "170141183460469231731687303715884105727";

struct Script(PathBuf);

impl Script {
    fn new(source: &str) -> Self {
        loop {
            let root = std::env::temp_dir().join(format!(
                "flux-draft05-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&root) {
                Ok(()) => {
                    fs::write(root.join("test.flx"), source).expect("write fixture");
                    return Self(root);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("create fixture directory: {e}"),
            }
        }
    }

    fn cli(&self, mode: Option<&str>) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_flux"));
        command.current_dir(&self.0);
        if let Some(mode) = mode {
            command.arg(mode);
        }
        command.arg("test.flx").output().expect("public Flux CLI")
    }
}

impl Drop for Script {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn printing(expressions: &[&str]) -> String {
    let mut source = String::from("<< \"io\"; << \"math\";\n!{ ^{\n");
    for e in expressions {
        source.push_str(&format!("    ^? ! io.print ({e});\n"));
    }
    source.push_str("    ()\n} };\n");
    source
}

fn strict(f: &Script, expected: &str) {
    let check = f.cli(Some("check"));
    assert!(
        check.status.success() && check.stdout.is_empty(),
        "check: {}",
        String::from_utf8_lossy(&check.stderr)
    );
    for mode in [Some("run-hir"), None] {
        let out = f.cli(mode);
        assert!(
            out.status.success(),
            "{mode:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&out.stdout), expected, "{mode:?}");
    }
}

#[test]
fn wide_integers_and_rationals_cross_inline_limits_exactly() {
    let source = printing(&[
        &format!("+ {INLINE_MAX}z 1z"),
        &format!("+ {WIDE}n 1n"),
        &format!("- {WIDE}n {WIDE}n"),
        &format!("/ (* {WIDE}z 2z) (* {WIDE}z 3z)"),
        &format!("/ 1z {WIDE}z"),
        &format!("^= math.div 1z {WIDE}z {{ _ : 0q; }}"),
        &format!("+ [1z {WIDE}z] 1z"),
    ]);
    strict(
        &Script::new(&source),
        &format!(
            "170141183460469231731687303715884105728\n340282366920938463463374607431768211458\n0\n2/3\n1/{WIDE}\n1/{WIDE}\n[2 340282366920938463463374607431768211458]\n"
        ),
    );
}

#[test]
fn huge_signed_quotients_floor_and_remainders_follow_the_divisor() {
    let source = printing(&[
        &format!("(// -{WIDE}z 7z,% -{WIDE}z 7z)"),
        &format!("(// {WIDE}z -7z,% {WIDE}z -7z)"),
        &format!("(// -{WIDE}z -7z,% -{WIDE}z -7z)"),
    ]);
    strict(
        &Script::new(&source),
        "(-48611766702991209066196372490252601637, 2)\n(-48611766702991209066196372490252601637, -2)\n(48611766702991209066196372490252601636, -5)\n",
    );
}

#[test]
fn decimal_arithmetic_preserves_scale_but_comparison_ignores_it() {
    let source = printing(&[
        "1.20d",
        "+ 1.20d 2.3d",
        "- 1.20d 1.2d",
        "* 1.20d 2.0d",
        "^ 1.20d 2n",
        "^ 1.20d 0n",
        "+ 1.20d 3z",
        "-: 0.00d",
        "| -1.20d",
        "/ 1.20d 3.0d",
        "+ 1.20d (/ 1z 3z)",
        "= 1.20d 1.2d",
        "== 1.20d 1.2d",
        ".: 1.20d (=: (\\x -> x) [1.2d 1.20d])",
        "=. [1.20d 1.2d 2.30d]",
    ]);
    strict(
        &Script::new(&source),
        "1.20\n3.50\n0.00\n2.400\n1.4400\n1\n4.20\n0.00\n1.20\n2/5\n23/15\n1b\n1b\n?1 [1.2 1.20]\n[1.20 2.30]\n",
    );
}

#[test]
fn quantize_is_exact_half_even_and_preserves_requested_scale() {
    let source = printing(&[
        "math.quantize 2n 2.345d",
        "math.quantize 2n 2.355d",
        "math.quantize 2n -2.345d",
        "math.quantize 2n -2.355d",
        "math.quantize 2n (/ 1z 3z)",
        "math.quantize 4n (/ 5z 2z)",
        "math.quantize 0n (/ -5z 2z)",
        "math.quantize 0n (/ -7z 2z)",
        ":d (/ 7z 40z)",
        ":d 0.125r",
        ":d 0.1200d",
    ]);
    strict(
        &Script::new(&source),
        "2.34\n2.36\n-2.34\n-2.36\n0.33\n2.5000\n-2\n-4\n0.175\n0.125\n0.1200\n",
    );
}

#[test]
fn binary64_conversions_cover_subnormal_and_ties_to_even() {
    let source = printing(&[
        "= (:q 5e-324r) (/ 1z (^ 2z 1074n))",
        "= (:r (/ 1z (^ 2z 1074n))) 5e-324r",
        "= (:r (/ 1z (^ 2z 1075n))) 0r",
        "= (:r (+ 1q (/ 1z (^ 2z 53n)))) 1r",
        "= (:r (+ 1q (/ 3z (^ 2z 53n)))) 1.0000000000000004r",
        "= (:r (:q 1.7976931348623157e308r)) 1.7976931348623157e308r",
    ]);
    strict(&Script::new(&source), "1b\n1b\n1b\n1b\n1b\n1b\n");
}

#[test]
fn unchecked_failures_are_diagnostics_but_checked_division_returns_a_result() {
    let source = printing(&["^= math.div 2q 0q { divzero : 7q; range : 8q; }"]);
    strict(&Script::new(&source), "7\n");
    for (expression, expected) in [
        ("/ 1.20d 0.00d", "division by zero"),
        ("- 0n 1n", "natural subtraction underflow"),
        (":d (/ 1z 3z)", "decimal"),
        (":r (^ 2z 1024n)", "nonfinite"),
    ] {
        let f = Script::new(&printing(&[expression]));
        assert!(f.cli(Some("check")).status.success(), "{expression}");
        let out = f.cli(Some("run-hir"));
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !out.status.success() && out.stdout.is_empty() && stderr.contains(expected),
            "{expression}: {stderr}"
        );
    }
}

#[test]
fn csv_preserves_decimal_scale_and_wide_rational_cells() {
    let source = r#"
<< "csv";
<< "io";
err :: { | csvfail csverr; | iofail ioerr; };
row :: { price:d; amount:z; rate:q; };
!{
    ^{
        table:#row := ^= ! csv.read "in.csv" { e : ^: csvfail e; };
        ^= ! io.print (.price table,.amount table,.rate table) { e : ^: iofail e; };
        ^= ! csv.write "out.csv" table { e : ^: csvfail e; };
        ()
    }
};
"#;
    let f = Script::new(source);
    let csv = format!("price,amount,rate\n123.4500,{WIDE},1/{WIDE}\n-0.50,-{WIDE},2/3\n");
    fs::write(f.0.join("in.csv"), &csv).unwrap();
    let check = f.cli(Some("check"));
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(!f.0.join("out.csv").exists());
    let run = f.cli(Some("run-hir"));
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        format!("([123.4500 -0.50], [{WIDE} -{WIDE}], [1/{WIDE} 2/3])\n")
    );
    let rewritten = fs::read_to_string(f.0.join("out.csv")).unwrap();
    assert_eq!(rewritten.replace("\r\n", "\n"), csv);
}

#[test]
fn repl_reports_domains_and_preserves_display_scale() {
    let mut repl = Command::new(env!("CARGO_BIN_EXE_flux"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start REPL");
    let source = format!("+ {WIDE}z 1z\n+ 1.20d 2.3d\n/ 1.20d 3.0d\n:q\n");
    repl.stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    let out = repl.wait_with_output().unwrap();
    assert!(
        out.status.success() && out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "340282366920938463463374607431768211458 : z\n3.50 : d\n2/5 : q\n"
    );
}

#[test]
fn wide_values_keep_collection_order_and_reduction_types() {
    let source = printing(&[
        &format!(".. {WIDE}z (+ {WIDE}z 2z)"),
        &format!("take {WIDE}n [1z 2z]"),
        &format!("drop {WIDE}n [1z 2z]"),
        &format!(".: {WIDE}n [1z 2z]"),
        "+/ [1.20d 2.3d]",
        "*/ [1.20d 2.0d]",
        &format!("^ -1z {WIDE}n"),
    ]);
    strict(
        &Script::new(&source),
        &format!(
            "[{WIDE} 340282366920938463463374607431768211458 340282366920938463463374607431768211459]\n[1 2]\n[]\n?0\n3.50\n2.400\n-1\n"
        ),
    );
}

#[test]
fn numeric_resource_guards_are_not_machine_overflow_or_rounding() {
    let source = printing(&[
        "= (^ 2z 1048575n) (* 2z (^ 2z 1048574n))",
        "math.quantize 10000n 0q",
        "^= math.div (^ 2z 600000n) (/ 1z (^ 2z 600000n)) { divzero : 7q; range : 9q; }",
    ]);
    strict(
        &Script::new(&source),
        &format!("1b\n0.{}\n9\n", "0".repeat(10_000)),
    );
    for expression in ["^ 2z 1048576n", "math.quantize 10001n 0q"] {
        let f = Script::new(&printing(&[expression]));
        assert!(f.cli(Some("check")).status.success(), "{expression}");
        let output = f.cli(Some("run-hir"));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success() && output.stdout.is_empty() && stderr.contains("resource"),
            "{expression}: {stderr}"
        );
    }
}

#[test]
fn decimal_does_not_hide_approximation_or_inexact_scale_types() {
    for expression in [
        "+ 1.20d 0.5r",
        "math.quantize 2n 1.25r",
        "math.quantize 2z 1.20d",
        "// 1.20d 1z",
    ] {
        let f = Script::new(&printing(&[expression]));
        let result = f.cli(Some("check"));
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(
            !result.status.success() && stderr.contains("E0200"),
            "{expression}: {stderr}"
        );
    }
}
