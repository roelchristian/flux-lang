use super::script::Script;
use super::specialised_arrays::{configuration, equivalent, Outcome};

#[test]
fn host_element_policy_can_lower_and_raise_the_collection_ceiling() {
    let source = r#"
!^{
    ! print "before";
    m := form [4n 5n] (.. 1z 20z);
    ! print (# m, dim m);
};
"#;
    equivalent(source, Outcome::Ok("before\n(20, [4 5])\n"), &[]);
    equivalent(
        source,
        Outcome::Fault {
            stdout: "before\n",
            diagnostic: &["R0001", "limit"],
        },
        &[("FLUX_MAX_ELEMENTS", "19")],
    );
    // The same override must permit both the source range and the dense form.
    equivalent(
        r#"!^{ ! print (# (form [100001n] (.. 1z 100001z))); };"#,
        Outcome::Ok("100001\n"),
        &[("FLUX_MAX_ELEMENTS", "100001")],
    );
}

#[test]
fn host_rank_policy_applies_to_logical_shapes_including_empty_shapes() {
    let source = r#"
empty:[z] := [];
a := form [2n 0n 3n] empty;
!^{ ! print (dim a, # a); };
"#;
    equivalent(source, Outcome::Ok("([2 0 3], 0)\n"), &[]);
    equivalent(
        source,
        Outcome::Fault {
            stdout: "",
            diagnostic: &["R0001", "rank"],
        },
        &[("FLUX_MAX_RANK", "2")],
    );
}

#[test]
fn memory_and_work_limits_fault_before_executing_later_statements() {
    equivalent(
        r#"
!^{
    ! print "before";
    n := form [4n 4n] (.. 1z 16z);
    ! print # n;
};
"#,
        Outcome::Fault {
            stdout: "before\n",
            diagnostic: &["R0001", "memory"],
        },
        &[("FLUX_MAX_MEMORY", "63")],
    );
    equivalent(
        r#"
<< "arr";
m := form [8n 8n] (.. 1z 64z);
!^{
    ! print "before";
    ! print # (arr.mul m m);
};
"#,
        Outcome::Fault {
            stdout: "before\n",
            diagnostic: &["R0001", "limit"],
        },
        &[("FLUX_MAX_WORK", "400")],
    );
}

#[test]
fn exact_arithmetic_does_not_escape_the_host_bit_budget_on_a_packed_path() {
    equivalent(
        r#"
a := form [2n] [9223372036854775807z 1z];
!^{
    ! print "before";
    ! print ,. (* a a);
};
"#,
        Outcome::Fault {
            stdout: "before\n",
            diagnostic: &["R0001", "bit"],
        },
        &[("FLUX_MAX_EXACT_BITS", "64")],
    );
}

#[test]
fn invalid_host_policy_is_not_silently_ignored() {
    let script = Script::new("flux-machina-invalid", "test.flx", "!^{ ! print 7z; };");
    for (name, bad) in [
        ("FLUX_OPTIMIZE", "perhaps"),
        ("FLUX_MAX_ELEMENTS", "0"),
        ("FLUX_MAX_MEMORY", "-1"),
        ("FLUX_MAX_RANK", "17"),
        ("FLUX_MAX_WORK", "184467440737095516160"),
    ] {
        let field = name.trim_start_matches("FLUX_").to_ascii_lowercase();
        for mode in [None, Some("run-hir")] {
            let mut command = script.command(mode);
            configuration(&mut command, false, &[(name, bad)]);
            let result = command.output().expect("invoke Flux");
            let error = String::from_utf8_lossy(&result.stderr);
            assert!(
                !result.status.success()
                    && result.stdout.is_empty()
                    && (error.contains(name) || error.contains(&field)),
                "{mode:?}: {name}={bad} accepted or not diagnosed: {error}"
            );
        }
    }
}

#[test]
fn added_scan_seed_cannot_cross_the_collection_budget_or_invoke_the_callback() {
    equivalent(
        r#"
<< "arr";
bad(a,b:z):z := // a 0z;
m := form [2n 5n] (.. 1z 10z);
!^{
    ! print "before";
    ! print ,. (arr.scana 2n &bad 0z m);
};
"#,
        Outcome::Fault {
            stdout: "before\n",
            diagnostic: &["R0001", "limit"],
        },
        &[("FLUX_MAX_ELEMENTS", "10")],
    );
}

#[test]
fn host_numerical_budget_remains_fatal_outside_result_recovery() {
    let source = r#"
<< "num";
square(x:r):r := * x x;
!^{
    ! print "before";
    answer := ^= num.diff &square 2r 0.25r { _ : 0r; };
    ! print answer;
};
"#;
    equivalent(source, Outcome::Ok("before\n4\n"), &[]);
    equivalent(
        source,
        Outcome::Fault {
            stdout: "before\n",
            diagnostic: &["R0001", "iteration"],
        },
        &[("FLUX_MAX_NUMERIC_ITERATIONS", "1")],
    );
}

#[test]
fn host_depth_limit_is_respected_by_the_generic_and_optimized_walk() {
    let source = r#"
recur(x:n):n := ?{
    = x 0n : 0n;
    _ : + 1n recur (- x 1n);
};
!^{
    ! print "before";
    ! print (recur 8n);
};
"#;
    equivalent(source, Outcome::Ok("before\n8\n"), &[]);
    equivalent(
        source,
        Outcome::Fault {
            stdout: "before\n",
            diagnostic: &["R0001", "limit"],
        },
        &[("FLUX_MAX_DEPTH", "6")],
    );
}
