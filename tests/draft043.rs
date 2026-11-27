use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        loop {
            let root = std::env::temp_dir().join(format!(
                "flux-draft043-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            match fs::create_dir(&root) {
                Ok(()) => return Self(root),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("create fixture directory: {e}"),
            }
        }
    }

    fn write(&self, path: &str, data: &str) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).expect("module directory");
        fs::write(path, data).expect("write module fixture");
    }

    fn cli(&self, from: &str, command: &str, root: impl AsRef<Path>) -> Output {
        let mut app = Command::new(env!("CARGO_BIN_EXE_flux"));
        app.current_dir(self.0.join(from));
        if !command.is_empty() {
            app.arg(command);
        }
        app.arg(root.as_ref()).output().expect("invoke Flux CLI")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn accept(f: &Fixture, from: &str, path: &str, expected: &str) {
    for cmd in ["check", "hir", "run-hir", ""] {
        let out = f.cli(from, cmd, path);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{cmd} {path}: {stderr}");
        match cmd {
            "check" => assert!(stdout.is_empty(), "check executed effects"),
            "hir" => assert!(stdout.contains("HirProgram"), "missing typed HIR"),
            _ => assert_eq!(stdout, expected, "{cmd} {path}"),
        }
    }
}

fn reject(f: &Fixture, path: &str, fragments: &[&str]) -> String {
    let mut primary = String::new();
    for cmd in ["check", "run-hir", ""] {
        let out = f.cli("", cmd, path);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !out.status.success() && out.stdout.is_empty(),
            "{cmd}: {out:?}"
        );
        assert!(
            fragments.iter().all(|part| stderr.contains(part)),
            "{cmd}: expected {fragments:?}, got {stderr}",
        );
        if cmd == "check" {
            primary = stderr.into_owned();
        }
    }
    primary
}

const FINANCE: &str = r#"
>> (net, margin);
net(revenue,cost:q):q := - revenue cost;
margin(revenue,cost:q):?q := ?{
    = revenue 0 : ?0;
    _ : ?1 (/ (net revenue cost) revenue);
};
private(x:q):q := * x 2q;
"#;

#[test]
fn finance_workflow_uses_relative_interfaces_not_the_working_directory() {
    let f = Fixture::new();
    f.write("app/finance.flx", FINANCE);
    f.write("finance.flx", "this is deliberately not Flux");
    f.write(
        "app/root.flx",
        r#"
<< "finance.flx" : fin;
<< "io";
profit := fin.net 12000 8500;
ratio := fin.margin 12000 8500;
!{ ^{ ^? ! io.print (profit,ratio,fin.margin 0 8500); () } };
"#,
    );
    accept(&f, "", "app/root.flx", "(3500, ?1 7/24, ?0)\n");
    accept(&f, "app", "root.flx", "(3500, ?1 7/24, ?0)\n");
}

#[test]
fn the_interface_rejects_private_calls_wrong_arity_and_wrong_types() {
    let f = Fixture::new();
    f.write("finance.flx", FINANCE);
    for (source, expected) in [
        ("fin.private 9q", "private"),
        ("fin.net 9q", "operand"),
        ("fin.net \"nine\" 2q", "unify"),
    ] {
        f.write(
            "root.flx",
            &format!("<< \"finance.flx\" : fin;\nx := {source};\n!{{ () }};"),
        );
        let diag = reject(&f, "root.flx", &[expected]);
        assert!(diag.contains("root.flx:2:"), "{diag}");
    }
}

#[test]
fn canonical_diamond_instantiates_the_dependency_once() {
    let f = Fixture::new();
    f.write("shared/base.flx", ">> (v);\nv:z := + 424242z 1z;\n");
    f.write(
        "a/left.flx",
        "<< \"../shared/base.flx\" : d;\n>> (v);\nv:z := d.v;",
    );
    f.write(
        "b/right.flx",
        "<< \"../shared/./base.flx\" : d;\n>> (v);\nv:z := d.v;",
    );
    f.write(
        "root.flx",
        r#"
<< "a/left.flx" : left;
<< "b/right.flx" : right;
<< "io";
!{ ^{ ^? ! io.print (left.v,right.v); () } };
"#,
    );
    accept(&f, "", "root.flx", "(424243, 424243)\n");
    let loaded = flux_lang::check_path(f.0.join("root.flx")).expect("checked diamond");
    let interfaces =
        flux_lang::semantic::check_module_interfaces(&loaded.program, &loaded.module_inputs)
            .expect("checked module interfaces");
    assert_eq!(interfaces.len(), 4, "shared dependency was duplicated");
    let from = |name: &str| {
        interfaces
            .iter()
            .find(|interface| interface.path.file_name().is_some_and(|p| p == name))
            .expect("named canonical interface")
    };
    let base = from("base.flx");
    for dependent in ["left.flx", "right.flx"] {
        assert_eq!(
            from(dependent).dependencies.as_slice(),
            std::slice::from_ref(&base.path)
        );
    }
    assert_eq!(base.exports.len(), 1);
    assert_eq!(base.exports[0].member.name, "v");
    let ir = f.cli("", "hir", "root.flx");
    let ir = String::from_utf8_lossy(&ir.stdout);
    assert_eq!(
        ir.matches("424242").count(),
        1,
        "shared library declaration lowered more than once",
    );
}

#[test]
fn explicit_generic_functions_are_reinstantiated_and_function_values_keep_their_kind() {
    let f = Fixture::new();
    f.write(
        "identity.flx",
        r#"
>> (id, square, twice);
id(x:'a):'a := x;
square(x:'a):'a | num 'a := * x x;
twice:z->z := \x -> * x 2z;
"#,
    );
    f.write(
        "root.flx",
        r#"
<< "identity.flx" : m;
<< "io";
!{ ^{ ^? ! io.print (m.id 3z,m.id "s",m.square 0.5q,@ m.twice 5z); () } };
"#,
    );
    accept(&f, "", "root.flx", "(3, s, 1/4, 10)\n");
    f.write(
        "root.flx",
        "<< \"identity.flx\" : m;\nx := m.twice 5z;\n!{ () };",
    );
    reject(&f, "root.flx", &["root.flx:2:"]);
}

#[test]
fn ambiguous_aliases_and_inferred_value_exports_are_rejected() {
    let f = Fixture::new();
    f.write("one.flx", ">> (v);\nv:z := 1z;");
    f.write("two.flx", ">> (v);\nv:z := 2z;");
    f.write(
        "root.flx",
        "<< \"one.flx\" : m;\n<< \"two.flx\" : m;\n!{ () };",
    );
    reject(&f, "root.flx", &["alias", "m"]);

    f.write("one.flx", ">> (v);\nv := 1z;");
    f.write("root.flx", "<< \"one.flx\" : m;\n!{ () };");
    reject(&f, "root.flx", &["one.flx", "v"]);
}

#[test]
fn imported_nominal_records_have_module_identity() {
    let f = Fixture::new();
    let library = r#"
>> (row, make, get);
row :: { id:z; };
make(x:z):row := row { id := x; };
get(x:row):z := .id x;
"#;
    f.write("a.flx", library);
    f.write("b.flx", library);
    let imports = "<< \"a.flx\" : a;\n<< \"b.flx\" : b;\n<< \"io\";\n";
    f.write(
        "root.flx",
        &format!("{imports}x:a.row := a.make 3z; !{{ ^{{ ^? ! io.print a.get x; () }} }};"),
    );
    accept(&f, "", "root.flx", "3\n");
    f.write(
        "root.flx",
        &format!("{imports}x := a.get (b.make 3z); !{{ () }};"),
    );
    reject(&f, "root.flx", &["root.flx:4:"]);
}

#[test]
fn qualified_sum_constructors_and_recovery_use_the_exported_identity() {
    let f = Fixture::new();
    let library = r#"
>> (failure, fail);
failure :: { | message s; | spare; };
fail(x:s):s^failure := ^{ ^: message x };
"#;
    f.write("a.flx", library);
    f.write("b.flx", library);
    let imports = "<< \"a.flx\" : a;\n<< \"b.flx\" : b;\n<< \"io\";\n";
    f.write(
        "root.flx",
        &format!(
            r#"{imports}
raise(x:s):s^a.failure := ^{{ ^: a.message x }};
!{{
    ^{{ ^? ! io.print (^= a.fail "alpha" {{ a.message m : m; a.spare : "unused"; }},^= raise "beta" {{ a.message m : m; a.spare : "unused"; }}); () }}
}};"#
        ),
    );
    accept(&f, "", "root.flx", "(alpha, beta)\n");
    f.write(
        "root.flx",
        &format!("{imports}v := ^= a.fail \"alpha\" {{ b.message msg : msg; _ : \"unused\"; }}; !{{ () }};"),
    );
    reject(&f, "root.flx", &["root.flx:4:"]);
}

#[test]
fn only_the_root_may_have_an_entry_and_checks_execute_nothing() {
    let f = Fixture::new();
    f.write(
        "bad.flx",
        "<< \"io\";\n!{ ^{ ^? ! io.write \"should-not-exist.txt\" \"bad\"; () } };",
    );
    f.write("root.flx", "<< \"bad.flx\" : bad;\n!{ () };");
    reject(&f, "root.flx", &["bad.flx", "entry"]);
    assert!(!f.0.join("should-not-exist.txt").exists());

    f.write(
        "good.flx",
        "<< \"io\";\n>> (pending);\npending:!(()^ioerr) := io.write \"should-not-exist.txt\" \"bad\";",
    );
    f.write("root.flx", "<< \"good.flx\" : good;\n!{ () };");
    accept(&f, "", "root.flx", "");
    assert!(!f.0.join("should-not-exist.txt").exists());
}

#[test]
fn import_cycles_include_every_participating_file_and_an_import_location() {
    let f = Fixture::new();
    f.write("root.flx", "<< \"a.flx\" : a;\n!{ () };");
    f.write("a.flx", "<< \"dir/b.flx\" : b;");
    f.write("dir/b.flx", "<< \".././a.flx\" : a;");
    let diag = reject(&f, "root.flx", &["cycle", "a.flx", "b.flx"]);
    assert!(diag.contains(".flx:1:"), "{diag}");
}

#[test]
fn imported_parse_type_and_runtime_errors_report_the_library_file() {
    let f = Fixture::new();
    f.write("root.flx", "<< \"lib.flx\" : lib;\n!{ () };");
    f.write("lib.flx", ">> (bad);\nbad(x:z):z := + x;");
    reject(&f, "root.flx", &["lib.flx:2:", "operand"]);
    f.write("lib.flx", ">> (bad);\nbad(x:z):z := \"mistake\";");
    reject(&f, "root.flx", &["lib.flx:2:"]);

    f.write("lib.flx", ">> (bad);\nbad(x:q):q := / 1q x;");
    f.write(
        "root.flx",
        "<< \"lib.flx\" : lib;\n<< \"io\";\n!{ ^{ ^? ! io.print lib.bad 0q; () } };",
    );
    assert!(f.cli("", "check", "root.flx").status.success());
    let run = f.cli("", "run-hir", "root.flx");
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(!run.status.success() && run.stdout.is_empty());
    assert!(
        stderr.contains("lib.flx:2:") && stderr.contains("division"),
        "{stderr}"
    );
}

#[test]
fn remote_absolute_and_dynamic_source_imports_are_not_supported() {
    let f = Fixture::new();
    for source in [
        "<< \"https://example.invalid/remote.flx\" : m; !{ () };",
        "<< \"file:///sample.flx\" : m; !{ () };",
        "<< \"/absolute.flx\" : m; !{ () };",
        "<< \"C:/absolute.flx\" : m; !{ () };",
        "path := \"library.flx\"; << path : m; !{ () };",
    ] {
        f.write("root.flx", source);
        reject(&f, "root.flx", &["root.flx:1:"]);
    }
}

#[test]
fn imports_do_not_reexport_private_or_transitive_names() {
    let f = Fixture::new();
    f.write("a.flx", ">> (v);\nv:z := 1z;");
    f.write("b.flx", "<< \"a.flx\" : a;\n>> (w);\nw:z := a.v;");
    for visible in ["a.v", "b.v"] {
        f.write(
            "root.flx",
            &format!("<< \"b.flx\" : b;\nx := {visible};\n!{{ () }};"),
        );
        reject(&f, "root.flx", &["root.flx:2:", visible]);
    }
}

#[test]
fn default_module_namespace_and_public_path_api_share_the_same_loader() {
    let f = Fixture::new();
    f.write("app/finance.flx", FINANCE);
    f.write(
        "app/edit.flx",
        "<< \"finance.flx\"; << \"io\"; !{ ^{ ^? ! io.print finance.net 12q 8q; () } };",
    );
    accept(&f, "", "app/edit.flx", "4\n");
    let path = f.0.join("app/edit.flx");
    assert!(flux_lang::check_path(&path).is_ok());
    let output = flux_lang::run_path(&path).expect("public typed path run");
    assert_eq!(output.stdout, "4\n");
}

#[test]
fn an_uncaught_imported_failure_retains_origin_and_root_propagation_note() {
    let f = Fixture::new();
    f.write(
        "lib.flx",
        ">> (failure, fail);\nfailure :: { | missing; };\nfail(x:()):()^failure := ^{ ^: missing };",
    );
    f.write(
        "root.flx",
        "<< \"lib.flx\" : lib;\n!{ ^{ ^? lib.fail (); () } };",
    );
    assert!(f.cli("", "check", "root.flx").status.success());
    let out = f.cli("", "run-hir", "root.flx");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1));
    for piece in [
        "lib.flx:3:",
        "failure",
        "missing",
        "root.flx:2:",
        "propagated here",
    ] {
        assert!(stderr.contains(piece), "{piece}: {stderr}");
    }
}

#[test]
fn multiple_aliases_of_a_canonical_file_share_one_nominal_identity() {
    let f = Fixture::new();
    f.write(
        "row.flx",
        ">> (row, make, get);\nrow :: { id:z; };\nmake(x:z):row := row { id := x; };\nget(x:row):z := .id x;",
    );
    f.write(
        "root.flx",
        r#"
<< "row.flx" : left;
<< "./row.flx" : right;
<< "io";
!{ ^{ ^? ! io.print left.get (right.make 9z); () } };
"#,
    );
    accept(&f, "", "root.flx", "9\n");
}

#[test]
fn private_nominal_types_cannot_leak_through_public_interfaces() {
    let f = Fixture::new();
    f.write("root.flx", "<< \"lib.flx\" : lib;\n!{ () };");
    for source in [
        ">> (make);\nhidden :: { id:z; };\nmake(x:z):hidden := hidden { id := x; };",
        ">> (public);\nhidden :: { id:z; };\npublic :: { field:hidden; };",
        ">> (failure);\nhidden :: { id:z; };\nfailure :: { | one hidden; };",
    ] {
        f.write("lib.flx", source);
        reject(&f, "root.flx", &["lib.flx:3:", "private nominal", "hidden"]);
    }
}

#[test]
fn public_nominal_from_a_dependency_can_appear_in_another_interface() {
    let f = Fixture::new();
    f.write(
        "row.flx",
        ">> (row, make);\nrow :: { id:z; };\nmake(x:z):row := row { id := x; };",
    );
    f.write(
        "ops.flx",
        "<< \"row.flx\" : r;\n>> (get);\nget(x:r.row):z := .id x;",
    );
    f.write(
        "root.flx",
        r#"
<< "row.flx" : r;
<< "ops.flx" : ops;
<< "io";
!{ ^{ ^? ! io.print ops.get (r.make 17z); () } };
"#,
    );
    accept(&f, "", "root.flx", "17\n");
}
