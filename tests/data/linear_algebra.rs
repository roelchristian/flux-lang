use super::specialised_arrays::{equivalent, Outcome};

#[test]
fn exact_zero_volume_rank_does_not_scan_the_empty_axis() {
    equivalent(
        r#"
<< "lin";
empty:[z] := [];
a := form [1000000000n 0n] empty;
b := form [0n 1000000000n] empty;
!^{
    ! print (lin.rank a, lin.rank b);
};
"#,
        Outcome::Ok("(ok 0, ok 0)\n"),
        &[
            ("FLUX_MAX_ELEMENTS", "1000000000"),
            ("FLUX_MAX_WORK", "100"),
        ],
    );
}

#[test]
fn exact_linear_algebra_checks_both_rhs_forms_and_decimal_domain() {
    equivalent(
        r#"
<< "lin";
a := form [2n 2n] [2.00d 1.0d 1.0d 3.00d];
b := form [2n] [5.00d 5.00d];
rhs := form [2n 2n] [5.00d 0.00d 5.00d 5.00d];
noneq:?array[q] := ?0;
noned:?array[d] := ?0;
!^{
    ! print (lin.trace a, lin.det a, lin.rank a);
    ! print ,. (^= lin.solve a b { _ : ?! noneq; });
    ! print ,. (^= lin.solve a rhs { _ : ?! noneq; });
    ! print ,. (^= lin.pow a 1n { _ : ?! noned; });
};
"#,
        Outcome::Ok("(ok 5.00, ok 5, ok 2)\n[2 1]\n[2 -1 1 2]\n[2.00 1.0 1.0 3.00]\n"),
        &[],
    );
}

#[test]
fn solver_domain_failures_are_typed_but_caps_are_fatal() {
    equivalent(
        r#"
<< "lin";
a := form [2n 2n] [1z 2z 1z 2z];
b := form [1n] [1z];
!^{
    ! print (lin.inv a, lin.solve a b);
};
"#,
        Outcome::Ok("(err singular, err badshape)\n"),
        &[],
    );
    equivalent(
        r#"
<< "lin";
a := form [2n 2n] [1152921504606846976z 0z 0z 1152921504606846976z];
!^{
    ! print "started";
    ! print lin.det a;
};
"#,
        Outcome::Fault {
            stdout: "started\n",
            diagnostic: &["R0001", "bit limit", "test.flx:6:"],
        },
        &[("FLUX_MAX_EXACT_BITS", "105")],
    );
}

#[test]
fn complex_components_stay_finite_before_the_final_rounding() {
    equivalent(
        r#"
<< "cx";
a := cx.make 1.4e308r 3.5e307r;
b := cx.make 1.3r 0.325r;
c := cx.make 1e-300r 1e-300r;
p := * a b;
q := / a a;
r := / c c;
z := cx.conj (cx.make 1r 0r);
!^{
    ! print (> cx.re p 1.7e308r, > cx.im p 9e307r, q, r, cx.im z);
};
"#,
        Outcome::Ok("(1b, 1b, cx(1, 0), cx(1, 0), -0)\n"),
        &[],
    );
}
