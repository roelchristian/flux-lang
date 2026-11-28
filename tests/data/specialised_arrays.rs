//! Black-box parity with the independent generic evaluator. Representation
//! selection, sharing and plan admission are checked by runtime unit tests.
use super::script::Script;
use std::process::{Command, Output};

pub(super) enum Outcome<'a> {
    Ok(&'a str),
    Fault {
        stdout: &'a str,
        diagnostic: &'a [&'a str],
    },
}

pub(super) fn configuration(command: &mut Command, generic: bool, limits: &[(&str, &str)]) {
    command
        .env_remove("FLUX_OPTIMIZE")
        .env_remove("FLUX_MAX_ELEMENTS")
        .env_remove("FLUX_MAX_MEMORY")
        .env_remove("FLUX_MAX_WORK")
        .env_remove("FLUX_MAX_RANK")
        .env_remove("FLUX_MAX_EXACT_BITS")
        .env_remove("FLUX_MAX_NUMERIC_ITERATIONS")
        .env_remove("FLUX_MAX_DEPTH");
    if generic {
        command.env("FLUX_OPTIMIZE", "0");
    }
    command.envs(limits.iter().copied());
}

fn run(script: &Script, mode: Option<&str>, generic: bool, limits: &[(&str, &str)]) -> Output {
    let mut command = script.command(mode);
    configuration(&mut command, generic, limits);
    command.output().expect("invoke Flux with array policy")
}

pub(super) fn equivalent(source: &str, expected: Outcome<'_>, limits: &[(&str, &str)]) {
    let fixture = Script::new("flux-machina", "test.flx", source);
    fixture.check_hir();
    for mode in [None, Some("run-hir")] {
        let optimized = run(&fixture, mode, false, limits);
        let generic = run(&fixture, mode, true, limits);
        assert_eq!(
            (&optimized.status, &optimized.stdout, &optimized.stderr),
            (&generic.status, &generic.stdout, &generic.stderr),
            "{mode:?}: optimized and generic observable behavior differ\n{source}"
        );
        let err = String::from_utf8_lossy(&optimized.stderr);
        match expected {
            Outcome::Ok(stdout) => {
                assert!(
                    optimized.status.success() && err.is_empty(),
                    "{mode:?}: {err}"
                );
                assert_eq!(optimized.stdout, stdout.as_bytes(), "{mode:?}");
            }
            Outcome::Fault { stdout, diagnostic } => {
                assert!(!optimized.status.success(), "{mode:?}: expected a failure");
                assert_eq!(optimized.stdout, stdout.as_bytes(), "{mode:?}");
                for fragment in diagnostic {
                    assert!(
                        err.contains(fragment),
                        "{mode:?}: expected {fragment:?}: {err}"
                    );
                }
            }
        }
    }
}

#[test]
fn compact_exact_kernels_widen_without_wrapping_or_rounding() {
    equivalent(
        r#"
s := form [2n] [9223372036854775807z -9223372036854775808z];
u := form [2n] [18446744073709551615n 4294967296n];
w := form [2n] [1361129467683753853853498429727072845824z 3z];
!^{
    ! print (,. (+ s 1z), ,. (- s 1z), ,. (-: s));
    ! print (,. (+ u 1n), ,. (* u u));
    ! print (,. (* w 3z), ,. (/ w 4z));
};
"#,
        Outcome::Ok(concat!(
            "([9223372036854775808 -9223372036854775807], [9223372036854775806 -9223372036854775809], [-9223372036854775807 9223372036854775808])\n",
            "([18446744073709551616 4294967297], [340282366920938463426481119284349108225 18446744073709551616])\n",
            "([4083388403051261561560495289181218537472 9], [340282366920938463463374607431768211456 3/4])\n",
        )),
        &[],
    );
}

#[test]
fn fallback_preserves_decimal_scale_and_exact_fraction_domains() {
    equivalent(
        r#"
<< "arr";
d := form [2n 1n] [1.20d 2.0d];
x := form [1n 2n] [2.00d 0.125d];
q := form [2n 1n] [/ 1q 3q / 2q 7q];
a := + d x;
b := * d x;
c := + q x;
!^{
    ! print (dim a, ,. a, ,. b, ,. c);
};
"#,
        Outcome::Ok("([2 2], [3.20 1.325 4.00 2.125], [2.4000 0.15000 4.000 0.2500], [7/3 11/24 16/7 23/56])\n"),
        &[],
    );
}

#[test]
fn dense_exact_dot_uses_wide_integer_accumulation() {
    equivalent(
        r#"
<< "arr";
a := form [2n] [9223372036854775807z 9223372036854775807z];
b := form [2n] [2z 3z];
!^{ ! print (arr.dot a b); };
"#,
        Outcome::Ok("46116860184273879035\n"),
        &[],
    );
}

#[test]
fn native_real_and_boolean_ops_preserve_shape_and_signed_zero() {
    equivalent(
        r#"
<< "math";
left := form [2n 1n] [-0.0r 2r];
right := form [1n 2n] [0r 3r];
x := * left right;
neg:array[r]->array[r] := &-:;
negated := @ neg x;
flag := >= x 1r;
!^{
    ! print (dim x, ,. x, ,. negated, ,. flag);
    ! print (,. (~ flag), ,. (math.sin (form [2n] [-0.0r 0.0r])));
    ! print (,. (math.exp (form [2n] [0.0r -0.0r])), ,. (&& flag (form [1n] [1b])));
};
"#,
        Outcome::Ok("([2 2], [-0 -0 0 6], [0 0 -0 -6], [0b 0b 0b 1b])\n([1b 1b 1b 0b], [-0 0])\n([1 1], [0b 0b 0b 1b])\n"),
        &[],
    );
}

#[test]
fn finite_checks_and_natural_underflow_report_the_same_first_failure() {
    equivalent(
        r#"
!^{
    ! print "before";
    a := form [2n] [1.0e308r 2r];
    ! print ,. (* a 2r);
    ! print "unreachable";
};
"#,
        Outcome::Fault {
            stdout: "before\n",
            diagnostic: &["R0001", "nonfinite"],
        },
        &[],
    );
    equivalent(
        r#"
a := form [3n] [2n 0n 3n];
!^{ ! print ,. (- a 1n); };
"#,
        Outcome::Fault {
            stdout: "",
            diagnostic: &["R0001", "underflow"],
        },
        &[],
    );
}

#[test]
fn compact_conversion_to_real_is_nearest_even_at_integer_boundaries() {
    // Convert the real result back to exact q for an unambiguous assertion of
    // its binary64 value, independent of shortest-decimal formatting.
    equivalent(
        r#"
<< "arr";
signed := form [2n 3n] [
    9007199254740993z 9007199254740995z -9007199254740993z
    -9007199254740995z 9223372036854775807z -9223372036854775808z
];
natural := form [6n] [
    0n 9007199254740993n 9007199254740995n
    18446744073709550591n 18446744073709550592n 18446744073709551615n
];
convert(x:array[z]):array[r] := :r x;
!^{
    ! print ,. (:q (:r signed));
    ! print ,. (:q (@ &convert (|: signed)));
    ! print ,. (:q (:r (arr.sect [2n] [5n] natural)));
};
"#,
        Outcome::Ok(concat!(
            "[9007199254740992 9007199254740996 -9007199254740992 -9007199254740996 9223372036854775808 -9223372036854775808]\n",
            "[9007199254740992 -9007199254740996 9007199254740996 9223372036854775808 -9007199254740992 -9223372036854775808]\n",
            "[9007199254740992 9007199254740996 18446744073709549568 18446744073709551616 18446744073709551616]\n"
        )),
        &[],
    );
}
