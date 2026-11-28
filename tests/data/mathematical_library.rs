use super::script::{self, Script};

pub(super) fn specimen(source: &str) -> Script {
    Script::new("flux-mathesis", "test.flx", source)
}

pub(super) fn outputs(imports: &str, defs: &str, operands: &[&str]) -> Script {
    let mut source = format!("{imports}\n{defs}\n!^{{\n");
    for op in operands {
        source.push_str(&format!("! print ({op});\n"));
    }
    source.push_str("};\n");
    specimen(&source)
}

pub(super) fn close(actual: &str, expected: f64) {
    let got = actual
        .parse::<f64>()
        .unwrap_or_else(|_| panic!("{actual:?} is not a real"));
    let tolerance = 2e-11 * expected.abs().max(1.0);
    assert!(
        got.is_finite() && (got - expected).abs() <= tolerance,
        "expected {expected:.17e}, got {got:.17e}"
    );
}

fn runtime_failure(op: &str) {
    let script = outputs("<< \"math\";", "", &[op]);
    script.check_hir();
    for mode in [None, Some("run-hir")] {
        let output = script.run(mode);
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success() && output.stdout.is_empty() && diagnostic.contains("R0001"),
            "{mode:?}: {op}: {diagnostic}"
        );
    }
}

#[test]
fn exact_rounding_and_fraction_do_not_round_through_binary64() {
    outputs(
        r#"<< "math";"#,
        r#"
wide := ^ 2z 180n;
x := + wide (/ 17q 20q);
"#,
        &[
            "(math.floor x,math.ceil x,math.trunc (-: x),math.round x)",
            "(math.floor -1.20d,math.ceil -1.20d,math.trunc -1.20d,math.frac -1.20d)",
            "(math.round -2.5q,math.round -3.5q,math.round 2.5d,math.round 3.5r)",
            "(math.sign -2q,math.sign 0n,math.sign -0.0r,math.sign 1.20d)",
            "(math.frac 4n,math.frac -4z,math.frac (/ -17q 4q),math.frac -1.25r)",
            "(math.recip 8n,math.recip -4z,math.recip 0.125d,math.recip 0.25r)",
            "(math.floor 340282366920938463463374607431768211456r,math.trunc -1.75r)",
        ],
    )
    .accepts(concat!(
        "(1532495540865888858358347027150309183618739122183602176, 1532495540865888858358347027150309183618739122183602177, -1532495540865888858358347027150309183618739122183602176, 1532495540865888858358347027150309183618739122183602177)\n",
        "(-2, -1, -1, -0.20)\n(-2, -4, 2, 4)\n(-1, 0, 0, 1)\n",
        "(0, 0, -1/4, -0.25)\n(1/8, -1/4, 8, 4)\n",
        "(340282366920938463463374607431768211456, -1)\n"
    ));
}

#[test]
fn each_real_function_has_a_checked_finite_numerical_contract() {
    let cases = [
        ("math.pi", std::f64::consts::PI),
        ("math.e", std::f64::consts::E),
        ("math.tau", std::f64::consts::TAU),
        ("math.exp 1r", std::f64::consts::E),
        ("math.expm1 1r", std::f64::consts::E - 1.0),
        ("math.ln 2r", std::f64::consts::LN_2),
        ("math.log10 1000r", 3.0),
        ("math.log1p 1r", std::f64::consts::LN_2),
        ("math.log 2r 32r", 5.0),
        ("math.pow -2r 3r", -8.0),
        ("math.root 3n -8r", -2.0),
        ("math.root 2n 4r", 2.0),
        ("math.hypot 3r 4r", 5.0),
        ("math.sin (/ math.pi 2r)", 1.0),
        ("math.cos math.pi", -1.0),
        ("math.tan (/ math.pi 4r)", 1.0),
        ("math.asin 1r", std::f64::consts::FRAC_PI_2),
        ("math.acos -1r", std::f64::consts::PI),
        ("math.atan 1r", std::f64::consts::FRAC_PI_4),
        ("math.atan2 1r -1r", 3.0 * std::f64::consts::FRAC_PI_4),
        ("math.sinh 1r", 1.1752011936438014),
        ("math.cosh 1r", 1.5430806348152437),
        ("math.tanh 1r", 0.7615941559557649),
        ("math.asinh 1r", 0.881373587019543),
        ("math.acosh 2r", 1.3169578969248166),
        ("math.atanh 0.5r", 0.5493061443340548),
        ("math.rad 180r", std::f64::consts::PI),
        ("math.deg math.pi", 180.0),
        ("math.gamma 5r", 24.0),
        ("math.gamma -0.5r", -2.0 * std::f64::consts::PI.sqrt()),
        ("math.lgamma 5r", 24.0_f64.ln()),
        ("math.erf 1r", 0.8427007929497149),
        ("math.erfc 1r", 0.15729920705028513),
        ("math.beta 2r 3r", 1.0 / 12.0),
        // Unsuffixed literals use the checked approximate context.
        ("math.log 2 8", 3.0),
    ];
    let operations = cases.iter().map(|&(op, _)| op).collect::<Vec<_>>();
    let script = outputs("<< \"math\";", "", &operations);
    script.check_hir();
    for mode in [Some("run-hir"), None] {
        let result = script.run(mode);
        assert!(
            result.status.success() && result.stderr.is_empty(),
            "{mode:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let stdout = String::from_utf8(result.stdout).unwrap();
        assert_eq!(stdout.lines().count(), cases.len());
        for ((op, want), got) in cases.into_iter().zip(stdout.lines()) {
            assert!(!got.is_empty(), "{op} output missing");
            close(got, want);
        }
    }
}

#[test]
fn real_domain_finiteness_and_exact_separation_are_enforced() {
    for op in [
        "math.exp 800r",
        "math.ln 0r",
        "math.log -2r 3r",
        "math.log 1r 3r",
        "math.log1p -1r",
        "math.asin 2r",
        "math.acos -2r",
        "math.acosh 0.5r",
        "math.atanh 1r",
        "math.pow -2r 0.5r",
        "math.root 0n 2r",
        "math.root 2n -2r",
        "math.gamma 0r",
        "math.lgamma -1r",
        "math.beta -1r 2r",
        "math.recip 0z",
        "math.recip -0.0r",
    ] {
        runtime_failure(op);
    }
    for line in [
        "a:=math.sin 1q;",
        "a:=math.atan2 1r 1d;",
        "a:=math.log 2r 3z;",
        "a:=math.root 2z 9r;",
    ] {
        let source = format!("<< \"math\"; {line}");
        script::rejects(&specimen(&source), &source, &["E0200"]);
    }
}

#[test]
fn qualified_native_functions_compose_and_lift_through_checked_instances() {
    outputs(
        r#"<< "math";"#,
        r#"
rounded:array[z]->array[z] := &math.round;
absgcd:array[z]->z->array[n] := &math.gcd;
floorseq:[q]->[z] := &math.floor;
reciprocal:array[q]->array[q] := &math.recip;
rounds/1:array[z]->array[z] := &{ @. math.round . };
tac/1:[q]->[z] := &{ @. math.floor . };
a := form [2n 1n] [4z 6z];
b := form [1n 3n] [2z 3z 4z];
x := form [2n] [/ 1q 2q / 1q 4q];
"#,
        &[
            "(dim (math.gcd a b), ,. (math.gcd a b), ,. (@@ absgcd (a,8z)))",
            "(,. (@ rounded a), ,. (rounds a), @ floorseq [1.20q -1.20q], tac [1.20q -1.20q])",
            "(,. (@ reciprocal x),math.sign [0.0d -1.20d 2.00d])",
            "(math.hypot [0r 0r] [4r 9r], math.gcd [-12z 18z] 8z)",
        ],
    )
    .accepts(concat!(
        "([2 3], [2 1 4 2 3 2], [4 2])\n",
        "([4 6], [4 6], [1 -2], [1 -2])\n([2 4], [0 -1 1])\n([4 9], [4 2])\n"
    ));
}

#[test]
fn integer_theory_preserves_wide_results_and_nonnegative_contracts() {
    outputs(
        r#"<< "math";"#,
        r#"
wide := ^ 2n 180n;
modular := &math.modpow;
"#,
        &[
            "(math.gcd -54z 24z,math.lcm -54z 24z,math.gcd 0z 0z,math.lcm 0z 0z)",
            "(math.gcd (-: (:z wide)) (:z (* wide 3n)),math.lcm 12z -18z)",
            "(math.fact 0n,math.fact 25n,math.choose 100n 50n,math.perm 30n 20n)",
            "(math.choose 3n 6n,math.perm 3n 6n,math.fib 0n,math.fib 100n)",
            "(math.modpow 7n 128n 19n,@@ modular (7n,128n,19n),math.modpow 0n 0n 1n)",
            "(math.isqrt 0n,math.isqrt 80n,math.isqrt (- (^ 2n 256n) 1n))",
        ],
    )
    .accepts(concat!(
        "(6, 216, 0, 0)\n(1532495540865888858358347027150309183618739122183602176, 36)\n",
        "(1, 15511210043330985984000000, 100891344545564193334812497256, 73096577329197271449600000)\n",
        "(0, 0, 0, 354224848179261915075)\n(11, 11, 0)\n",
        "(0, 8, 340282366920938463463374607431768211455)\n"
    ));
}

#[test]
fn primality_and_factorization_are_exact_or_refuse_work() {
    outputs(
        r#"<< "math";"#, "",
        &[
            "math.prime [0n 1n 2n 3n 4n 97n 341550071728321n 3825123056546413051n 2305843009213693951n]",
            "(math.factor 1n,math.factor 360n,math.factor 43867n)",
            "(math.divisors 1n,math.divisors 36n)",
        ],
    ).accepts(concat!(
        "[0b 0b 1b 1b 0b 1b 0b 0b 1b]\n",
        "([], [2 2 2 3 3 5], [43867])\n([1], [1 2 3 4 6 9 12 18 36])\n"
    ));
    for op in [
        "math.factor 0n",
        "math.divisors 0n",
        "math.modpow 1n 2n 0n",
        "math.fact 999999999999999999999n",
        "math.fib 999999999999999999999n",
    ] {
        runtime_failure(op);
    }
    // A well-known composite strong pseudoprime beyond u64. Accept a proof
    // of compositeness or a checked resource diagnostic, never "prime".
    let script = outputs(
        "<< \"math\";",
        "",
        &["math.prime 318665857834031151167461n"],
    );
    script.check_hir();
    for mode in [Some("run-hir"), None] {
        let r = script.run(mode);
        if r.status.success() {
            assert_eq!(r.stdout, b"0b\n");
            assert!(r.stderr.is_empty());
        } else {
            assert!(r.stdout.is_empty() && String::from_utf8_lossy(&r.stderr).contains("R0001"));
        }
    }
}

#[test]
fn polynomial_arithmetic_is_exact_and_normalizes_produced_coefficients() {
    outputs(
        r#"<< "poly";"#,
        r#"
z:[z] := [];
t:[q] := [];
eval:[z]->z->z := &poly.eval;
derive/1:[q]->[q] := &{ @. poly.deriv . };
"#,
        &[
            "(poly.eval [1z 2z 3z] 2z,@@ eval ([1z 2z 3z],2z),poly.eval z 9z)",
            "(poly.add [1z 0z 3z] [5z 0z -3z],poly.sub [2z 1z] [1z 1z],poly.mul [1z 2z] [3z 4z])",
            "(poly.deriv [1z 2z 3z 0z],derive [1q (/ 2q 3q) 2q],poly.deriv [7z])",
            "(poly.integ [2d 3.5d],poly.integ [1r 3.5r],poly.integ t)",
            "(poly.degree [0z 0z],poly.degree [3z 4z 0z],poly.degree z,poly.mul z [1z])",
            "(poly.eval [1.20d 2.00d] 3d,poly.mul [1.20d 2.0d] [3d])",
        ],
    )
    .accepts(concat!(
        "(17, 17, 0)\n([6], [1], [3 10 8])\n([2 6], [2/3 4], [])\n",
        "([0 2 7/4], [0 1 1.75], [])\n(?0, ?1 1, ?0, [])\n(7.20, [3.60 6.0])\n"
    ));
}
