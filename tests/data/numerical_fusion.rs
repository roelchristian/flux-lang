//! Observable checks, not proof that a plan was selected. Checked-plan
//! construction and path selection need their own white-box kernel tests.
use super::specialised_arrays::{equivalent, Outcome};

#[test]
fn pure_unary_chains_preserve_signed_zero_and_strided_output() {
    equivalent(
        r#"
real := |: (form [2n 2n] [-0.0r 1r 0.0r -2r]);
bools := |: (form [2n 2n] [0b 1b 1b 1b]);
square/1:array[r]->array[r] := &{ @* * . . };
neg/1:array[r]->array[r] := &{ @. -: . };
!^{
    ! print (dim real, ,. (-: (-: (-: real))), ,. (-: (-: real)));
    ! print (,. (~ (~ (~ bools))), ,. (~ (~ bools)));
    ! print (,. (square real), ,. (neg real));
};
"#,
        Outcome::Ok(concat!(
            "([2 2], [0 -0 -1 2], [-0 0 1 -2])\n",
            "([1b 0b 0b 0b], [0b 1b 1b 1b])\n([0 0 1 4], [0 -0 -1 2])\n"
        )),
        &[],
    );
}

#[test]
fn contractions_keep_left_association_and_do_not_fuse_multiplication_and_addition() {
    equivalent(
        r#"
<< "arr";
a := form [3n] [1e16r 1r -1e16r];
ones := form [3n] [1r 1r 1r];
m := |: (form [3n 2n] [1e16r 1r 1r 1r -1e16r 1r]);
n := |: (form [2n 3n] [1r 1r 1r 1r 2r -1r]);
u := form [2n] [-1r 1.0000000074505806r];
v := form [2n] [1r 0.9999999925494194r];
!^{
    ! print (arr.dot a ones, arr.dot u v, ,. (arr.mul m n));
};
"#,
        Outcome::Ok("(0, 0, [0 20000000000000000 3 2])\n"),
        &[],
    );
}

#[test]
fn nested_failures_keep_operand_before_outer_shape_check() {
    equivalent(
        r#"
!^{
    ! print "before";
    a := form [2n] [1.0e308r 2r];
    b := form [3n] [1r 2r 3r];
    ! print ,. (+ (* a 2r) b);
    ! print "unreachable";
};
"#,
        Outcome::Fault {
            stdout: "before\n",
            diagnostic: &["nonfinite", "test.flx:6:"],
        },
        &[],
    );
}

#[test]
fn an_unselected_branch_is_not_evaluated_and_action_order_survives() {
    equivalent(
        r#"
a := form [2n] [1e308r 2r];
join(a,b:!(()^ioerr)):!(()^ioerr) := !^{ ! a; ! b; };
actions := @: &print (|: (form [2n 2n] ["one" "two" "three" "four"]));
safe := |{
    a;
    ?{ 0b : * $ 2r; _ : -: $; };
    -: $
};
!^{
    ! print (dim safe, == safe a);
    ! (?! (@/ &join (,. actions)));
};
"#,
        Outcome::Ok("([2], 1b)\none\nthree\ntwo\nfour\n"),
        &[],
    );
}

#[test]
fn contractions_read_each_sections_offset_and_stride() {
    equivalent(
        r#"
<< "arr";
ma := form [3n 4n] [
    99z 99z 99z 99z
    99z 2z 4z 99z
    99z 3z 5z 99z
];
mb := form [3n 4n] [
    99z 6z 7z 99z
    99z 8z 9z 99z
    99z 99z 99z 99z
];
left := |: (arr.sect [2n 2n] [2n 2n] ma);
right := arr.sect [1n 2n] [2n 2n] mb;
real_left := |: (arr.sect [2n 2n] [2n 2n] (:r ma));
real_right := arr.sect [1n 2n] [2n 2n] (:r mb);
u := arr.sect [2n] [2n] (form [4n] [99r 2r 3r 99r]);
v := arr.sect [3n] [2n] (form [5n] [99r 99r 5r 7r 99r]);
!^{
    ! print (,. (arr.mul left right), ,. (arr.mul real_left real_right));
    ! print (arr.dot u v, arr.dot (arr.diag real_left) (arr.diag real_right));
};
"#,
        Outcome::Ok("([36 41 64 73], [36 41 64 73])\n(31, 57)\n"),
        &[],
    );
}
