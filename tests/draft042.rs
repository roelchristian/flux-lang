use flux_lang::builtins::{Availability, REGISTRY};
use std::{
    collections::BTreeSet,
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Script(PathBuf);

impl Script {
    fn new(source: &str) -> Self {
        loop {
            let root = std::env::temp_dir().join(format!(
                "flux-draft042-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&root) {
                Ok(()) => {
                    fs::write(root.join("test.flx"), source).expect("write test script");
                    return Self(root);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("create fixture directory: {e}"),
            }
        }
    }

    fn cli(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_flux"))
            .current_dir(&self.0)
            .args(args)
            .output()
            .expect("invoke public CLI")
    }
}

impl Drop for Script {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const PRELUDE: &str = r#"
<< "io";
<< "math";
<< "df";
key(x:z):z := % x 2;
rs :: { k:z; value:z; };
rows:#rs := #[rs { k := 1; value := 2; } rs { k := 1; value := 3; }];
summ(k:z,t:#rs) := .{ k := k; count := # t; };
"#;

type Case = (&'static str, &'static str, &'static str);

const ARITHMETIC: &[Case] = &[
    ("+", "+ [1 2] 3", "[4 5]"),
    ("-", "- 7 2", "5"),
    ("*", "* 3 4", "12"),
    ("/", "/ 3 4", "3/4"),
    (
        "math.div",
        "^= math.div 3q 0q { divzero : 7q; range : 9q; }",
        "7",
    ),
    ("//", "// 7 3", "2"),
    ("%", "% 7 3", "1"),
    ("^", "^ 2 4", "16"),
    ("-:", "-: 4", "-4"),
    ("^/", "^/ 81", "9"),
    ("|", "| -4", "4"),
    (":n", ":n 4z", "4"),
    (":z", ":z 4n", "4"),
    (":q", ":q 0.5r", "1/2"),
    (":d", ":d 1z", "1"),
    ("math.quantize", "math.quantize 2n (/ 1z 3z)", "0.33"),
    (":r", ":r 0.5q", "0.5"),
];

const LOGIC: &[Case] = &[
    ("<", "< 2 3", "1b"),
    ("<=", "<= 3 3", "1b"),
    (">", "> 3 2", "1b"),
    (">=", ">= 3 3", "1b"),
    ("=", "= [1 2] 2", "[0b 1b]"),
    ("/=", "/= 1 2", "1b"),
    ("==", "== [1 2] [2 1]", "0b"),
    ("~", "~ 1b", "0b"),
    ("&&", "&& 1b 0b", "0b"),
    ("||", "|| 1b 0b", "1b"),
    ("&&/", "&&/ [1b 1b]", "1b"),
    ("||/", "||/ [0b 1b]", "1b"),
    ("?0", "?? 2z ?0", "2"),
    ("?1", "?1 5z", "?1 5"),
    ("?+", "?+ (?1 5z)", "1b"),
    ("??", "?? (/ 1q 0q) (?1 5q)", "5"),
    ("?!", "?! (?1 5z)", "5"),
];

const COLLECTIONS: &[Case] = &[
    ("#", "# [1 2 3]", "3"),
    ("..", ".. 2 4", "[2 3 4]"),
    ("@", "@ (@ &+ 2z) 3z", "5"),
    ("@@", "@@ &+ (2z,3z)", "5"),
    ("@:", "@: &key [3z 4z]", "[1 0]"),
    ("?:", "?: [1b 0b] [4z 5z]", "[4]"),
    ("=:", ".: 1z (=: &key [3z 4z 5z])", "?1 [3 5]"),
    (">:", ">: &key [3z 4z 1z 2z]", "[4 2 3 1]"),
    ("<:", "<: &key [3z 4z 1z 2z]", "[3 1 4 2]"),
    ("+:", "+: [1z] [2z 3z]", "[1 2 3]"),
    ("&:", "&: [1z 2z] [\"a\" \"b\"]", "[(1, a) (2, b)]"),
    (
        "*:",
        "*: [1z 2z] [\"a\" \"b\"]",
        "[(1, a) (1, b) (2, a) (2, b)]",
    ),
    (
        "=.",
        "=. [(1z,\"a\") (2z,\"b\") (1z,\"a\")]",
        "[(1, a) (2, b)]",
    ),
    (".:", ".: 2n [4z 6z]", "?1 6"),
    ("[:", "[: 2n [4z 6z 9z]", "[4 6]"),
    ("/:", "/: &- 10z [1z 2z 3z]", "4"),
    ("\\:", "\\: &- 10z [1z 2z 3z]", "[10 9 7 4]"),
    ("take", "take 2n \"abc\"", "ab"),
    ("drop", "drop 2n [1z 2z 3z]", "[3]"),
    ("slice", "slice 2n 2n [1z 2z 3z 4z]", "[2 3]"),
    ("rev", "rev \"a\\u{e9}c\"", "céa"),
    ("flat", "flat [[1z 2z] [3z]]", "[1 2 3]"),
    ("tr", "tr [[1z 2z] [3z 4z]]", "[[1 3] [2 4]]"),
];

const REDUCTIONS: &[Case] = &[
    ("+/", "+/ [3z 4z]", "7"),
    ("*/", "*/ [3z 4z]", "12"),
    (">/", ">/ [3z 9z 4z]", "?1 9"),
    ("</", "</ [3z 9z 4z]", "?1 3"),
    ("avg", "avg [1q 2q]", "?1 3/2"),
    ("pct", "pct 25q [6q 0q]", "?1 3/2"),
    ("med", "med [1q 4q]", "?1 5/2"),
    ("sd", "sd [4q 4q]", "?1 0"),
    ("var", "var [1q 3q]", "?1 1"),
];

const TABLES: &[Case] = &[
    ("#[", "# #[.{ x := 2z; }]", "1"),
    ("#:", ".x #: .{ x := [2z 3z]; }", "[2 3]"),
    (
        ":#",
        ":# rows",
        "[{k := 1; value := 2} {k := 1; value := 3}]",
    ),
    ("df.agg", ".count df.agg &.k &summ rows", "[2]"),
    ("df.structural", ".value df.structural rows", "[2 3]"),
    ("df.extend", ".b df.extend .{ b := [8z 9z]; } rows", "[8 9]"),
];

const EFFECTS: &[&str] = &[
    "!",
    "io.print",
    "io.read",
    "io.write",
    "io.exists",
    "csv.read",
    "csv.write",
];

// Each name below is actually invoked in the strict public fixtures in
// tests/draft05_stdlib.rs. Prelude print, fs.exists and stat add aliases,
// not new canonical entries. Keep complete equality with the registry.
const STDLIB_05: &[&str] = &[
    "opt.map",
    "opt.bind",
    "opt.result",
    "res.map",
    "res.maperr",
    "res.collect",
    "res.partition",
    "res.ok",
    "res.optional",
    "io.readln",
    "fs.ls",
    "fs.mkdir",
    "fs.base",
    "fs.join",
    "sys.args",
    "sys.env",
    "sys.pid",
    "str.upper",
    "str.lower",
    "str.trim",
    "str.split",
    "str.join",
    "rx.find",
    "dt.days",
    "dt.date",
    "json.encode",
    "json.decode",
];

// Each entry has a strict public fixture in tests/draft06_relational.rs.
const RELATIONAL_06: &[&str] = &[
    "*=",
    "*?",
    "df.semi",
    "df.anti",
    "df.union",
    "df.distinct",
    "df.select",
    "df.dropna",
    "df.fill",
    "df.present",
];

// Exercised through ordinary and strict HIR by tests/draft012_arrays.rs.
const ARRAYS_012: &[&str] = &[
    "arr.reshape",
    "arr.shape",
    "arr.ravel",
    "arr.at",
    "arr.map",
    "arr.zip",
    "arr.outer",
    "arr.rankmap",
    "arr.scan",
    "arr.windows",
    "arr.select",
    "arr.grade",
    "arr.permute",
];

fn expression_source(expression: &str) -> String {
    format!("{PRELUDE}\n!{{ ^= ! io.print {expression} {{ _ : (); }}; }};")
}

fn assert_cases(cases: &[Case]) {
    let mut failures = Vec::new();
    for &(op, expression, expected) in cases {
        let f = Script::new(&expression_source(expression));
        let output = f.cli(&["run-hir", "test.flx"]);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !output.status.success() || stdout != format!("{expected}\n") {
            failures.push(format!(
                "{op}: {expression}: expected {expected:?}, got {stdout:?}; {stderr}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn every_runtime_registry_entry_has_a_strict_fixture() {
    let covered: BTreeSet<_> = [ARITHMETIC, LOGIC, COLLECTIONS, REDUCTIONS, TABLES]
        .into_iter()
        .flat_map(|cases| cases.iter().map(|case| case.0))
        .chain(
            EFFECTS
                .iter()
                .chain(STDLIB_05)
                .chain(RELATIONAL_06)
                .chain(ARRAYS_012)
                .copied(),
        )
        .collect();
    let registered: BTreeSet<_> = REGISTRY
        .iter()
        .filter(|entry| entry.availability == Availability::Runtime)
        .map(|entry| entry.canonical)
        .collect();
    assert_eq!(covered, registered);
}

#[test]
fn all_arithmetic_and_conversion_builtins_execute_in_hir() {
    assert_cases(ARITHMETIC);
}

#[test]
fn all_logic_and_optional_builtins_execute_in_hir() {
    assert_cases(LOGIC);
}

#[test]
fn all_collection_builtins_execute_in_hir() {
    assert_cases(COLLECTIONS);
}

#[test]
fn all_numeric_and_order_reductions_execute_in_hir() {
    assert_cases(REDUCTIONS);
}

#[test]
fn all_table_builtins_execute_in_hir() {
    assert_cases(TABLES);
}

#[test]
fn operating_and_csv_actions_execute_truly_and_remain_fallible() {
    let source = r#"
<< "io";
<< "csv";
err :: { | file ioerr; | badcsv csverr; };
row :: { n:z; text:s; };
t:#row := #[row { n := 2; text := "comma,here"; }];
!{
    ^{
        ^= ! io.write "test.txt" "test payload" { e : ^: file e; };
        flag := ^= ! io.exists "test.txt" { e : ^: file e; };
        text := ^= ! io.read "test.txt" { e : ^: file e; };
        ^= ! csv.write "test.csv" t { e : ^: badcsv e; };
        actual:#row := ^= ! csv.read "test.csv" { e : ^: badcsv e; };
        ^= ! io.print (flag,text,.text actual) { e : ^: file e; };
        ()
    }
};
"#;
    let f = Script::new(source);
    let checked = f.cli(&["check", "test.flx"]);
    assert!(
        checked.status.success() && checked.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    assert!(!f.0.join("test.txt").exists());
    let out = f.cli(&["run-hir", "test.flx"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "(1b, test payload, [comma,here])\n"
    );
    assert_eq!(
        fs::read_to_string(f.0.join("test.txt")).unwrap(),
        "test payload"
    );
    assert_eq!(
        fs::read_to_string(f.0.join("test.csv"))
            .unwrap()
            .replace("\r\n", "\n"),
        "n,text\n2,\"comma,here\"\n"
    );
}

#[test]
fn inherited_runtime_semantics_cover_empty_and_indirect_boundaries() {
    let fixture = r#"
<< "io";
sum(f,seed,xs) := /: f seed xs;
minus(x:z,y:z):z := - x y;
xs:[z] := [];
bs:[b] := [];
grid:[[z]] := [];
!{
    ^= ! io.print (sum &minus 8z [1z 2z],sum &minus 8z xs,\: &minus 8z xs) { _ : (); };
    ^= ! io.print (&&/ bs,||/ bs,>/ xs,</ xs,tr grid,var xs,pct 50z xs) { _ : (); };
    ^= ! io.print (.: 2n [1z 2z 3z]) { _ : (); };
};
"#;
    let f = Script::new(fixture);
    let out = f.cli(&["run-hir", "test.flx"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "(5, 8, [8])\n(1b, 0b, ?0, ?0, [], ?0, ?0)\n?1 2\n"
    );
}

#[test]
fn invalid_collection_shapes_have_typed_runtime_diagnostics() {
    for (source, message) in [
        ("&: [1z] [2z 3z]", "length"),
        ("tr [[1z] [2z 3z]]", "rectangular"),
        ("slice 0n 1n [2z]", "one-based"),
        ("pct 101z [1z 2z]", "percentile"),
    ] {
        let f = Script::new(&expression_source(source));
        let check = f.cli(&["check", "test.flx"]);
        assert!(
            check.status.success(),
            "{source}: {}",
            String::from_utf8_lossy(&check.stderr)
        );
        let run = f.cli(&["run-hir", "test.flx"]);
        let stderr = String::from_utf8_lossy(&run.stderr);
        assert!(
            !run.status.success() && !stderr.contains("E0400") && stderr.contains(message),
            "{source}: {stderr}"
        );
    }
}

#[test]
fn exported_module_aliases_use_the_same_typed_operations() {
    let source = r#"
<< "io" : out;
<< "statistics" : stat;
<< "table" : tbl;
row :: { k:z; };
t:#row := #[row { k := 2z; }];
summ(k:z,g:#row) := .{ k := k; size := # g; };
!{ ^= ! out.println (stat.mean [2q 4q],stat.pct 25q [0q 6q],.size tbl.agg &.k &summ t) { _ : (); }; };
"#;
    let f = Script::new(source);
    let out = f.cli(&["run-hir", "test.flx"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "(?1 3, ?1 3/2, [1])\n"
    );
}

#[test]
fn recognized_unsupported_operations_do_not_execute_or_fall_back() {
    let cases = [("~=", "~= 0.1r 1r 2r"), ("cor", "cor [1q 2q] [1q 2q]")];
    let exercised: BTreeSet<_> = cases.iter().map(|(op, _)| *op).collect();
    let registered: BTreeSet<_> = REGISTRY
        .iter()
        .filter(|entry| entry.availability == Availability::Recognized)
        .map(|entry| entry.canonical)
        .collect();
    assert_eq!(exercised, registered);

    for (op, expression) in cases {
        let source = format!(
            "<< \"io\"; k(x:(z,z)):z := .1 x; x := {expression}; !{{ ^= ! io.print x {{ _ : (); }}; }};"
        );
        let f = Script::new(&source);
        for args in [
            vec!["check", "test.flx"],
            vec!["hir", "test.flx"],
            vec!["test.flx"],
        ] {
            let out = f.cli(&args);
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(
                !out.status.success()
                    && out.stdout.is_empty()
                    && stderr.contains(op)
                    && ["not executable", "unsupported", "not implemented"]
                        .iter()
                        .any(|s| stderr.contains(s)),
                "{op} {args:?}: {stderr}"
            );
        }
    }
}

#[test]
fn rust_api_and_repl_execute_former_legacy_operations() {
    use std::{io::Write, process::Stdio};

    let source = expression_source("/: &+ 0z [1z 2z 3z]");
    let checked = flux_lang::check_source(&source);
    assert!(checked.is_ok(), "public check: {checked:?}");
    let result = flux_lang::run_source(&source).expect("public typed run");
    assert_eq!(result.stdout, "6\n");

    let mut repl = Command::new(env!("CARGO_BIN_EXE_flux"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start REPL");
    repl.stdin
        .take()
        .expect("REPL stdin")
        .write_all(b"/: &+ 0z [1z 2z 3z]\n:q\n")
        .unwrap();
    let out = repl.wait_with_output().unwrap();
    assert!(
        out.status.success() && out.stderr.is_empty(),
        "REPL stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("6 : z"));
}

#[test]
fn indirect_fold_preserves_exact_instantiations_and_does_not_redo_overload() {
    let f = Script::new(
        r#"
<< "io";
fold(f,seed,xs) := /: f seed xs;
sub := &-;
!{
    ^= ! io.print fold sub 1q [0.25q] { _ : (); };
    ^= ! io.print fold sub 1n [2n] { _ : (); };
};
"#,
    );
    let check = f.cli(&["check", "test.flx"]);
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    let out = f.cli(&["run-hir", "test.flx"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "3/4\n");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("natural subtraction underflow"), "{stderr}");
}

#[test]
fn empty_table_compositions_keep_collection_and_schema_shape() {
    let f = Script::new(
        r#"
<< "io";
row :: { x:q; };
empty:#row := #[];
full:#row := #[row { x := 1q; }];
joined := +: empty full;
cross := *: empty full;
!{ ^= ! io.print (.x joined,# cross,:# cross) { _ : (); }; };
"#,
    );
    let out = f.cli(&["run-hir", "test.flx"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "([1], 0, [])\n");
}

#[test]
fn canonical_annotation_ast_keeps_type_precedence_and_source_offsets() {
    use flux_lang::parser::{parse_type_annotation, TypeAnnotationKind as K};
    let source = " !(#rows ^ err) -> [?q^err] ";
    let root = parse_type_annotation(source).expect("type syntax");
    assert_eq!(&source[root.span.clone()], "!(#rows ^ err) -> [?q^err]");
    let K::Function(args, output) = root.kind else {
        panic!("arrow was not outermost");
    };
    let [param] = args.as_slice() else {
        panic!("expected one argument");
    };
    let K::Action(body) = &param.kind else {
        panic!("argument should be an action returning a result");
    };
    let K::Result(ok, err) = &body.kind else {
        panic!("action output was not a result");
    };
    assert!(matches!(ok.kind, K::Table(_)));
    assert!(matches!(&err.kind, K::Named(name) if name == "err"));
    assert_eq!(&source[err.span.clone()], "err");
    let K::Sequence(element) = output.kind else {
        panic!("result of function should be a sequence");
    };
    let K::Result(optional, _) = element.kind else {
        panic!("sequence element was not a result");
    };
    assert!(matches!(optional.kind, K::Optional(_)));
}
