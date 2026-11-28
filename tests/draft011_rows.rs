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
            let dir = std::env::temp_dir().join(format!(
                "flux-rows-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&dir) {
                Ok(()) => {
                    fs::write(dir.join("main.flx"), source).expect("main fixture");
                    return Self(dir);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("fixture directory: {e}"),
            }
        }
    }

    fn write(&self, filename: &str, source: &str) {
        fs::write(self.0.join(filename), source).expect("module fixture");
    }

    fn cli(&self, mode: &str) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_flux"));
        command.current_dir(&self.0);
        if !mode.is_empty() {
            command.arg(mode);
        }
        command.arg("main.flx").output().expect("execute Flux")
    }

    fn accepts(&self, expected: &str) {
        for mode in ["check", "hir", "run-hir", ""] {
            let out = self.cli(mode);
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(
                out.status.success() && stderr.is_empty(),
                "{mode}: {stderr}"
            );
            match mode {
                "check" => assert!(stdout.is_empty(), "{stdout}"),
                "hir" => assert!(stdout.contains("HirProgram"), "{stdout}"),
                _ => assert_eq!(stdout, expected, "{mode}"),
            }
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn rejects(source: &str, words: &[&str]) {
    let out = Fixture::new(source).cli("check");
    let error = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success() && out.stdout.is_empty(), "{source}");
    assert!(
        words.iter().all(|word| error.contains(word)),
        "expected {words:?}: {error}"
    );
}

const KEEP: &str = r#"
keep(t:#.{ value:q; | ~r }):#.{ value:q; | ~r } := ?: (> .value t 0q) t;
"#;

#[test]
fn one_open_record_signature_projects_from_different_closed_rows() {
    Fixture::new(
        r#"
score(x:.{ value:q; | ~r }):q := .value x;
a := .{ label:="west"; value:=1.25q; };
b := .{ value:=2.5q; flag:=1b; code:=7n; };
!{ ^{ ^? ! print (score a,@ &score b); () } };
"#,
    )
    .accepts("(5/4, 5/2)\n");
}

#[test]
fn one_filter_preserves_distinct_closed_table_schemas_in_direct_and_first_class_calls() {
    Fixture::new(&format!(
        "{KEEP}{}",
        r#"
a:#.{ id:s; value:q; } := #[.{ id:="ok";value:=2q; } .{ id:="negative";value:=-1q; }];
b:#.{ value:q; flag:b; } := #[.{ value:=3q;flag:=1b; } .{ value:=0q;flag:=0b; }];
!{ ^{
    ^? ! print (.id (keep a),.flag (@ &keep b));
    ()
} };
"#
    ))
    .accepts("([ok], [1b])\n");
}

#[test]
fn empty_filtered_table_keeps_all_column_names_in_checked_order() {
    let fixture = Fixture::new(&format!(
        "<< \"csv\";\n{KEEP}{}",
        r#"
problem :: { | file csverr; | display ioerr; };
empty:#.{ label:s; value:q; flag:b; } := #[];
!{ ^{
    out := @ &keep empty;
    ^= ! csv.write "empty.csv" out { e : ^: file e; };
    ^= ! print (# out) { e : ^: display e; };
    ()
} };
"#
    ));
    let filename = fixture.0.join("empty.csv");
    for mode in ["check", "hir", "run-hir", ""] {
        let out = fixture.cli(mode);
        assert!(
            out.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        match mode {
            "check" => assert!(!filename.exists() && out.stdout.is_empty()),
            "hir" => assert!(
                !filename.exists() && String::from_utf8_lossy(&out.stdout).contains("HirProgram")
            ),
            _ => {
                assert_eq!(out.stdout, b"0\n");
                assert_eq!(
                    fs::read_to_string(&filename).unwrap().replace("\r\n", "\n"),
                    "label,value,flag\n"
                );
                fs::remove_file(&filename).unwrap();
            }
        }
    }
}

#[test]
fn explicit_structural_view_retains_named_table_order_values_and_empty_schema() {
    let fixture = Fixture::new(&format!(
        "<< \"df\";\n<< \"csv\";\n{KEEP}{}",
        r#"
reading :: { label:s; value:q; flag:b; };
problem :: { | file csverr; | display ioerr; };
data:#reading := #[reading { label:="r";value:=2q;flag:=1b; }];
empty:#reading := #[];
!{ ^{
    result := keep (df.structural data);
    ^= ! csv.write "view.csv" (keep (df.structural empty)) { e : ^: file e; };
    ^= ! print (.label result,.value result,.flag result) { e : ^: display e; };
    ()
} };
"#
    ));
    let filename = fixture.0.join("view.csv");
    for mode in ["check", "hir", "run-hir", ""] {
        let out = fixture.cli(mode);
        assert!(
            out.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        if mode == "check" || mode == "hir" {
            assert!(!filename.exists(), "{mode} created a file");
        } else {
            assert_eq!(out.stdout, b"([r], [2], [1b])\n");
            assert_eq!(
                fs::read_to_string(&filename).unwrap().replace("\r\n", "\n"),
                "label,value,flag\n"
            );
            fs::remove_file(&filename).unwrap();
        }
    }
}

#[test]
fn qualified_open_row_interface_reinstantiates_for_each_caller() {
    let fixture = Fixture::new(
        r#"
<< "filters.flx" : f;
a:#.{ label:s; value:q; } := #[.{label:="x";value:=1q;}];
b:#.{ value:q; status:b; } := #[.{value:=2q;status:=0b;}];
!{ ^{ ^? ! print (.label (f.keep a),.status (@ &f.keep b)); () } };
"#,
    );
    fixture.write("filters.flx", &format!(">> (keep);\n{KEEP}"));
    fixture.accepts("([x], [0b])\n");
}

#[test]
fn absent_required_fields_and_conflicting_field_domains_are_rejected() {
    let declaration = "score(x:.{ value:q; | ~r }):q := .value x;";
    rejects(
        &format!("{declaration} y := score .{{ label:=\"missing\"; }};"),
        &["value"],
    );
    rejects(
        &format!("{declaration} y := score .{{ value:=\"wrong\"; }};"),
        &["value"],
    );
}

#[test]
fn duplicate_labels_mixed_row_kinds_and_dropped_rigid_tails_are_rejected() {
    rejects(
        "duplicate(x:.{ value:q; value:q; | ~r }):q := .value x;",
        &["duplicate", "value"],
    );
    rejects(
        "bad(x:.{ value:~r; | ~r }):~r := .value x;",
        &["row variable", "ordinary type"],
    );
    rejects(
        "drop(x:.{ value:q; | ~r }):.{ value:q; | ~r } := .{ value:=.value x; };",
        &["row"],
    );
}

#[test]
fn named_table_is_not_implicitly_coerced_to_a_structural_table() {
    rejects(
        &format!(
            "{KEEP}{}",
            r#"
reading :: { value:q; };
data:#reading := #[];
plain := keep data;
"#
        ),
        &["reading"],
    );
}

#[test]
fn open_key_fields_suffice_for_join_and_group_over_different_row_shapes() {
    Fixture::new(
        r#"
<< "df";
overlap(l:#.{ key:s; | ~l },r:#.{ key:s; | ~r }):n :=
    # (*= &.key &.key l r);
count(k:s,g:#.{ key:s; | ~r }):.{ key:s; size:n; } :=
    .{ key:=k; size:=# g; };
buckets(t:#.{ key:s; | ~r }):#.{ key:s; size:n; } :=
    df.agg &.key &count t;
left:#.{ key:s; label:s; } := #[.{key:="a";label:="present";}];
right:#.{ active:b; key:s; extra:n; } := #[.{active:=1b;key:="a";extra:=2n;}];
empty:#.{ extra:n; key:s; } := #[];
!{ ^{
    ^? ! print (overlap left right,overlap left empty,.size (buckets right),.size (buckets empty));
    ()
} };
"#,
    )
    .accepts("(1, 0, [1], [])\n");
}

#[test]
fn open_column_records_cannot_hide_nonsequence_fields_from_reconstruction() {
    rejects(
        r#"
cols(x:.{ value:[q]; | ~r }):#.{ value:q; } := #: x;
bad := cols .{ value := [1q 2q]; extra := "not a sequence"; };
"#,
        &["closed schema"],
    );
}

#[test]
fn roundtrip_uses_the_selected_inputs_order_not_another_shared_row_witness() {
    let fixture = Fixture::new(
        r#"
<< "csv";
second(a:#.{ value:q; | ~r },b:#.{ value:q; | ~r }):#.{ value:q; | ~r } := #: :# b;
l:#.{ tag:s; value:q; } := #[];
r:#.{ value:q; tag:s; } := #[];
!{ ^{ ^? ! csv.write "ordered.csv" (second l r); () } };
"#,
    );
    let out = fixture.cli("run-hir");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        fs::read_to_string(fixture.0.join("ordered.csv"))
            .unwrap()
            .replace("\r\n", "\n"),
        "value,tag\n"
    );
}
