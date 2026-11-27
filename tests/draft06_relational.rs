use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new(source: &str) -> Self {
        loop {
            let path = std::env::temp_dir().join(format!(
                "flux-relational-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => {
                    fs::write(path.join("test.flx"), source).expect("write fixture");
                    return Self(path);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("create fixture: {e}"),
            }
        }
    }

    fn run(&self, mode: Option<&str>) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_flux"));
        child.current_dir(&self.0);
        if let Some(mode) = mode {
            child.arg(mode);
        }
        child.arg("test.flx").output().expect("invoke Flux")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn check(f: &Fixture) {
    let out = f.run(Some("check"));
    assert!(
        out.status.success() && out.stdout.is_empty(),
        "check: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = f.run(Some("hir"));
    assert!(
        out.status.success() && String::from_utf8_lossy(&out.stdout).contains("HirProgram"),
        "hir: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn strict(source: &str, expected: &str) {
    let f = Fixture::new(source);
    check(&f);
    for mode in [Some("run-hir"), None] {
        let out = f.run(mode);
        assert!(
            out.status.success() && out.stderr.is_empty(),
            "{mode:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&out.stdout), expected, "{mode:?}");
    }
}

const JOIN_FIXTURE: &str = r#"
<< "df";
<< "opt";
lrow :: { k:z; tag:s; };
rrow :: { k:z; tag:s; };
left:#lrow := #[
    lrow { k := 1; tag := "a"; }
    lrow { k := 2; tag := "u"; }
    lrow { k := 1; tag := "b"; }
    lrow { k := 3; tag := "c"; }
];
right:#rrow := #[
    rrow { k := 1; tag := "x"; }
    rrow { k := 3; tag := "z"; }
    rrow { k := 1; tag := "y"; }
];
pair(x:(lrow,rrow)):(s,s) := (.tag .1 x,.tag .2 x);
name(x:rrow):s := .tag x;
maybe(x:(lrow,?rrow)):(s,s) := (.tag .1 x,?? "none" (opt.map &name .2 x));
"#;

#[test]
fn joins_keep_left_then_right_order_and_duplicate_matches() {
    strict(
        &format!(
            r#"{JOIN_FIXTURE}
!{{
    ^{{
        matches := *= &.k &.k left right;
        outer := *? &.k &.k left right;
        ^? ! print :# (df.select &pair matches);
        ^? ! print :# (df.select &maybe outer);
        ^? ! print .tag (df.semi &.k &.k left right);
        ^? ! print .tag (df.anti &.k &.k left right);
        ()
    }}
}};"#
        ),
        "[(a, x) (a, y) (b, x) (b, y) (c, z)]\n[(a, x) (a, y) (u, none) (b, x) (b, y) (c, z)]\n[a b c]\n[u]\n",
    );
}

#[test]
fn empty_join_results_have_checked_pair_and_optional_row_shapes() {
    strict(
        &format!(
            r#"{JOIN_FIXTURE}
emptyl:#lrow := #[];
emptyr:#rrow := #[];
inn:#(lrow,rrow) := *= &.k &.k left emptyr;
ll:#(lrow,?rrow) := *? &.k &.k left emptyr;
rr:#(lrow,?rrow) := *? &.k &.k emptyl right;
!{{
    ^{{
        ^? ! print (# inn,# ll,# rr);
        ^? ! print :# (df.select &maybe ll);
        ^? ! print .tag (df.semi &.k &.k left emptyr);
        ^? ! print .tag (df.anti &.k &.k left emptyr);
        ()
    }}
}};"#
        ),
        "(0, 4, 0)\n[(a, none) (u, none) (b, none) (c, none)]\n[]\n[a u b c]\n",
    );
}

#[test]
fn compound_optional_decimal_keys_join_group_and_aggregate_by_value() {
    strict(
        r#"
<< "df";
reading :: { bucket:s; value:?d; id:s; };
a:#reading := #[
    reading { bucket := "one"; value := ?0; id := "missing"; }
    reading { bucket := "one"; value := ?1 0.00d; id := "zero"; }
    reading { bucket := "one"; value := ?1 1.20d; id := "one-a"; }
    reading { bucket := "one"; value := ?1 1.2d; id := "one-b"; }
    reading { bucket := "two"; value := ?1 1.20d; id := "two"; }
];
b:#reading := #[
    reading { bucket := "one"; value := ?1 1.200d; id := "a"; }
    reading { bucket := "one"; value := ?0; id := "b"; }
    reading { bucket := "one"; value := ?1 0d; id := "c"; }
];
key(x:reading):(?d,s) := (.value x,.bucket x);
ids(x:(reading,reading)):(s,s) := (.id .1 x,.id .2 x);
summary(k:(?d,s),rows:#reading) := .{ key := k; count := # rows; };
!{ ^{
    ^? ! print :# (df.select &ids (*= &key &key a b));
    ^? ! print .count (df.agg &key &summary a);
    groups := =: &key a;
    ^? ! print .id (?! (.: ((?1 1.2000d),"one") groups));
    ^? ! print .id (df.dropna &.value a);
    ^? ! print (df.fill 9.90d (.value a),df.present (.value a));
    ()
} };
"#,
        "[(missing, b) (zero, c) (one-a, a) (one-b, a)]\n[1 1 2 1]\n[one-a one-b]\n[zero one-a one-b two]\n([9.90 0.00 1.20 1.2 1.20], [0.00 1.20 1.2 1.20])\n",
    );
}

#[test]
fn distinct_and_union_keep_first_occurrences_while_concat_keeps_all_rows() {
    strict(
        r#"
<< "df";
row :: { label:s; amount:d; };
left:#row := #[
    row { label := "a"; amount := 1.20d; }
    row { label := "b"; amount := 2.30d; }
    row { label := "a"; amount := 1.2d; }
];
right:#row := #[
    row { label := "b"; amount := 2.3d; }
    row { label := "c"; amount := 3.400d; }
    row { label := "c"; amount := 3.4d; }
];
!{ ^{
    ^? ! print (.label (df.distinct left),.amount (df.distinct left));
    ^? ! print (.label (df.union left right),.amount (df.union left right));
    ^? ! print (# (+: left right));
    ()
} };
"#,
        "([a b], [1.20 2.30])\n([a b c], [1.20 2.30 3.400])\n6\n",
    );
}

#[test]
fn projection_uses_checked_empty_schema_and_does_not_call_the_projector() {
    let f = Fixture::new(
        r#"
<< "df";
<< "csv";
row :: { price:d; label:s; };
out :: { label:s; amount:d; };
empty:#row := #[];
bomb(x:row):out := out { label := .label x; amount := :d (/ 1q 0q); };
!{ ^{
    flat:#out := df.select &bomb empty;
    ^? ! csv.write "out.csv" flat;
    ()
} };
"#,
    );
    check(&f);
    assert!(!f.0.join("out.csv").exists());
    let out = f.run(Some("run-hir"));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let written = fs::read_to_string(f.0.join("out.csv")).unwrap();
    assert_eq!(written.replace("\r\n", "\n"), "label,amount\n");
}

#[test]
fn joins_do_not_call_keys_on_empty_rows_and_have_deterministic_failure_order() {
    strict(
        r#"
<< "df";
row :: { k:z; };
empty:#row := #[];
bomb(x:row):z := // 1z 0z;
!{ ^{ ^? ! print (# (*= &bomb &bomb empty empty),# (*? &bomb &bomb empty empty)); () } };
"#,
        "(0, 0)\n",
    );

    let f = Fixture::new(
        r#"
row :: { k:z; };
rows:#row := #[row { k := 1z; }];
none:?z := ?0;
left(x:row):z := ?! none;
right(x:row):z := // 1z 0z;
!{ ^{ ^? ! print (*= &left &right rows rows); () } };
"#,
    );
    check(&f);
    let out = f.run(Some("run-hir"));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success() && out.stdout.is_empty() && stderr.contains("division by zero"),
        "{stderr}"
    );
}

#[test]
fn relational_types_reject_non_rows_bad_keys_and_incompatible_nominals() {
    let common = r#"
<< "df";
reading :: { id:z; };
srow :: { id:z; };
a:#reading := #[reading { id := 1z; }];
b:#srow := #[srow { id := 1z; }];
text(x:reading):s := "a";
callable(x:reading):z->z := \n -> n;
number(x:reading):z := .id x;
"#;
    for expr in [
        "*= &text &.id a b",
        "*? &callable &callable a a",
        "df.select &number a",
        "df.dropna &number a",
        "df.union a b",
        "df.fill 1z [1z]",
    ] {
        let f = Fixture::new(&format!("{common}\nx := {expr}; !{{ () }};"));
        let out = f.run(Some("check"));
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !out.status.success() && stderr.contains("E0200") && !stderr.contains("E0400"),
            "{expr}: {stderr}"
        );
    }
}

#[test]
fn join_output_limit_distinguishes_materialization_from_input_rows() {
    let f = Fixture::new(
        r#"
row :: { value:n; };
mk(x:n):row := row { value := x; };
one(x:row):n := 1n;
a:#row := #: (@: &mk (.. 1n 317n));
!{ ^{ ^? ! print (# (*= &one &one a a)); () } };
"#,
    );
    check(&f);
    let out = f.run(Some("run-hir"));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success() && out.stdout.is_empty() && stderr.contains("limit"),
        "{stderr}"
    );
}

#[test]
fn signed_real_zero_is_one_compound_key() {
    strict(
        r#"
row :: { value:r; id:s; };
a:#row := #[
    row { value := -0.0r; id := "minus"; }
    row { value := 0.0r; id := "plus"; }
];
b:#row := #[row { value := 0.0r; id := "right"; }];
key(x:row):(r,s) := (.value x,"bucket");
!{ ^{
    ^? ! print (# (*= &key &key a b));
    groups := =: &key a;
    ^? ! print (# groups,.id (?! (.: (-0.0r,"bucket") groups)));
    ()
} };
"#,
        "2\n(1, [minus plus])\n",
    );
}

#[test]
fn calibration_audit_distinguishes_unmatched_missing_duplicate_and_zero() {
    macro_rules! calibration {
        ($name:literal) => {
            include_str!(concat!("../examples/applications/calibration/", $name))
        };
    }
    for mode in [Some("run-hir"), None] {
        let f = Fixture::new(calibration!("audit.flx"));
        for (name, contents) in [
            ("schema.flx", calibration!("schema.flx")),
            ("matching.flx", calibration!("matching.flx")),
            ("findings.flx", calibration!("findings.flx")),
            ("measured.csv", calibration!("measured.csv")),
            ("reference.csv", calibration!("reference.csv")),
        ] {
            fs::write(f.0.join(name), contents).unwrap();
        }
        check(&f);
        let findings = f.0.join("findings.csv");
        assert!(!findings.exists());
        let out = f.run(mode);
        assert!(
            out.status.success() && out.stderr.is_empty(),
            "{mode:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&out.stdout), "(8, 9, 5)\n");
        let report = fs::read(&findings).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&report).replace("\r\n", "\n"),
            calibration!("expected.csv").replace("\r\n", "\n"),
        );

        let rerun = f.run(mode);
        assert_eq!(rerun.status.code(), Some(1));
        assert!(rerun.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&rerun.stderr).contains("output fileerr exists findings.csv")
        );
        assert_eq!(fs::read(&findings).unwrap(), report);
    }
}

#[test]
fn union_accepts_same_structural_fields_in_different_orders() {
    strict(
        r#"
<< "df";
a:#.{ x:z; y:s; } := #[.{ x := 1z; y := "a"; }];
b:#.{ y:s; x:z; } := #[.{ y := "a"; x := 1z; } .{ y := "b"; x := 2z; }];
!{ ^{
    merged := df.union a b;
    ^? ! print (# merged,.x merged,.y merged);
    ()
} };
"#,
        "(2, [1 2], [a b])\n",
    );
}

#[test]
fn generic_projection_instantiates_output_schema_even_when_empty() {
    let f = Fixture::new(
        r#"
<< "df";
<< "csv";
wrap(x:('a,s)):.{ value:'a; } := .{ value := .1 x; };
project(t:#('a,s)):#.{ value:'a; } := df.select &wrap t;
ints:#(z,s) := #[(1z,"one") (2z,"two")];
moneys:#(d,s) := #[];
!{ ^{
    ^? ! csv.write "nonempty.csv" (project ints);
    ^? ! csv.write "empty.csv" (project moneys);
    ()
} };
"#,
    );
    check(&f);
    let out = f.run(Some("run-hir"));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    for (name, expected) in [("nonempty.csv", "value\n1\n2\n"), ("empty.csv", "value\n")] {
        let written = fs::read_to_string(f.0.join(name)).unwrap();
        assert_eq!(written.replace("\r\n", "\n"), expected, "{name}");
    }
}

#[test]
fn reordered_named_table_fields_are_structural_and_display_in_schema_order() {
    strict(
        r#"
<< "df";
a:#.{id:n; value:s;} := #[.{id:=1n; value:="a";}];
b:#.{value:s; id:n;} := #[.{value:="a"; id:=1n;}];
c:#.{value:s; id:n;} := #[.{value:="b"; id:=2n;}];
rows := #[.{child:=a;} .{child:=b;}];
empty_left:#.{child:#.{id:n;value:s;};} := #[];
empty_right:#.{child:#.{value:s;id:n;};} := #[];
!{ ^{
    ^? ! print (== a b, # (df.distinct rows), == empty_left empty_right);
    ^? ! print (df.union a c);
    ()
} };
"#,
        "(1b, 1, 1b)\n#[{id := 1; value := a} {id := 2; value := b}]\n",
    );
}
