use super::specialised_arrays::{equivalent, Outcome};

#[test]
fn chained_views_observe_logical_row_major_coordinates() {
    equivalent(
        r#"
<< "arr";
m := form [2n 3n 4n] (.. 1z 24z);
p := arr.axes [3n 1n 2n] m;
r := arr.sect [2n 1n 2n] [2n 2n 9n] p;
back := arr.axes [2n 3n 1n] p;
t := |: r;
reform := form [2n 4n] t;
!^{
    ! print (dim p, # p, == m back, .: [4n 2n 3n] p);
    ! print (dim r, ,. r, .: [1n 2n 2n] r, .: [1n 1n 0n] r);
    ! print (dim t, ,. t, dim reform, ,. reform);
    ! print (== r (|: t), == reform t, .: [1n 1n 1n] m);
};
"#,
        Outcome::Ok(concat!(
            "([4 2 3], 24, 1b, ?1 24)\n([2 2 2], [6 10 18 22 7 11 19 23], ?1 22, ?0)\n",
            "([2 2 2], [6 7 18 19 10 11 22 23], [2 4], [6 7 18 19 10 11 22 23])\n",
            "(1b, 0b, ?1 1)\n"
        )),
        &[],
    );
}

#[test]
fn empty_strides_and_rank_zero_views_are_distinct() {
    equivalent(
        r#"
<< "arr";
xs:[z] := [];
a := form [100000n 100000n 0n 100000n] xs;
p := |: a;
blank := arr.sect [1n 1n 100001n 1n] [2n 4n 1n 3n] p;
scalar := form [] [7q];
s := arr.axes [] scalar;
!^{
    ! print (dim p, # p, ,. p, == a (|: p));
    ! print (dim blank, # blank, .: [1n 1n 1n 1n] blank);
    ! print (dim s, # s, .: [] s, .: [1n] s, == scalar (|: s));
};
"#,
        Outcome::Ok(concat!(
            "([100000 0 100000 100000], 0, [], 1b)\n",
            "([2 0 0 3], 0, ?0)\n([], 1, ?1 7, ?0, 1b)\n"
        )),
        &[],
    );
}

#[test]
fn strided_scan_and_fold_preserve_seed_cells_and_callback_order() {
    equivalent(
        r#"
<< "arr";
a := |: (form [2n 3n] (.. 1z 6z));
sub(a,b:z):z := - a b;
s := arr.scana 1n &sub 10z a;
f := arr.fold 1n &sub 10z a;
r := arr.red 2n &sub a;
empty:[z] := [];
zero := arr.scana 2n &sub 10z (|: (form [0n 2n] empty));
!^{
    ! print (dim a, ,. a, dim s, ,. s, ,. f, ,. r);
    ! print (dim zero, ,. zero);
};
"#,
        Outcome::Ok(concat!(
            "([3 2], [1 4 2 5 3 6], [4 2], [10 10 9 6 7 1 4 -5], [4 -5], [?1 -3 ?1 -3 ?1 -3])\n",
            "([2 1], [10 10])\n"
        )),
        &[],
    );
}

#[test]
fn strided_callbacks_raise_the_first_logical_not_physical_error() {
    equivalent(
        r#"
f(x:z):z := ?{
    = x 4z : // 1z 0z;
    = x 2z : ?! (?0);
    _ : x;
};
!^{
    ! print "entered";
    m := |: (form [2n 3n] (.. 1z 6z));
    ! print ,. (@: &f m);
    ! print "unreachable";
};
"#,
        Outcome::Fault {
            stdout: "entered\n",
            diagnostic: &["division by zero", "test.flx:3:"],
        },
        &[],
    );
}
