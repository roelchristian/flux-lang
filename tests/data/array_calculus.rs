use super::script::{self, Script};

fn program(source: &str) -> Script {
    Script::new(
        "flux-array-calculus",
        "main.flx",
        &format!("<< \"arr\";\n{source}"),
    )
}

fn rejects(source: &str, expected: &[&str]) {
    script::rejects(&program(source), source, expected);
}

fn fails_at_runtime(defs: &str, expr: &str, expected: &[&str]) {
    let src = format!("{defs}\n!{{ ^{{ ^? ! print ({expr}); () }} }};");
    let program = program(&src);
    program.check_hir();
    for mode in [None, Some("run-hir")] {
        let out = program.run(mode);
        let stderr = String::from_utf8_lossy(&out.stderr).to_lowercase();
        assert!(
            !out.status.success() && out.stdout.is_empty(),
            "{mode:?}: {src}"
        );
        for fragment in expected {
            assert!(
                stderr.contains(fragment),
                "{mode:?}: expected {fragment:?}, got {stderr:?}"
            );
        }
    }
}

#[test]
fn dense_construction_shape_size_ravel_and_reshape_share_row_major_order() {
    program(
        r#"
a := form [2n 3n] [1z 2z 3z 4z 5z 6z];
b := form [3n 2n] a;
c := arr.reshape [6n] (,. b);
scalar := form [] [9z];
empty:[z] := [];
blank := form [2n 0n 3n] empty;
!{ ^{
    ^? ! print (dim a, # a, ,. a, arr.shape b, arr.ravel c);
    ^? ! print (dim scalar, # scalar, ,. scalar, dim blank, # blank);
    ()
} };
"#,
    )
    .accepts("([2 3], 6, [1 2 3 4 5 6], [3 2], [1 2 3 4 5 6])\n([], 1, [9], [2 0 3], 0)\n");
    fails_at_runtime("", "form [3n] (form [2n] [1z 2z])", &["shape"]);
}

#[test]
fn axis_reversal_coordinates_and_first_class_primitives_preserve_element_types() {
    program(
        r#"
a := form [2n 1n 3n] ["a" "b" "c" "d" "e" "f"];
scalar := form [] [7q];
m:array[s]->array[s] := &|:;
r := @ m a;
indexes:[n]->array[s]->?s := &.:;
!{ ^{
    ^? ! print (dim r, ,. r, == a (|: r));
    ^? ! print (.: [3n 1n 2n] r, @@ indexes ([2n 1n 1n],r));
    ^? ! print (.: [] scalar, .: [1n] scalar, .: [1n 0n 1n] r, .: [4n 1n 1n] r);
    ^? ! print (@ &dim scalar, @ &# scalar, @ &,. scalar, == scalar (|: scalar));
    ()
} };
"#,
    )
    .accepts("([3 1 2], [a d b e c f], 1b)\n(?1 f, ?1 b)\n(?1 7, ?0, ?0, ?0)\n([], 1, [7], 1b)\n");
}

#[test]
fn trailing_broadcast_covers_two_arrays_both_scalar_sides_and_zero_width() {
    program(
        r#"
a := form [2n 1n 3n] [1z 2z 3z 4z 5z 6z];
b := form [2n 1n] [10z 20z];
c := - a b;
d := - 10z (form [2n] [1z 4z]);
e := - (form [2n] [1z 4z]) 10z;
f := <= a b;
scalar := form [] [5z];
empty:[z] := [];
zero := + (form [2n 0n 3n] empty) (form [1n 3n] [1z 2z 3z]);
!{ ^{
    ^? ! print (dim c, ,. c, ,. d, ,. e);
    ^? ! print (dim f, ,. f, dim zero, # zero);
    ^? ! print (dim (+ scalar 3z), ,. (+ scalar 3z));
    ^? ! print (== a (form [6n] (,. a)), == a (form [2n 1n 3n] (,. a)));
    ()
} };
"#,
    )
    .accepts(concat!(
        "([2 2 3], [-9 -8 -7 -19 -18 -17 -6 -5 -4 -16 -15 -14], [9 6], [-9 -6])\n",
        "([2 2 3], [1b 1b 1b 1b 1b 1b 1b 1b 1b 1b 1b 1b], [2 0 3], 0)\n",
        "([], [8])\n(0b, 1b)\n"
    ));
    fails_at_runtime(
        "xs:[z] := [];",
        "+ (form [0n] xs) (form [2n] [1z 2z])",
        &["broadcast"],
    );
}

#[test]
fn dense_numeric_domains_and_capabilities_remain_static() {
    program(
        r#"
square(x:~a):~a | num ~a := * x x;
a := form [2n] [1.20d 0.3d];
b := form [2n] [2.0d 3d];
c := @: &square a;
sumf:array[d]->array[d]->array[d] := &+;
add_scalar:array[d]->d->array[d] := &+;
p := @@ sumf (a,b);
s := @@ add_scalar (a,1.0d);
q := / a b;
opts := @: (\x -> ?1 x) a;
bools := ~ (form [3n] [0b 1b 0b]);
!{ ^{
    ^? ! print (,. c, ,. p, ,. s, ,. q, ,. opts, ,. bools);
    ^? ! print (,. (-: a), ,. (:q a), ,. (&& bools (form [1n] [1b])));
    ()
} };
"#,
    )
    .accepts(concat!(
        "([1.4400 0.09], [3.20 3.3], [2.20 1.3], [3/5 1/10], [?1 1.20 ?1 0.3], [1b 0b 1b])\n",
        "([-1.20 -0.3], [6/5 3/10], [1b 0b 1b])\n"
    ));
    rejects("a:=form [2n] [1z 2z]; b:=+ a [1z 2z];", &["E0200"]);
    rejects("a:=form [1n] [1q]; b:=+ a (form [1n] [1r]);", &["E0200"]);
    rejects("id(x:z):z:=x; a:=form [1n] [&id]; b:= == a a;", &["E0200"]);
    rejects("a := dim [[1z 2z] [3z]];", &["E0200"]);
}

#[test]
fn final_axis_reduction_folds_left_and_ramps_keep_seed() {
    program(
        r#"
a := form [2n 2n 2n] [10z 3z 12z 4z 11z 1z 9z 4z];
total := +/ a;
unseeded := @/ &- a;
fromseed := /: &- 100z a;
progress := \: &- 100z a;
vec := form [3n] [10z 3z 2z];
g:((z->z->z))->array[z]->array[?z] := &@/;
!{ ^{
    ^? ! print (dim total, ,. total, ,. (*/ a), ,. (>/ a), ,. (</ a));
    ^? ! print (dim unseeded, ,. unseeded, dim fromseed, ,. fromseed);
    ^? ! print (dim progress, ,. progress);
    ^? ! print (dim (@/ &- vec), ,. (@/ &- vec), ,. (@@ g (&-,vec)));
    ()
} };
"#,
    )
    .accepts(concat!(
        "([2 2], [13 16 12 13], [30 48 11 36], [?1 10 ?1 12 ?1 11 ?1 9], [?1 3 ?1 4 ?1 1 ?1 4])\n",
        "([2 2], [?1 7 ?1 8 ?1 10 ?1 5], [2 2], [87 84 88 87])\n",
        "([2 2 3], [100 90 87 100 88 84 100 89 88 100 91 87])\n([], [?1 5], [?1 5])\n"
    ));
}

#[test]
fn empty_and_scalar_cells_use_distinct_reduction_contracts() {
    program(
        r#"
empty:[z] := [];
flags:[b] := [];
bad(x,y:z):z := // x 0z;
a := form [2n 0n] empty;
b := form [2n 0n] flags;
none := form [0n 2n] empty;
singles := form [2n 1n] [4z 6z];
!{ ^{
    ^? ! print (dim (+/ a), ,. (+/ a), ,. (*/ a), ,. (>/ a), ,. (@/ &bad a));
    ^? ! print (,. (&&/ b), ,. (||/ b), ,. (@/ &bad singles));
    ^? ! print (dim (/: &bad 9z a), ,. (/: &bad 9z a), dim (\: &bad 9z a), ,. (\: &bad 9z a));
    ^? ! print (dim (@/ &bad none), ,. (@/ &bad none));
    ()
} };
"#,
    )
    .accepts(concat!(
        "([2], [0 0], [1 1], [?0 ?0], [?0 ?0])\n",
        "([1b 1b], [0b 0b], [?1 4 ?1 6])\n([2], [9 9], [2 1], [9 9])\n([0], [])\n"
    ));
    fails_at_runtime("", "+/ (form [] [1z])", &["rank"]);
    fails_at_runtime("", "arr.fold 1n &+ 0z (form [] [1z])", &["axis"]);
}

#[test]
fn explicit_axis_reductions_and_scans_keep_nonreduced_axes_in_order() {
    program(
        r#"
a := form [2n 2n 3n] (.. 1z 12z);
red := arr.red 2n &- a;
fold := arr.fold 1n &- 100z a;
scan := arr.scana 2n &- 100z a;
per := arr.axes [3n 1n 2n] a;
part := arr.sect [1n 2n 2n] [9n 1n 9n] a;
blank := arr.sect [1n 3n 1n] [2n 2n 3n] a;
scalar := arr.axes [] (form [] [7z]);
!{ ^{
    ^? ! print (dim red, ,. red, dim fold, ,. fold);
    ^? ! print (dim scan, ,. scan, dim per, ,. per);
    ^? ! print (dim part, ,. part, dim blank, # blank, dim scalar, ,. scalar);
    ()
} };
"#,
    )
    .accepts(concat!(
        "([2 3], [?1 -3 ?1 -3 ?1 -3 ?1 -3 ?1 -3 ?1 -3], [2 3], [92 90 88 86 84 82])\n",
        "([2 3 3], [100 100 100 99 98 97 95 93 91 100 100 100 93 92 91 83 81 79], [3 2 2], [1 4 7 10 2 5 8 11 3 6 9 12])\n",
        "([2 1 2], [5 6 11 12], [2 0 3], 0, [], [7])\n"
    ));
}

#[test]
fn axis_and_shape_validation_precedes_use_and_resource_overflow() {
    let a = "a:=form [2n 2n] [1z 2z 3z 4z];";
    for expr in [
        "arr.red 0n &- a",
        "arr.fold 3n &- 9z a",
        "arr.scana 0n &+ 0z a",
    ] {
        fails_at_runtime(a, expr, &["axis"]);
    }
    for expr in [
        "arr.axes [1n] a",
        "arr.axes [1n 1n] a",
        "arr.axes [0n 1n] a",
    ] {
        fails_at_runtime(a, expr, &["permut"]);
    }
    fails_at_runtime(a, "arr.sect [0n 1n] [2n 2n] a", &["start"]);
    fails_at_runtime(a, "arr.sect [1n] [2n 2n] a", &["rank"]);
    fails_at_runtime(
        "bad(x,y:z):z := // x 0z; a := form [100000n] (.. 1z 100000z);",
        "arr.scana 1n &bad 0z a",
        &["limit"],
    );
    let overrank = format!("form [{}] [0z]", vec!["1n"; 17].join(" "));
    fails_at_runtime("", &overrank, &["rank"]);
}

#[test]
fn dot_matrix_diagonal_and_empty_contractions_are_exact() {
    program(
        r#"
a := form [2n 3n] [1q 2q 3q 4q 5q 6q];
b := form [3n 2n] [/ 1q 2q 0q 0q / 1q 3q / 1q 2q / 1q 3q];
m := arr.mul a b;
d := arr.dot (form [2n] [1.20d 0.30d]) (form [2n] [2.0d 3d]);
xs:[q] := [];
zero := arr.mul (form [2n 0n] xs) (form [0n 3n] xs);
emptydot := arr.dot (form [0n] xs) (form [0n] xs);
!{ ^{
    ^? ! print (dim m, ,. m, dim (arr.diag a), ,. (arr.diag a), d);
    ^? ! print (dim zero, ,. zero, emptydot);
    ()
} };
"#,
    )
    .accepts("([2 2], [2 5/3 5 11/3], [2], [1 5], 3.300)\n([2 3], [0 0 0 0 0 0], 0)\n");
    fails_at_runtime(
        "",
        "arr.dot (form [2n] [1z 2z]) (form [1n] [1z])",
        &["length"],
    );
    fails_at_runtime("", "arr.diag (form [1n] [1z])", &["rank"]);
}

#[test]
fn array_map_preserves_suspended_effects_and_ravel_order() {
    program(
        r#"
a := form [2n 2n] ["north" "east" "south" "west"];
writes := @: &print a;
emit(a,b:!(()^ioerr)):!(()^ioerr) := !{ ^{ ^? ! a; ^? ! b; () } };
!{ ^{
    ^? ! print (dim writes);
    ^? ! (?! (@/ &emit (,. writes)));
    ()
} };
"#,
    )
    .accepts("[2 2]\nnorth\neast\nsouth\nwest\n");
}

#[test]
fn dense_power_broadcasts_natural_exponents_on_either_side() {
    program(
        r#"
base := form [2n 1n] [2z 3z];
exp := form [1n 3n] [1n 2n 3n];
numeric:array[z]->array[n]->array[z] := &^;
!{ ^{
    ^? ! print (dim (^ base exp), ,. (^ base exp), ,. (@@ numeric (base,exp)));
    ^? ! print (,. (^ 2z exp), ,. (^ base 2n), ,. (^/ (form [2n] [4q 9q])));
    ()
} };
"#,
    )
    .accepts("([2 3], [2 4 8 3 9 27], [2 4 8 3 9 27])\n([2 4 8], [4 9], [2 3])\n");
    rejects("a:=form [2n] [1z 2z]; b:= ^ 2z a;", &["E0200"]);
}
