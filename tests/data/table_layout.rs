use super::specialised_arrays::{equivalent, Outcome};

#[test]
fn column_projection_and_chained_selection_preserve_exact_values() {
    equivalent(
        r#"
t := #: .{
    code := ["aa" "bb" "cc" "dd"];
    price := [12.340d -2.0d 0.000d 7.10d];
};
g := ?: [1b 0b 1b 1b] t;
r := rev g;
s := sect 2n 2n r;
!^{
    ! print (# t, .price t, .code s, .price s);
    ! print :# s;
};
"#,
        Outcome::Ok(concat!(
            "(4, [12.340 -2.0 0.000 7.10], [cc aa], [0.000 12.340])\n",
            "[{code := cc; price := 0.000} {code := aa; price := 12.340}]\n",
        )),
        &[],
    );
}

#[test]
fn zero_column_tables_keep_height_and_positional_tables_keep_order() {
    equivalent(
        r#"
zero := #[ .{} .{} .{} ];
filtered := ?: [0b 1b 1b] zero;
s := sect 2n 1n filtered;
p := #[ (10z,"aa") (20z,"bb") ];
!^{
    ! print (# zero, # filtered, # s, :# s);
    ! print (.1 (rev p), :# (rev p));
};
"#,
        Outcome::Ok("(3, 2, 1, [{}])\n([20 10], [(20, bb) (10, aa)])\n"),
        &[],
    );
}

#[test]
fn row_order_is_not_erased_by_transposition_eligibility() {
    equivalent(
        r#"
mixed := #[ .{ b := 2z; a := 1z; } .{ a := 3z; b := 4z; } ];
!^{
    ! print mixed;
    ! print :# mixed;
    ! print .a mixed;
};
"#,
        Outcome::Ok(concat!(
            "#[{b := 2; a := 1} {b := 4; a := 3}]\n",
            "[{b := 2; a := 1} {a := 3; b := 4}]\n",
            "[1 3]\n",
        )),
        &[],
    );
}

#[test]
fn column_length_and_selection_errors_match_the_reference() {
    equivalent(
        r#"!^{ ! print #: .{ a := [1z 2z]; b := [3z]; }; };"#,
        Outcome::Fault {
            stdout: "",
            diagnostic: &["record columns have unequal lengths"],
        },
        &[],
    );
    equivalent(
        r#"
tab := #: .{ val := [1z 2z]; };
!^{ ! print "before"; ! print ?: [1b] tab; };
"#,
        Outcome::Fault {
            stdout: "before\n",
            diagnostic: &["mask length differs from collection length"],
        },
        &[],
    );
}
