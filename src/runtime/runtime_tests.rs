//! Internal typed-runtime regression tests.

use super::*;
use crate::diagnostic::Category;
use num_bigint::BigInt;
use num_rational::BigRational;

fn compile(source: &str) -> HirProgram {
    let p = crate::parser::parse(source).unwrap();
    let names = crate::resolve::resolve(&p).unwrap();
    let types = crate::infer::infer(&p, &names).unwrap();
    crate::hir::lower(&p, &types, &names).unwrap()
}

#[test]
fn owned_typed_hir_runs_after_surface_tree_is_dropped() {
    let ir = compile(
        r#"
        << "io"; emit(x) := !{ ^= ! io.print x { _ : (); } };
        id(x) := x;
        f := |{ 4; \y -> + $ y };
        !{
            ! emit id 9;
            ! emit id "polymorphic";
            ! emit @ f 5;
        };
    "#,
    );
    let result = run(&ir).unwrap();
    assert_eq!(result.stdout, "9\npolymorphic\n9\n");
    assert_eq!(result.status, 0);
}

#[test]
fn typed_pipeline_and_lexical_closures() {
    let ir = compile(
        r#"
        << "io"; emit(x) := !{ ^= ! io.print x { _ : (); } };
        twice(x:z):z := + x x;
        !{
            x := |{ ! !{ ! emit "once"; 4 }; + $ $; @ &twice $ };
            ! emit x;
        };
    "#,
    );
    assert_eq!(run(&ir).unwrap().stdout, "once\n16\n");
}

#[test]
fn typed_hir_runtime_domain_error_and_lazy_branches() {
    let good = compile(
        r#" << "io"; emit(x) := !{ ^= ! io.print x { _ : (); } }; !{ ! emit ? 1b (/ 3 2) (/ 1 0); }; "#,
    );
    assert_eq!(run(&good).unwrap().stdout, "3/2\n");
    let bad = compile("a := 1;\nb := / a 0;\n!{ (); };");
    let error = run(&bad).err().unwrap();
    assert_eq!(error.start, 13);
    assert!(error.message.contains("division by zero"));
}
#[test]
fn typed_row_generalization_runs_without_legacy() {
    let ir = compile(
        r#"
        << "io"; emit(x) := !{ ^= ! io.print x { _ : (); } };
        amount(x) := .amount x;
        !{
            ! emit amount .{ amount:=12; label:="kept"; };
            ! emit amount .{ amount:=9; approved:=1b; };
        };
    "#,
    );
    assert_eq!(run(&ir).unwrap().stdout, "12\n9\n");
    assert!(ir
        .arena
        .iter()
        .any(|n| matches!(n.kind, ExprKind::Record(_))));
}
#[test]
fn typed_table_aggregate_uses_static_layout_even_for_no_rows() {
    let ir = compile(
        r#"
        << "df";
        helper(k:s) := .{ station:=k; impossible:=/ 1 0; };
        summarize(k:s,g:#.{station:s;}) := helper k;
        !{
            t:#.{station:s;} := #: .{ station:=[]; };
            df.agg &.station &summarize t
        };
    "#,
    );
    let p = view_entry(&ir).unwrap();
    assert_eq!(p.value, "station | impossible\n--------+-----------");
    assert!(p.stdout.is_empty());
}
#[test]
fn lifted_direct_calls_encode_all_static_shape_modes() {
    use LiftMode::*;
    let ir = compile(
        r#"
        << "io"; emit(x) := !{ ^= ! io.print x { _ : (); } };
        x := + 1 2;
        y := - [4 6] 1;
        z := - 9 [1 2];
        w := * [1 2] [2 3];
        !{ ! emit (x,y,z,w); };
    "#,
    );
    let found = ir
        .groups
        .iter()
        .flatten()
        .filter_map(|d| ir.node(d.body).lift_mode)
        .collect::<Vec<_>>();
    assert_eq!(found, vec![Scalar, LeftSequence, RightSequence, Pairwise]);
    assert_eq!(run(&ir).unwrap().stdout, "(3, [3 5], [8 7], [2 6])\n");
    let ir = compile("x := + [1 2] [3]; !{ (); };");
    assert!(run(&ir)
        .err()
        .unwrap()
        .message
        .contains("sequence lengths differ"));
}
#[test]
fn checked_instances_cover_numeric_closures_and_indirect_broadcast() {
    let ir = compile(
        r#"
        << "io"; emit(x) := !{ ^= ! io.print x { _ : (); } };
        twice(x) := + x x;
        through(f,x) := @ f x;
        twice_with(f,x) := @ (@ f x) x;
        plus := &+;
        factory(x) := \y -> + x y;
        add:[z] z->[z] := &+;
        scale:z [z]->[z] := &*;
        pair:[z] [z]->[z] := &-;
        f := factory 0.5q;
        !{
            ! emit twice 4z;
            ! emit twice 0.25q;
            ! emit twice 0.25r;
            ! emit through &twice 0.25r;
            ! emit twice_with plus 0.25q;
            ! emit @ f 0.25q;
            ! emit @@ add ([1 2],4);
            ! emit @ (@ scale 2) [4 5];
            ! emit @@ pair ([9 8],[3 2]);
        };
    "#,
    );
    assert_eq!(
        run(&ir).unwrap().stdout,
        "8\n1/2\n0.5\n0.5\n1/2\n3/4\n[5 6]\n[8 10]\n[6 6]\n"
    );
    assert!(ir.arena.iter().any(|n|matches!(&n.callee_type,Some(Type::Fun(arg,out)) if **arg==Type::Prim(Prim::R)&&**out==Type::Prim(Prim::R))));
}

#[test]
fn indirect_pairwise_preserves_the_shape_check() {
    let ir = compile("add:[z] [z]->[z] := &+; fail := @@ add ([1 2],[3]); !{ (); };");
    let err = run(&ir).err().unwrap();
    assert!(err.message.contains("shape mismatch"));
}

#[test]
fn generic_natural_instance_preserves_underflow() {
    let ir = compile("sub(x,y) := - x y; fail := sub 1n 2n; !{ (); };");
    assert!(run(&ir)
        .err()
        .unwrap()
        .message
        .contains("natural subtraction underflow"));
}
#[test]
fn failure_provenance_is_not_part_of_result_equality() {
    let ir = compile(
        r#"
        first(x:z):z^z := ^{ ^: x };
        second(x:z):z^z := ^{ ^? first x };
        !{ == (first 3z) (second 3z) };
    "#,
    );
    assert_eq!(view_entry(&ir).unwrap().value, "1b");
}

#[test]
fn optional_tuple_keys_and_duplicate_join_rows_are_stable() {
    let ir = compile(
        r#"
        l:#(?s,z) := #[(?0,1) (?1 "x",2) (?0,3)];
        r:#(?s,z) := #[(?0,4) (?0,5) (?1 "y",6)];
        key(row:(?s,z)) := (.1 row,0z);
        j := *= &key &key l r;
        wanted:#((?s,z),(?s,z)) := #[
            ((?0,1),(?0,4)) ((?0,1),(?0,5))
            ((?0,3),(?0,4)) ((?0,3),(?0,5))
        ];
        !{ == j wanted };
    "#,
    );
    assert_eq!(view_entry(&ir).unwrap().value, "1b");
}

#[test]
fn real_signed_zeros_share_one_join_key() {
    let ir = compile(
        r#"
        left:#(r,z) := #[(-0.0r,4z)];
        right:#(r,z) := #[(0.0r,5z)];
        !{ # (*= &.1 &.1 left right) };
    "#,
    );
    assert_eq!(view_entry(&ir).unwrap().value, "1");
}

#[test]
fn empty_join_keeps_instantiated_paired_layout() {
    let ir = compile(
        r#"
        left:#(z,s) := #[];
        right:#(z,b) := #[(1,1b)];
        lk(row:(z,s)) := .1 row;
        rk(row:(z,b)) := .1 row;
        outer := &*?;
        !{ @@ outer (&lk,&rk,left,right) };
    "#,
    );
    let out = view_entry(&ir).unwrap().value;
    assert!(out.contains(".1 | .2"), "{out}");
    assert_eq!(out.lines().count(), 2, "{out}");
}

#[test]
fn tacit_lowering_equivalents_keep_polymorphism_and_numeric_evidence() {
    let ir = compile(
        r#"
        square := \x -> @@ &* (x,x);
        square_field := \x -> @@ &* (@ &.value x,@ &.value x);
        field := \x -> @ &.value (@ &.inner x);
        vector_square:[n]->[n] := \x -> @@ &* (x,x);
        curried := \x -> \y -> @@ &+ (x,y);
        !{
            (@ square 3n, @ square 0.5q,
             @ square_field .{ value:=3n; text:="ignored"; },
             @ square_field .{ value:=0.5q; tag:=1b; },
             @ field .{ inner:=.{value:="text"; extra:=1n;}; },
             @ vector_square [2n 3n],
             @@ curried (2n,3n), @@ curried (0.25q,0.5q))
        };
    "#,
    );
    assert_eq!(
        view_entry(&ir).unwrap().value,
        "(9, 1/4, 9, 1/4, \"text\", [4 9], 5, 3/4)"
    );
}

#[test]
fn tacit_fork_lowering_evaluates_the_left_branch_first() {
    let ir = compile(
        r#"
        left(x:n):n := - x 10n;
        right(x:n):n := // x 0n;
        both := \x -> @@ &+ (@ &left x,@ &right x);
        !{ @ both 1n };
    "#,
    );
    let err = view_entry(&ir).err().unwrap();
    assert!(
        err.message.contains("natural subtraction underflow"),
        "{}",
        err.message
    );
}

#[test]
fn result_combinators_keep_first_failure_and_failure_provenance() {
    let source = r#"
        << "res";
        first(x:z):z^z := ^{ ^: x };
        second(x:z):z^z := ^{ ^? first x };
        shifted := res.maperr (\x -> + x 100z) res.collect [second 5z first 2z];
        !{ ^{ ^? shifted; () } };
    "#;
    let ir = compile(source);
    let err = run(&ir).err().unwrap();
    assert!(err.message.contains("105"), "{}", err.message);
    let diagnostic = err.structured().unwrap();
    assert_eq!(
        &source[diagnostic.primary.start..diagnostic.primary.end],
        "^: x"
    );
    assert!(diagnostic
        .related
        .iter()
        .any(|r| r.label.contains("propagat")));
}

#[test]
fn option_combinators_use_checked_callback_instances_and_skip_missing_payload() {
    let ir = compile(
        r#"
        << "opt";
        choose := &opt.map;
        inc(x) := + x 1n;
        bad(x:n):n := - x 99n;
        none:?n := ?0;
        !{ (@@ choose (&inc,?1 5n), opt.map &bad none) };
    "#,
    );
    assert_eq!(view_entry(&ir).unwrap().value, "(?1 6, ?0)");
}

#[test]
fn recover_selects_one_arm_once_and_wildcard_payload_slots_do_not_bind() {
    let ir = compile(
        r#"
        trouble :: { | other; | both z z; };
        raise_two(x:z):z^trouble := ^{ ^: both 9z x };
        !{ ^= raise_two 3z { other : // 2z 0z; both _ y : y; } };
    "#,
    );
    assert_eq!(view_entry(&ir).unwrap().value, "3");
}

#[test]
fn uncaught_result_contains_original_span_and_each_propagation() {
    let source = "first(x:z):z^z := ^{ ^: x };
second(x:z):z^z := ^{ ^? first x };
!{ ^{ lost := ^? second 5z; () } };";
    let ir = compile(source);
    let err = match run(&ir) {
        Ok(_) => panic!("failure became successful"),
        Err(e) => e,
    };
    let d = err.structured().unwrap();
    assert_eq!(d.category, Category::RuntimeOperational);
    assert!(d.message.contains("z: 5"), "{}", d.message);
    assert_eq!(d.primary.start, source.find("^:").unwrap());
    assert_eq!(
        d.related
            .iter()
            .map(|note| note.span.start)
            .collect::<Vec<_>>(),
        source
            .match_indices("^?")
            .map(|(i, _)| i)
            .collect::<Vec<_>>()
    );
}

#[test]
fn runtime_result_instantiation_is_polymorphic() {
    let ir = compile(
        r#"
        keep(x:~a):~a^z := ^{ x };
        get(fallback:~a,wrapped:~a^z):~a := ^= wrapped { _ : fallback; };
        !{ (get 1z (keep 8z), get "stale" (keep "fresh")) };
    "#,
    );
    assert_eq!(view_entry(&ir).unwrap().value, "(8, \"fresh\")");
}
#[test]
fn injectable_reader_drives_action_and_does_not_reexecute_it() {
    struct Host {
        reads: Vec<String>,
    }
    impl RuntimeInput for Host {
        fn open(&mut self, path: &str) -> std::io::Result<OpenedInput> {
            self.reads.push(path.into());
            let bytes = b"injected data";
            Ok(OpenedInput {
                reader: Box::new(std::io::Cursor::new(bytes)),
                byte_len: Some(bytes.len() as u64),
            })
        }
    }
    let ir = compile(
        r#"
        << "io";
        !{ ^{
            data := ^? ! io.read "virtual-fixture";
            ^? ! io.print data;
            ()
        } };
    "#,
    );
    let mut host = Host { reads: vec![] };
    let mut output = Vec::new();
    let status = run_with_io(&ir, &mut host, &mut output).unwrap();
    assert_eq!(status.status, 0);
    assert_eq!(status.stdout, "");
    assert_eq!(host.reads, ["virtual-fixture"]);
    assert_eq!(output, b"injected data\n");
}
#[test]
fn registry_collection_calls_keep_checked_higher_order_instances() {
    let ir = compile(
        r#"
        << "io";
        emit(x) := !{ ^= ! io.print x { _ : (); } };
        fold := /: &+ 0z [1z 2z 3z];
        scan := \: &+ 0z [1z 2z 3z];
        concat:[z] [z]->[z] := &+:;
        lookup:n [z]->?z := &.:;
        varof:[q]->?q := &var;
        sqrt:z->r := &^/;
        !{
            ! emit (fold, scan);
            ! emit @ (@ concat [1z]) [2z];
            ! emit @@ lookup (2n, [1z 2z]);
            ! emit @ varof [1q 2q 3q];
            ! emit @ sqrt 9z;
        };
    "#,
    );
    assert_eq!(
        run(&ir).unwrap().stdout,
        "(6, [0 1 3 6])\n[1 2]\n?1 2\n?1 2/3\n3\n"
    );
}
#[test]
fn exact_small_values_stay_inline_and_wide_results_demote() {
    #[cfg(target_pointer_width = "64")]
    assert!(std::mem::size_of::<V>() <= 48);
    let big = exact_binary(
        CoreOp::Add,
        &exact_i128(i128::MAX, Domain::Integer),
        &exact_i128(1, Domain::Integer),
        Domain::Integer,
    )
    .unwrap();
    assert!(matches!(big, V::Wide(_)));
    let small = exact_binary(
        CoreOp::Sub,
        &big,
        &exact_i128(1, Domain::Integer),
        Domain::Integer,
    )
    .unwrap();
    assert!(matches!(small,V::Exact(n) if n.n==i128::MAX));
    let odd = wide::ratio_value(
        BigRational::from_integer((BigInt::from(1u8) << 160usize) + 1u8),
        Domain::Natural,
    )
    .unwrap();
    let ans = exact_binary(
        CoreOp::Pow,
        &exact_i128(-1, Domain::Integer),
        &odd,
        Domain::Integer,
    )
    .unwrap();
    assert!(matches!(ans,V::Exact(n) if n.n == -1));
}

#[test]
fn a_wide_range_obeys_a_distinct_aggregate_resource_limit() {
    let lo = BigInt::from(1u8) << 700_000usize;
    let hi = &lo + 999u16;
    let lo = wide::ratio_value(BigRational::from_integer(lo), Domain::Natural).unwrap();
    let hi = wide::ratio_value(BigRational::from_integer(hi), Domain::Natural).unwrap();
    assert_eq!(
        range(&lo, &hi, Domain::Natural).err(),
        Some("wide range resource limit exceeded (64 MiB of integer data)")
    );
}
#[test]
fn wide_sqrt_need_not_first_round_the_radicand() {
    let enormous = BigRational::from_integer(BigInt::from(10u8).pow(400));
    assert!(wide::to_real(&enormous).is_err());
    assert_eq!(wide::sqrt(&enormous).unwrap(), 1e200);
    let tiniest = wide::from_real(f64::from_bits(1)).unwrap();
    assert_eq!(wide::sqrt(&tiniest).unwrap(), f64::from_bits(1).sqrt());
}
#[test]
fn exact_wide_sd_does_not_lose_the_small_variance_of_a_large_offset() {
    let base = BigInt::from(1u8) << 2048usize;
    let near = &base + 2u8;
    let xs = [
        wide::ratio_value(BigRational::from_integer(base), Domain::Integer).unwrap(),
        wide::ratio_value(BigRational::from_integer(near), Domain::Integer).unwrap(),
    ];
    assert!(matches!(wide_standard_deviation(&xs).unwrap(),V::Real(r) if r==1.0));
}
