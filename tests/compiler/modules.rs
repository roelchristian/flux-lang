use crate::support::{flux, TempDir};
use std::{fs, process::Output};

fn write(dir: &TempDir, path: &str, data: &str) {
    let path = dir.join(path);
    fs::create_dir_all(path.parent().unwrap()).expect("module directory");
    fs::write(path, data).expect("write module fixture");
}

fn cli(dir: &TempDir, from: &str, command: &str, root: &str) -> Output {
    let mut app = flux();
    app.current_dir(dir.join(from));
    if !command.is_empty() {
        app.arg(command);
    }
    app.arg(root).output().expect("invoke Flux CLI")
}

fn accept(f: &TempDir, from: &str, path: &str, expected: &str) {
    for cmd in ["check", "hir", "run-hir", ""] {
        let out = cli(f, from, cmd, path);
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

fn reject(f: &TempDir, path: &str, fragments: &[&str]) -> String {
    let mut primary = String::new();
    for cmd in ["check", "run-hir", ""] {
        let out = cli(f, "", cmd, path);
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
    let f = TempDir::new("modules");
    write(&f, "app/finance.flx", FINANCE);
    write(&f, "finance.flx", "this is deliberately not Flux");
    write(
        &f,
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
    let f = TempDir::new("modules");
    write(&f, "finance.flx", FINANCE);
    for (source, expected) in [
        ("fin.private 9q", "private"),
        ("fin.net 9q", "operand"),
        ("fin.net \"nine\" 2q", "unify"),
    ] {
        write(
            &f,
            "root.flx",
            &format!("<< \"finance.flx\" : fin;\nx := {source};\n!{{ () }};"),
        );
        let diag = reject(&f, "root.flx", &[expected]);
        assert!(diag.contains("root.flx:2:"), "{diag}");
    }
}

#[test]
fn canonical_diamond_instantiates_the_dependency_once() {
    let f = TempDir::new("modules");
    write(&f, "shared/base.flx", ">> (v);\nv:z := + 424242z 1z;\n");
    write(
        &f,
        "a/left.flx",
        "<< \"../shared/base.flx\" : d;\n>> (v);\nv:z := d.v;",
    );
    write(
        &f,
        "b/right.flx",
        "<< \"../shared/./base.flx\" : d;\n>> (v);\nv:z := d.v;",
    );
    write(
        &f,
        "root.flx",
        r#"
<< "a/left.flx" : left;
<< "b/right.flx" : right;
<< "io";
!{ ^{ ^? ! io.print (left.v,right.v); () } };
"#,
    );
    accept(&f, "", "root.flx", "(424243, 424243)\n");
    let loaded = flux_lang::check_path(f.join("root.flx")).expect("checked diamond");
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
    let ir = cli(&f, "", "hir", "root.flx");
    let ir = String::from_utf8_lossy(&ir.stdout);
    assert_eq!(
        ir.matches("424242").count(),
        1,
        "shared library declaration lowered more than once",
    );
}

#[test]
fn explicit_generic_functions_are_reinstantiated_and_function_values_keep_their_kind() {
    let f = TempDir::new("modules");
    write(
        &f,
        "identity.flx",
        r#"
>> (id, square, twice);
id(x:~a):~a := x;
square(x:~a):~a | num ~a := * x x;
twice:z->z := \x -> * x 2z;
"#,
    );
    write(
        &f,
        "root.flx",
        r#"
<< "identity.flx" : m;
<< "io";
!{ ^{ ^? ! io.print (m.id 3z,m.id "s",m.square 0.5q,@ m.twice 5z); () } };
"#,
    );
    accept(&f, "", "root.flx", "(3, s, 1/4, 10)\n");
    write(
        &f,
        "root.flx",
        "<< \"identity.flx\" : m;\nx := m.twice 5z;\n!{ () };",
    );
    reject(&f, "root.flx", &["root.flx:2:"]);
}

#[test]
fn ambiguous_aliases_and_inferred_value_exports_are_rejected() {
    let f = TempDir::new("modules");
    write(&f, "one.flx", ">> (v);\nv:z := 1z;");
    write(&f, "two.flx", ">> (v);\nv:z := 2z;");
    write(
        &f,
        "root.flx",
        "<< \"one.flx\" : m;\n<< \"two.flx\" : m;\n!{ () };",
    );
    reject(&f, "root.flx", &["alias", "m"]);

    write(&f, "one.flx", ">> (v);\nv := 1z;");
    write(&f, "root.flx", "<< \"one.flx\" : m;\n!{ () };");
    reject(&f, "root.flx", &["one.flx", "v"]);
}

#[test]
fn imported_nominal_records_have_module_identity() {
    let f = TempDir::new("modules");
    let library = r#"
>> (row, make, get);
row :: { id:z; };
make(x:z):row := row { id := x; };
get(x:row):z := .id x;
"#;
    write(&f, "a.flx", library);
    write(&f, "b.flx", library);
    let imports = "<< \"a.flx\" : a;\n<< \"b.flx\" : b;\n<< \"io\";\n";
    write(
        &f,
        "root.flx",
        &format!("{imports}x:a.row := a.make 3z; !{{ ^{{ ^? ! io.print a.get x; () }} }};"),
    );
    accept(&f, "", "root.flx", "3\n");
    write(
        &f,
        "root.flx",
        &format!("{imports}x := a.get (b.make 3z); !{{ () }};"),
    );
    reject(&f, "root.flx", &["root.flx:4:"]);
}

#[test]
fn qualified_sum_constructors_and_recovery_use_the_exported_identity() {
    let f = TempDir::new("modules");
    let library = r#"
>> (failure, fail);
failure :: { | message s; | spare; };
fail(x:s):s^failure := ^{ ^: message x };
"#;
    write(&f, "a.flx", library);
    write(&f, "b.flx", library);
    let imports = "<< \"a.flx\" : a;\n<< \"b.flx\" : b;\n<< \"io\";\n";
    write(
        &f,
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
    write(&f,
        "root.flx",
        &format!("{imports}v := ^= a.fail \"alpha\" {{ b.message msg : msg; _ : \"unused\"; }}; !{{ () }};"),
    );
    reject(&f, "root.flx", &["root.flx:4:"]);
}

#[test]
fn only_the_root_may_have_an_entry_and_checks_execute_nothing() {
    let f = TempDir::new("modules");
    write(
        &f,
        "bad.flx",
        "<< \"io\";\n!{ ^{ ^? ! io.write \"should-not-exist.txt\" \"bad\"; () } };",
    );
    write(&f, "root.flx", "<< \"bad.flx\" : bad;\n!{ () };");
    reject(&f, "root.flx", &["bad.flx", "entry"]);
    assert!(!f.join("should-not-exist.txt").exists());

    write(&f,
        "good.flx",
        "<< \"io\";\n>> (pending);\npending:!(()^ioerr) := io.write \"should-not-exist.txt\" \"bad\";",
    );
    write(&f, "root.flx", "<< \"good.flx\" : good;\n!{ () };");
    accept(&f, "", "root.flx", "");
    assert!(!f.join("should-not-exist.txt").exists());
}

#[test]
fn import_cycles_include_every_participating_file_and_an_import_location() {
    let f = TempDir::new("modules");
    write(&f, "root.flx", "<< \"a.flx\" : a;\n!{ () };");
    write(&f, "a.flx", "<< \"dir/b.flx\" : b;");
    write(&f, "dir/b.flx", "<< \".././a.flx\" : a;");
    let diag = reject(&f, "root.flx", &["cycle", "a.flx", "b.flx"]);
    assert!(diag.contains(".flx:1:"), "{diag}");
}

#[test]
fn imported_parse_type_and_runtime_errors_report_the_library_file() {
    let f = TempDir::new("modules");
    write(&f, "root.flx", "<< \"lib.flx\" : lib;\n!{ () };");
    write(&f, "lib.flx", ">> (bad);\nbad(x:z):z := + x;");
    reject(&f, "root.flx", &["lib.flx:2:", "operand"]);
    write(&f, "lib.flx", ">> (bad);\nbad(x:z):z := \"mistake\";");
    reject(&f, "root.flx", &["lib.flx:2:"]);

    write(&f, "lib.flx", ">> (bad);\nbad(x:q):q := / 1q x;");
    write(
        &f,
        "root.flx",
        "<< \"lib.flx\" : lib;\n<< \"io\";\n!{ ^{ ^? ! io.print lib.bad 0q; () } };",
    );
    assert!(cli(&f, "", "check", "root.flx").status.success());
    let run = cli(&f, "", "run-hir", "root.flx");
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(!run.status.success() && run.stdout.is_empty());
    assert!(
        stderr.contains("lib.flx:2:") && stderr.contains("division"),
        "{stderr}"
    );
}

#[test]
fn remote_absolute_and_dynamic_source_imports_are_not_supported() {
    let f = TempDir::new("modules");
    for source in [
        "<< \"https://example.invalid/remote.flx\" : m; !{ () };",
        "<< \"file:///sample.flx\" : m; !{ () };",
        "<< \"/absolute.flx\" : m; !{ () };",
        "<< \"C:/absolute.flx\" : m; !{ () };",
        "path := \"library.flx\"; << path : m; !{ () };",
    ] {
        write(&f, "root.flx", source);
        reject(&f, "root.flx", &["root.flx:1:"]);
    }
}

#[test]
fn imports_do_not_reexport_private_or_transitive_names() {
    let f = TempDir::new("modules");
    write(&f, "a.flx", ">> (v);\nv:z := 1z;");
    write(&f, "b.flx", "<< \"a.flx\" : a;\n>> (w);\nw:z := a.v;");
    for visible in ["a.v", "b.v"] {
        write(
            &f,
            "root.flx",
            &format!("<< \"b.flx\" : b;\nx := {visible};\n!{{ () }};"),
        );
        reject(&f, "root.flx", &["root.flx:2:", visible]);
    }
}

#[test]
fn default_module_namespace_and_public_path_api_share_the_same_loader() {
    let f = TempDir::new("modules");
    write(&f, "app/finance.flx", FINANCE);
    write(
        &f,
        "app/edit.flx",
        "<< \"finance.flx\"; << \"io\"; !{ ^{ ^? ! io.print finance.net 12q 8q; () } };",
    );
    accept(&f, "", "app/edit.flx", "4\n");
    let path = f.join("app/edit.flx");
    assert!(flux_lang::check_path(&path).is_ok());
    let output = flux_lang::run_path(&path).expect("public typed path run");
    assert_eq!(output.stdout, "4\n");
}

#[test]
fn an_uncaught_imported_failure_retains_origin_and_root_propagation_note() {
    let f = TempDir::new("modules");
    write(&f,
        "lib.flx",
        ">> (failure, fail);\nfailure :: { | missing; };\nfail(x:()):()^failure := ^{ ^: missing };",
    );
    write(
        &f,
        "root.flx",
        "<< \"lib.flx\" : lib;\n!{ ^{ ^? lib.fail (); () } };",
    );
    assert!(cli(&f, "", "check", "root.flx").status.success());
    let out = cli(&f, "", "run-hir", "root.flx");
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
    let f = TempDir::new("modules");
    write(&f,
        "row.flx",
        ">> (row, make, get);\nrow :: { id:z; };\nmake(x:z):row := row { id := x; };\nget(x:row):z := .id x;",
    );
    write(
        &f,
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
    let f = TempDir::new("modules");
    write(&f, "root.flx", "<< \"lib.flx\" : lib;\n!{ () };");
    for source in [
        ">> (make);\nhidden :: { id:z; };\nmake(x:z):hidden := hidden { id := x; };",
        ">> (public);\nhidden :: { id:z; };\npublic :: { field:hidden; };",
        ">> (failure);\nhidden :: { id:z; };\nfailure :: { | one hidden; };",
    ] {
        write(&f, "lib.flx", source);
        reject(&f, "root.flx", &["lib.flx:3:", "private nominal", "hidden"]);
    }
}

#[test]
fn public_nominal_from_a_dependency_can_appear_in_another_interface() {
    let f = TempDir::new("modules");
    write(
        &f,
        "row.flx",
        ">> (row, make);\nrow :: { id:z; };\nmake(x:z):row := row { id := x; };",
    );
    write(
        &f,
        "ops.flx",
        "<< \"row.flx\" : r;\n>> (get);\nget(x:r.row):z := .id x;",
    );
    write(
        &f,
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
