//! Benefits and observable constraints of direct checked dense ingress.
use super::{
    script::Script,
    specialised_arrays::{configuration, equivalent, Outcome},
};

#[test]
fn direct_range_admits_the_destination_without_a_boxed_staging_buffer() {
    let source = r#"
!^{
    ! print "entered";
    a := form [20000n] (.. 1z 20000z);
    ! print (# a, .: [1n] a, .: [20000n] a);
};
"#;
    equivalent(
        source,
        Outcome::Ok("entered\n(20000, ?1 1, ?1 20000)\n"),
        &[],
    );
    let fixture = Script::new("flux-packed-range", "test.flx", source);
    let budget = [("FLUX_MAX_MEMORY", "170000")];
    for mode in [None, Some("run-hir")] {
        let mut packed = fixture.command(mode);
        configuration(&mut packed, false, &budget);
        let packed = packed.output().expect("run compact range");
        assert!(
            packed.status.success() && packed.stderr.is_empty(),
            "{mode:?}: {}",
            String::from_utf8_lossy(&packed.stderr)
        );
        assert_eq!(packed.stdout, b"entered\n(20000, ?1 1, ?1 20000)\n");

        // The generic executor must still reserve its actual boxed staging.
        let mut generic = fixture.command(mode);
        configuration(&mut generic, true, &budget);
        let generic = generic.output().expect("run generic range");
        assert!(!generic.status.success(), "{mode:?}");
        assert_eq!(generic.stdout, b"entered\n");
        let error = String::from_utf8_lossy(&generic.stderr);
        assert!(
            error.contains("R0001") && error.contains("tracked runtime memory"),
            "{error}"
        );
    }
}

#[test]
fn packed_and_wide_ranges_evaluate_all_operands_once_in_source_order() {
    equivalent(
        r#"
large := ^ 2z 130n;
!^{
    compact := form
        (^? ! !^{ ! print "shape"; [2n] })
        (.. (^? ! !^{ ! print "lo"; -3z }) (^? ! !^{ ! print "hi"; -2z }));
    ! print (dim compact, .: [1n] compact, .: [2n] compact);
    wide := form
        (^? ! !^{ ! print "shape"; [2n] })
        (.. (^? ! !^{ ! print "lo"; large }) (^? ! !^{ ! print "hi"; + large 1z }));
    ! print (dim wide, .: [1n] wide, .: [2n] wide);
};
"#,
        Outcome::Ok(concat!(
            "shape\nlo\nhi\n([2], ?1 -3, ?1 -2)\n",
            "shape\nlo\nhi\n([2], ?1 1361129467683753853853498429727072845824, ",
            "?1 1361129467683753853853498429727072845825)\n",
        )),
        &[],
    );
}

#[test]
fn compact_boundary_ranges_do_not_overrun_either_integer_representation() {
    equivalent(
        r#"
lo := form [2n] (.. -9223372036854775808z -9223372036854775807z);
hi := form [2n] (.. 9223372036854775806z 9223372036854775807z);
nat := form [2n] (.. 18446744073709551614n 18446744073709551615n);
!^{ ! print (,. lo, ,. hi, ,. nat); };
"#,
        Outcome::Ok(concat!(
            "([-9223372036854775808 -9223372036854775807], ",
            "[9223372036854775806 9223372036854775807], ",
            "[18446744073709551614 18446744073709551615])\n",
        )),
        &[],
    );
}

#[test]
fn direct_ingress_keeps_the_original_range_depth_check() {
    equivalent(
        r#"
dims := [2n];
a := form dims (.. 1z 2z);
!^{ ! print # a; };
"#,
        Outcome::Fault {
            stdout: "",
            diagnostic: &["R0001", "recursion limit (2) exceeded", "test.flx:3:"],
        },
        &[("FLUX_MAX_DEPTH", "2")],
    );
}

#[test]
fn shared_memory_denial_keeps_the_range_span_and_precedes_shape_mismatch() {
    equivalent(
        r#"
!^{
    ! print "entered";
    a := form [1n] (.. 3z 4z);
    ! print # a;
};
"#,
        Outcome::Fault {
            stdout: "entered\n",
            diagnostic: &["R0001", "tracked runtime memory", "test.flx:4:"],
        },
        &[("FLUX_MAX_MEMORY", "15")],
    );
}
