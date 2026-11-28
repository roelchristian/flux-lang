use super::script::Script;
use std::fs;

fn script(source: &str) -> Script {
    Script::new("flux-arrays", "main.flx", source)
}

fn rejects(source: &str, words: &[&str]) {
    super::script::rejects(&script(source), source, words);
}

fn array_program(source: &str) -> Script {
    script(&format!("<< \"arr\";\n{source}"))
}

fn runtime_rejects(expr: &str, fragment: &str) {
    let fixture = array_program(&format!("!{{ ^{{ ^? ! print ({expr}); () }} }};"));
    assert!(
        fixture.run(Some("check")).status.success(),
        "{expr} must type-check"
    );
    for mode in ["run-hir", ""] {
        let out = fixture.run((!mode.is_empty()).then_some(mode));
        let error = String::from_utf8_lossy(&out.stderr);
        assert!(
            !out.status.success() && out.stdout.is_empty(),
            "{mode}: {expr}"
        );
        assert!(
            error.to_lowercase().contains(fragment),
            "{mode}: {expr}: {error}"
        );
    }
}

#[test]
fn trailing_axis_broadcast_and_one_based_coordinates() {
    array_program(
        r#"
left := arr.reshape [2n 1n 3n] [1z 2z 3z 4z 5z 6z];
right := arr.reshape [2n 1n] [10z 20z];
out := arr.zip &+ left right;
!{ ^{
    ^? ! print (arr.shape out,arr.ravel out);
    ^? ! print (arr.at [2n 2n 3n] out,arr.at [0n 1n 1n] out,
        arr.at [1n 3n 1n] out,arr.at [2n 3n] out);
    ()
} };
"#,
    )
    .accepts("([2 2 3], [11 12 13 21 22 23 14 15 16 24 25 26])\n(?1 26, ?0, ?0, ?0)\n");
}

#[test]
fn reshape_does_not_flatten_ragged_sequences_or_merge_types() {
    array_program(
        r#"
ragged := arr.reshape [2n] [[1z] [2z 3z]];
!{ ^{ ^? ! print (arr.shape ragged,arr.ravel ragged); () } };
"#,
    )
    .accepts("([2], [[1] [2 3]])\n");
    runtime_rejects("arr.reshape [2n 2n] [1z 2z 3z]", "shape");
    runtime_rejects(
        "arr.zip &+ (arr.reshape [2n] [1z 2z]) (arr.reshape [3n] [1z 2z 3z])",
        "broadcast",
    );
}

#[test]
fn rank_zero_and_empty_axes_have_distinct_shapes() {
    array_program(
        r#"
empty:[z] := [];
s := arr.reshape [] [7z];
zero := arr.reshape [2n 0n 3n] empty;
singleton := arr.reshape [1n 3n] [1z 2z 3z];
b := arr.zip &+ zero singleton;
!{ ^{
    ^? ! print (arr.shape s,arr.ravel s,arr.at [] s,arr.at [1n] s);
    ^? ! print (arr.shape zero,arr.ravel zero,arr.shape b,arr.at [1n 1n 1n] b);
    ()
} };
"#,
    )
    .accepts("([], [7], ?1 7, ?0)\n([2 0 3], [], [2 0 3], ?0)\n");
    runtime_rejects("arr.reshape [] [1z 2z]", "shape");
    runtime_rejects("arr.scan &+ 0z (arr.reshape [] [7z])", "rank");
}

#[test]
fn outer_concatenates_shapes_and_uses_row_major_order() {
    array_program(
        r#"
left := arr.reshape [2n] [2z 3z];
right := arr.reshape [1n 3n] [4z 5z 6z];
product := arr.outer &* left right;
!{ ^{ ^? ! print (arr.shape product,arr.ravel product); () } };
"#,
    )
    .accepts("([2 1 3], [8 10 12 12 15 18])\n");
}

#[test]
fn rankmap_checks_rank_and_calls_both_scalar_and_empty_cells() {
    array_program(
        r#"
sum(cell:array[z]):z := +/ arr.ravel cell;
measure(cell:array[z]):n := # arr.ravel cell;
never(cell:array[z]):n := // 1n 0n;
empty:[z] := [];
a := arr.reshape [2n 3n] [1z 2z 3z 4z 5z 6z];
r1 := arr.rankmap 1n &sum a;
r0 := arr.rankmap 0n &sum a;
all := arr.rankmap 2n &sum a;
e := arr.rankmap 1n &measure (arr.reshape [2n 0n] empty);
no_cells := arr.rankmap 1n &never (arr.reshape [0n 2n] empty);
!{ ^{
    ^? ! print (arr.shape r1,arr.ravel r1,arr.shape r0,arr.ravel r0);
    ^? ! print (arr.shape all,arr.ravel all,arr.shape e,arr.ravel e);
    ^? ! print (arr.shape no_cells,arr.ravel no_cells);
    ()
} };
"#,
    )
    .accepts("([2], [6 15], [2 3], [1 2 3 4 5 6])\n([], [21], [2], [0 0])\n([0], [])\n");
    runtime_rejects(
        "arr.rankmap 2n (\\x -> # arr.ravel x) (arr.reshape [1n] [1z])",
        "rank",
    );
}

#[test]
fn last_axis_scan_is_seeded_per_frame_cell_including_empty_axis() {
    array_program(
        r#"
a := arr.reshape [2n 3n] [1z 2z 3z 4z 5z 6z];
result := arr.scan &+ 10z a;
empty:[z] := [];
seeded := arr.scan &+ 10z (arr.reshape [2n 0n] empty);
!{ ^{
    ^? ! print (arr.shape result,arr.ravel result);
    ^? ! print (arr.shape seeded,arr.ravel seeded);
    ()
} };
"#,
    )
    .accepts("([2 4], [10 11 13 16 10 14 19 25])\n([2 1], [10 10])\n");
}

#[test]
fn callbacks_reuse_checked_generic_evidence_and_preserve_decimal_scale() {
    array_program(
        r#"
square(x:~a):~a | num ~a := * x x;
map(f:~a->~b,data:array[~a]):array[~b] := arr.map f data;
a := arr.reshape [2n] [1.20d 2.0d];
b := arr.reshape [2n] [3n 4n];
c := arr.zip &+ a a;
!{ ^{
    ^? ! print (arr.ravel (map &square a),arr.ravel (map &square b),arr.ravel c);
    ()
} };
"#,
    )
    .accepts("([1.4400 4.00], [9 16], [2.40 4.0])\n");
}

#[test]
fn windows_gather_grade_and_permutation_are_stable_and_checked() {
    array_program(
        r#"
xs := [10z 20z 30z 40z];
key(x:(z,s)):z := .1 x;
rows := [(2z,"x") (1z,"first") (1z,"second") (3z,"last")];
g := arr.grade &key rows;
!{ ^{
    ^? ! print (arr.windows 3n xs,arr.windows 5n xs);
    ^? ! print (arr.select [4n 1n 4n] xs,arr.select [0n] xs,arr.select [5n] xs);
    ^? ! print (g,arr.permute g rows);
    ()
} };
"#,
    ).accepts("([[10 20 30] [20 30 40]], [])\n(?1 [40 10 40], ?0, ?0)\n([2 3 1 4], [(1, first) (1, second) (2, x) (3, last)])\n");
    runtime_rejects("arr.windows 0n [1z]", "width");
    runtime_rejects("arr.permute [1n 1n] [1z 2z]", "permut");
    runtime_rejects("arr.permute [1n] [1z 2z]", "permut");
    runtime_rejects("arr.permute [1n 3n] [1z 2z]", "permut");
}

#[test]
fn checks_array_argument_callback_and_order_domains() {
    rejects("<< \"arr\"; x:array[z] := [1z];", &["E0200"]);
    rejects(
        "<< \"arr\"; x := @ &arr.shape [1z];",
        &["E0200", "expected: array[", "actual: [z]"],
    );
    rejects(
        "<< \"arr\"; x := arr.map (\\x -> + x 1z) (arr.reshape [1n] [\"s\"]);",
        &["E0200"],
    );
    rejects("<< \"arr\"; x := arr.grade (\\x -> x) [1b 0b];", &["ord"]);
}

#[test]
fn huge_shape_is_rejected_without_allocating_a_result() {
    runtime_rejects("arr.reshape [100001n] [1z]", "limit");
}

#[test]
fn imported_array_signature_instantiates_across_element_types() {
    let fixture = array_program(
        r#"
<< "types.flx" : t;
a := t.keep (arr.reshape [] [2.00d]);
b := @ &t.keep (arr.reshape [1] ["flux"]);
!{ ^{ ^? ! print (arr.ravel a,arr.ravel b); () } };
"#,
    );
    fs::write(
        fixture.path().join("types.flx"),
        ">> (keep); keep(x:array[~a]):array[~a] := x;",
    )
    .unwrap();
    fixture.accepts("([2.00], [flux])\n");
}
