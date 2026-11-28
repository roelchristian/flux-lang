//! Observable semantics at the boundaries targeted by the next dense kernels.
use super::specialised_arrays::{equivalent, Outcome};

#[test]
fn range_form_preserves_domains_and_distinguishes_empty_from_rank_zero() {
    equivalent(
        r#"
mixed := form [2n 3n] (.. -3z 2z);
signed := form [3n] (.. 9223372036854775806z 9223372036854775808z);
natural := form [3n] (.. 18446744073709551614n 18446744073709551616n);
cell := form [] (.. -1z -1z);
empty := form [2n 0n 3n] (.. 1z 0z);
!^{
    ! print (dim mixed, ,. mixed);
    ! print (,. signed, ,. natural);
    ! print (dim cell, # cell, .: [] cell, dim empty, # empty, .: [1n 1n 1n] empty);
};
"#,
        Outcome::Ok(concat!(
            "([2 3], [-3 -2 -1 0 1 2])\n",
            "([9223372036854775806 9223372036854775807 9223372036854775808], ",
            "[18446744073709551614 18446744073709551615 18446744073709551616])\n",
            "([], 1, ?1 -1, [2 0 3], 0, ?0)\n",
        )),
        &[],
    );
}

#[test]
fn range_form_obeys_operand_order_and_checks_range_before_shape() {
    equivalent(
        r#"
dims(x:n):[n] := ?{ = x 0n : [x]; _ : ?! (?0); };
!^{
    ! print "entered";
    ! print # (form (dims 1n) (.. 0z (// 1z 0z)));
};
"#,
        Outcome::Fault {
            stdout: "entered\n",
            diagnostic: &["R0001", "cannot unwrap ?0", "test.flx:2:"],
        },
        &[],
    );
    let source = r#"
!^{
    ! print "entered";
    ! print # (form [3n 3n] (.. 1z 20z));
};
"#;
    for (limits, message) in [
        (&[][..], "array shape does not match the number of elements"),
        (
            &[("FLUX_MAX_ELEMENTS", "10")][..],
            "prototype range collection limit exceeded",
        ),
    ] {
        equivalent(
            source,
            Outcome::Fault {
                stdout: "entered\n",
                diagnostic: &["R0001", message, "test.flx:4:"],
            },
            limits,
        );
    }
}

#[test]
fn broadcast_uses_both_physical_offsets_and_right_aligned_singletons() {
    equivalent(
        r#"
<< "arr";
a := arr.axes [2n 4n 1n 3n] (form [3n 2n 4n 2n] (.. 1z 48z));
b := arr.axes [2n 1n 3n] (form [3n 3n 2n] (.. 101z 118z));
lhs := arr.sect [1n 2n 2n 2n] [2n 1n 2n 2n] a;
rhs := arr.sect [2n 2n 2n] [2n 1n 1n] b;
shape0 := form [] [7z];
zero := arr.sect [1n 2n 2n 1n] [2n 0n 2n 2n] a;
!^{
    added := + lhs rhs;
    scaled := * shape0 lhs;
    vacant := + zero shape0;
    ! print (dim added, ,. added);
    ! print (dim scaled, ,. scaled, dim vacant, # vacant);
};
"#,
        Outcome::Ok(concat!(
            "([2 2 2 2], [130 132 146 148 132 134 148 150 138 140 154 156 140 142 156 158])\n",
            "([2 1 2 2], [140 154 252 266 196 210 308 322], [2 0 2 2], 0)\n",
        )),
        &[],
    );
}

#[test]
fn empty_broadcast_cannot_hide_an_incompatible_trailing_extent() {
    equivalent(
        r#"
empty:[z] := [];
a := form [2n 0n 2n] empty;
b := form [3n 2n] (.. 1z 6z);
!^{
    ! print "entered";
    ! print # (+ a b);
};
"#,
        Outcome::Fault {
            stdout: "entered\n",
            diagnostic: &["R0001", "incompatible trailing axes", "test.flx:7:"],
        },
        &[],
    );
}

#[test]
fn reduction_keeps_negative_zero_singletons_and_zero_width_lanes() {
    equivalent(
        r#"
<< "arr";
z := |: (form [3n 2n] [-0.0r 0.0r -0.0r -0.0r -0.0r 0.0r]);
one := arr.sect [1n 1n] [2n 1n] z;
empty:[r] := [];
zero := form [2n 0n] empty;
!^{
    ! print (,. (arr.red 2n &+ z), ,. (arr.fold 2n &+ 0r z), ,. (arr.red 2n &+ one));
    ! print (,. (>/ z), ,. (</ z));
    no_cells := arr.red 1n &+ zero;
    ! print (,. (arr.red 2n &+ zero), ,. (arr.fold 2n &+ 0r zero), dim no_cells, ,. no_cells);
};
"#,
        Outcome::Ok(concat!(
            "([?1 -0 ?1 0], [0 0], [?1 -0 ?1 0])\n",
            "([?1 -0 ?1 0], [?1 -0 ?1 0])\n",
            "([?0 ?0], [0 0], [0], [])\n",
        )),
        &[],
    );
}

#[test]
fn unseeded_reduction_checks_wide_intermediate_even_if_final_value_fits() {
    let source = r#"
<< "arr";
x := form [1n 3n] [9223372036854775807z 9223372036854775807z -9223372036854775807z];
!^{
    ! print "entered";
    ! print ,. (arr.red 2n &+ x);
};
"#;
    equivalent(
        source,
        Outcome::Ok("entered\n[?1 9223372036854775807]\n"),
        &[],
    );
    equivalent(
        source,
        Outcome::Fault {
            stdout: "entered\n",
            diagnostic: &["R0001", "bit", "test.flx:6:"],
        },
        &[("FLUX_MAX_EXACT_BITS", "63")],
    );
}

#[test]
fn unseeded_callbacks_visit_one_logical_lane_before_the_next() {
    equivalent(
        r#"
<< "arr";
step(a,b:z):z := ?{
    = b 5z : // a 0z;
    = b 4z : ?! (?0);
    _ : - a b;
};
t := |: (form [3n 2n] [1z 2z 3z 4z 5z 6z]);
!^{
    ! print "entered";
    ! print ,. (arr.red 2n &step t);
    ! print "unreached";
};
"#,
        Outcome::Fault {
            stdout: "entered\n",
            // 5 ends the first logical lane, even though 4 occurs earlier
            // in backing storage and raises a distinct fault in the next lane.
            diagnostic: &["R0001", "division by zero", "test.flx:4:"],
        },
        &[],
    );
}

#[test]
fn unseeded_reduction_preserves_numeric_fault_precedence_at_the_work_boundary() {
    let source = r#"
<< "arr";
t := |: (form [3n 2n] [1r 1r 2r 1r 1e308r 1e308r]);
!^{
    ! print "entered";
    ! print ,. (arr.red 2n &* t);
    ! print "unreached";
};
"#;
    // 40 ticks reach the first lane's overflowing multiply, but not the
    // second lane. Charging the whole array before stepping would mask it.
    for (work, fault) in [
        ("39", "prototype execution limit exhausted"),
        ("40", "nonfinite real result"),
    ] {
        equivalent(
            source,
            Outcome::Fault {
                stdout: "entered\n",
                diagnostic: &["R0001", fault, "test.flx:6:"],
            },
            &[("FLUX_MAX_WORK", work)],
        );
    }
}

#[test]
fn pointwise_math_reports_the_first_logical_fault_not_the_first_storage_cell() {
    equivalent(
        r#"
<< "math";
!^{
    ! print "entered";
    a := |: (form [2n 2n] [1r 0r -1r 2r]);
    ! print ,. (math.pow a -0.5r);
    ! print "unreached";
};
"#,
        Outcome::Fault {
            stdout: "entered\n",
            // -1 to a fractional power fails before 0 to a negative power.
            diagnostic: &[
                "R0001",
                "negative base needs an integral exponent",
                "test.flx:6:",
            ],
        },
        &[],
    );
}
