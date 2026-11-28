use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new(source: &str) -> Self {
        loop {
            let dir = std::env::temp_dir().join(format!(
                "flux-match-{}-{}",
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

    fn cli(&self, mode: &str, input: &str) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_flux"));
        command.current_dir(&self.0);
        if !mode.is_empty() {
            command.arg(mode);
        }
        let mut child = command
            .arg("main.flx")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("execute Flux");
        if !input.is_empty() {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(input.as_bytes())
                .unwrap();
        }
        child.wait_with_output().expect("wait for Flux")
    }

    fn accepts(&self, input: &str, expected: &str) {
        for mode in ["check", "hir", "run-hir", ""] {
            let out = self.cli(
                mode,
                if mode == "check" || mode == "hir" {
                    ""
                } else {
                    input
                },
            );
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
    let out = Fixture::new(source).cli("check", "");
    let error = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success() && out.stdout.is_empty(), "{source}");
    assert!(
        words.iter().all(|word| error.contains(word)),
        "expected {words:?}: {error}"
    );
}

const TREE: &str = r#"
tree['a] :: {
    | leaf 'a;
    | node tree['a] tree['a];
};
"#;

#[test]
fn recursive_generic_tree_uses_fresh_constructor_instances_and_evaluates_selected_arms() {
    Fixture::new(&format!(
        "{TREE}{}",
        r#"
total(t:tree[z]):z := ?= t {
    leaf x : x;
    node l r : + total l total r;
};
first(t:tree['a]):'a := ?= t {
    leaf x : x;
    node l _ : first l;
};
number := node (leaf 3z) (node (leaf 4z) (leaf 5z));
word := node (leaf "rain") (leaf "wind");
!{ ^{ ^? ! print (total number,first number,first word); () } };
"#
    ))
    .accepts("", "(12, 3, rain)\n");
}

#[test]
fn nested_records_tuples_optionals_booleans_and_unit_are_exhaustive() {
    Fixture::new(
        r#"
kind(x:.{item:(?z,b,()); label:s;}):s := ?= x {
    .{ item := (?0,_,()); label := text; } : text;
    .{ item := (?1 _,0b,()); label := _; } : "off";
    .{ item := (?1 _,1b,()); label := _; } : "on";
};
!{ ^{
    ^? ! print (
        kind .{item:=(?0,1b,()); label:="missing";},
        kind .{item:=(?1 7z,0b,()); label:="seen";},
        kind .{item:=(?1 8z,1b,()); label:="seen";}
    );
    ()
} };
"#,
    )
    .accepts("", "(missing, off, on)\n");
}

#[test]
fn scrutinee_runs_once_and_unselected_arm_is_not_evaluated() {
    Fixture::new(
        r#"
<< "io";
never:?s := ?0;
!{ ^{
    x := ?= (^? ! io.readln) {
        ?0 : ?! never;
        ?1 text : text;
    };
    y := ^? ! io.readln;
    ^? ! print (x,y);
    ()
} };
"#,
    )
    .accepts("one\ntwo\n", "(one, ?1 two)\n");
}

#[test]
fn reports_a_missing_case_under_nested_constructor_payloads() {
    rejects(
        r#"
maybe['a] :: { | wrap ?'a; };
f(x:maybe[z]):z := ?= x { wrap ?1 y : y; };
"#,
        &["exhaustive"],
    );
}

#[test]
fn reports_arms_covered_by_the_union_of_earlier_nested_patterns() {
    rejects(
        r#"
f(x:(b,?z)):z := ?= x {
    (0b,_) : 0z;
    (1b,?0) : 1z;
    (1b,?1 y) : y;
    (_,_) : 2z;
};
"#,
        &["unreachable"],
    );
}

#[test]
fn reports_duplicate_binders_and_incompatible_arm_or_payload_types() {
    rejects("f(x:(z,z)):z := ?= x { (y,y) : y; };", &["duplicate", "y"]);
    rejects("f(x:b):z := ?= x { 0b : 0z; 1b : \"one\"; };", &["E0200"]);
    rejects(
        "maybe['a] :: { | wrap 'a; }; x:maybe[z] := wrap \"text\";",
        &["E0200"],
    );
    rejects(
        "maybe['a] :: { | wrap 'a; }; f(x:maybe[z]):z := ?= x { wrap 1b : 0z; _ : 1z; };",
        &["E0200"],
    );
}

#[test]
fn imported_generic_constructors_preserve_nominal_identity_across_aliases() {
    let fixture = Fixture::new(
        r#"
<< "tree.flx" : t;
<< "./tree.flx" : same;
head(x:t.tree['a]):'a := ?= x {
    same.leaf v : v;
    same.node l _ : head l;
};
a:t.tree[z] := same.node (same.leaf 3z) (t.leaf 5z);
b:same.tree[s] := t.node (same.leaf "first") (t.leaf "second");
!{ ^{ ^? ! print (head a,head b); () } };
"#,
    );
    fixture.write("tree.flx", &format!(">> (tree);\n{TREE}"));
    fixture.accepts("", "(3, first)\n");

    let fixture = Fixture::new(
        r#"
<< "tree.flx" : t;
<< "other.flx" : o;
bad(x:t.tree[z]):z := ?= x { o.leaf v : v; _ : 0z; };
"#,
    );
    for file in ["tree.flx", "other.flx"] {
        fixture.write(file, &format!(">> (tree);\n{TREE}"));
    }
    let out = fixture.cli("check", "");
    assert!(
        !out.status.success(),
        "a different module's constructor was accepted"
    );
    let error = String::from_utf8_lossy(&out.stderr);
    assert!(
        error.contains("main.flx:") && error.contains("distinct nominal types"),
        "{error}"
    );
}

#[test]
fn coverage_decomposes_one_layer_of_a_recursive_nominal_without_unrolling() {
    Fixture::new(&format!(
        "{TREE}{}",
        r#"
shape(t:tree[b]):s := ?= t {
    leaf 0b : "false leaf";
    leaf 1b : "true leaf";
    node (leaf 0b) _ : "false left";
    node (leaf 1b) _ : "true left";
    node (node _ _) _ : "branch left";
};
!{ ^{
    ^? ! print (shape (leaf 1b),shape (node (leaf 0b) (leaf 1b)),
        shape (node (node (leaf 1b) (leaf 0b)) (leaf 0b)));
    ()
} };
"#
    ))
    .accepts("", "(true leaf, false left, branch left)\n");
}

#[test]
fn exported_generic_application_cannot_hide_a_private_nominal_argument() {
    let fixture = Fixture::new(
        r#"
<< "tree.flx" : t;
!{ () };
"#,
    );
    fixture.write(
        "tree.flx",
        &format!(
            ">> (tree, sample);\n{TREE}\n{}",
            r#"
hidden :: { n:z; };
sample:tree[hidden] := leaf hidden { n:=1z; };
"#
        ),
    );
    let out = fixture.cli("check", "");
    let error = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success() && out.stdout.is_empty(), "{error}");
    assert!(
        error.contains("private nominal") && error.contains("hidden"),
        "{error}"
    );
}
