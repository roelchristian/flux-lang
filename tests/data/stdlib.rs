use super::script::Script;
use std::{
    fs,
    io::Write,
    process::{Output, Stdio},
};

fn script(source: &str) -> Script {
    Script::new("flux-stdlib", "test.flx", source)
}

fn cli(
    script: &Script,
    mode: Option<&str>,
    program_args: &[&str],
    input: &[u8],
    env: &[(&str, &str)],
) -> Output {
    let mut command = script.command(mode);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .envs(env.iter().copied());
    if !program_args.is_empty() {
        command.arg("--").args(program_args);
    }
    let mut child = command.spawn().expect("invoke Flux");
    if !input.is_empty() {
        child
            .stdin
            .take()
            .expect("Flux stdin")
            .write_all(input)
            .expect("write input");
    }
    child.wait_with_output().expect("wait for Flux output")
}

fn expect_output(out: Output, expected: &str) {
    assert!(
        out.status.success() && out.stderr.is_empty(),
        "run: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
}

#[test]
fn prelude_print_stays_suspended_until_explicit_execution() {
    let f = script(
        r#"
pending := print "do not print this";
!{ ^{ ^? ! print "Salve, munde."; () } };
"#,
    );
    f.check_hir();
    for mode in [Some("run-hir"), None] {
        expect_output(f.run(mode), "Salve, munde.\n");
    }

    let f = script(r#"!{ ! print "must handle the result"; () };"#);
    for mode in [Some("check"), Some("hir"), None] {
        let out = f.run(mode);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !out.status.success() && out.stdout.is_empty() && stderr.contains("discard"),
            "{mode:?}: {stderr}"
        );
    }
}

#[test]
fn optional_callbacks_run_only_for_present_values_and_can_bind_absence() {
    let f = script(
        r#"
<< "opt";
bad(x:q):q := / 1q 0q;
bump(x:q):q := + x 1q;
choose(x:q):?q := ?{ = x 0q : ?0; _ : ?1 x; };
!{
    ^{
        ^? ! print (opt.map &bad ?0,opt.map &bump (?1 2q));
        ^? ! print (opt.bind &choose (?1 0q),opt.bind &choose (?1 2q),opt.bind &choose ?0);
        none:?q := ?0;
        ^? ! print (opt.result "missing" (?1 3q),opt.result "missing" none);
        ()
    }
};
"#,
    );
    f.check_hir();
    let out = f.run(Some("run-hir"));
    expect_output(out, "(?0, ?1 3)\n(?0, ?1 2, ?0)\n(ok 3, err missing)\n");
}

const RESULT_PRELUDE: &str = r#"
<< "res";
e :: { | bad s; };
f :: { | wrapped e; };
failure(x:s):q^e := ^{ ^: bad x };
success(x:q):q^e := ^{ x };
bomb(x:q):q := / 1q 0q;
badmap(x:e):q := / 1q 0q;
toerr(x:e):f := wrapped x;
bump(x:q):q := + x 1q;
"#;

#[test]
fn result_callbacks_are_branch_selective_and_sequence_keeps_the_first_error() {
    let source = format!(
        r#"{RESULT_PRELUDE}
!{{
    ^{{
        ^? ! print (res.map &bomb (failure "first"),res.map &bump (success 2q));
        ^? ! print (res.maperr &badmap (success 3q),res.maperr &toerr (failure "converted"));
        rs := [success 1q (failure "first") success 2q (failure "second")];
        ^? ! print (res.collect rs,res.partition rs);
        ^? ! print (res.ok (success 2q),res.ok (failure "x"),res.optional (failure "x"),res.optional (success 5q));
        ^? ! print (res.collect [success 1q success 2q]);
        ()
    }}
}};"#
    );
    let f = script(&source);
    f.check_hir();
    expect_output(
        f.run(Some("run-hir")),
        "(err bad first, ok 3)\n(ok 3, err wrapped bad converted)\n(err bad first, ([1 2], [bad first bad second]))\n(1b, 0b, ?0, ?1 5)\nok [1 2]\n",
    );
}

#[test]
fn result_error_mapping_keeps_the_original_raise_location() {
    let f = script(&format!(
        r#"{RESULT_PRELUDE}
!{{
    ^{{
        xs := [failure "first" failure "second"];
        ^? res.maperr &toerr (res.collect xs);
        ()
    }}
}};"#
    ));
    f.check_hir();
    let out = f.run(Some("run-hir"));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    for part in [
        "test.flx:5:",
        "wrapped bad first",
        "propagated here",
        "R0101",
    ] {
        assert!(stderr.contains(part), "{part}: {stderr}");
    }
    assert!(!stderr.contains("bad second"), "{stderr}");
}

#[test]
fn process_args_env_pid_and_readln_have_explicit_effects() {
    let f = script(
        r#"
<< "io";
<< "sys";
!{
    ^{
        args := ! sys.args;
        pid := ! sys.pid;
        env := ^? ! sys.env "FLUX_STDLIB_TEST_ENV";
        a := ^? ! io.readln;
        b := ^? ! io.readln;
        c := ^? ! io.readln;
        end := ^? ! io.readln;
        ^? ! print (args,> pid 0n,env,a,b,c,end);
        ()
    }
};
"#,
    );
    f.check_hir();
    for mode in [None, Some("run-hir")] {
        expect_output(
            cli(
                &f,
                mode,
                &["--literal", "two words"],
                b"hello\r\n\nlast\r",
                &[("FLUX_STDLIB_TEST_ENV", "injected")],
            ),
            "([--literal two words], 1b, ?1 injected, ?1 hello, ?1 , ?1 last\r, ?0)\n",
        );
    }
}

#[test]
fn readln_non_utf8_is_recoverable_not_eof() {
    let f = script(
        r#"
<< "io";
!{ ^{
    result := ! io.readln;
    ^? ! print (^= result { encoding p : ?1 p; _ : ?0; });
    ()
} };
"#,
    );
    f.check_hir();
    expect_output(
        cli(&f, Some("run-hir"), &[], &[255, b'\n'], &[]),
        "?1 stdin\n",
    );
}

#[test]
fn filesystem_paths_are_lexical_and_actions_do_not_clobber() {
    let f = script(
        r#"
<< "fs";
<< "io";
<< "res";
pending := fs.mkdir "pending";
!{ ^{
    ^? ! fs.mkdir "dir";
    ^? ! io.write (fs.join "dir" "zeta") "z";
    ^? ! io.write (fs.join "dir" "alpha") "a";
    names := ^? ! fs.ls "dir";
    before := ^? ! fs.exists "pending";
    ^? ! print (names,fs.base (fs.join "parent" "child"),fs.base "",before);
    duplicate := ! fs.mkdir "dir";
    ^? ! print (res.ok duplicate);
    ^= duplicate { exists name : ^= ! print name { _ : (); }; _ : (); };
    ()
} };
"#,
    );
    f.check_hir();
    assert!(!f.path().join("dir").exists());
    expect_output(
        f.run(Some("run-hir")),
        "([alpha zeta], ?1 child, ?0, 0b)\n0b\ndir\n",
    );
    assert_eq!(fs::read_to_string(f.path().join("dir/alpha")).unwrap(), "a");
    assert!(!f.path().join("pending").exists());
}

#[test]
fn unicode_strings_and_regex_use_checked_pure_operations() {
    let f = script(
        r#"
<< "str";
<< "rx";
!{ ^{
    ^? ! print (str.upper "Stra\u{df}e",str.lower "\u{130}",str.trim "\u{2003}ok\u{2003}");
    ^? ! print (str.split "," "a,,b,",str.join "|" ["a" "" "b"],str.split "" "a\u{e9}");
    ^? ! print (rx.find "\\p{L}+" "99 \u{c9}cho 77",rx.find "z+" "x",rx.find "[" "x");
    ()
} };
"#,
    );
    f.check_hir();
    expect_output(
        f.run(Some("run-hir")),
        "(STRASSE, i\u{0307}, ok)\n([a  b ], a||b, [ a é ])\n(ok ?1 Écho, ok ?0, err badpattern [)\n",
    );
}

#[test]
fn gregorian_dates_use_unix_days_and_reject_invalid_leap_days() {
    let f = script(
        r#"
<< "dt";
!{ ^{
    ^? ! print (dt.days "1970-01-01",dt.days "1969-12-31",dt.days "0001-01-01",dt.date -1z);
    ^? ! print (dt.days "2000-02-29",dt.days "1900-02-29",dt.days "0000-01-01",dt.date 2932897z);
    ()
} };
"#,
    );
    f.check_hir();
    expect_output(
        f.run(Some("run-hir")),
        "(ok 0, ok -1, ok -719162, ok 1969-12-31)\n(ok 11016, err baddate 1900-02-29, err baddate 0000-01-01, err baddate 2932897)\n",
    );
}

#[test]
fn stat_namespace_reuses_existing_exact_reductions() {
    let f = script(
        r#"
<< "stat" : stats1;
!{ ^{ ^? ! print (stats1.mean [1q 2q],stats1.pct 25q [6q 0q],stats1.sd [4q 4q]); () } };
"#,
    );
    f.check_hir();
    expect_output(f.run(Some("run-hir")), "(?1 3/2, ?1 3/2, ?1 0)\n");

    let f = script(r#"<< "stat"; x := stat.cor [1q 2q] [1q 2q]; !{ () };"#);
    let check = f.run(Some("check"));
    let stderr = String::from_utf8_lossy(&check.stderr);
    assert!(
        !check.status.success() && stderr.contains("cor"),
        "{stderr}"
    );
}

#[test]
fn json_round_trips_typed_nominal_and_structural_values_without_losing_precision() {
    let f = script(
        r#"
<< "json";
err :: { | serial jsonerr; | output ioerr; };
row :: { price:d; amount:z; rate:q; tag:?s; parts:[z]; };
!{ ^{
    row:row := ^= json.decode "{\"parts\":[2,1],\"tag\":null,\"rate\":1.25e-1,\"amount\":340282366920938463463374607431768211457,\"price\":123.4500}" { e : ^: serial e; };
    text := ^= json.encode row { e : ^: serial e; };
    plain:.{ count:z; name:s; } := ^= json.decode "{\"name\":\"office\",\"count\":2.0}" { e : ^: serial e; };
    plaintext := ^= json.encode plain { e : ^: serial e; };
    ^= ! print text { e : ^: output e; };
    ^= ! print plaintext { e : ^: output e; };
    ()
} };
"#,
    );
    f.check_hir();
    expect_output(
        f.run(Some("run-hir")),
        "{\"price\":123.4500,\"amount\":340282366920938463463374607431768211457,\"rate\":0.125,\"tag\":null,\"parts\":[2,1]}\n{\"count\":2,\"name\":\"office\"}\n",
    );
}

#[test]
fn json_errors_distinguish_syntax_and_shape_and_keep_types_static() {
    let f = script(
        r#"
<< "json";
<< "res";
row :: { name:s; };
decode(s:s):row^jsonerr := json.decode s;
yes(x:row):s := .name x;
kind(x:row^jsonerr):s := ^= res.map &yes x { invalidjson _ : "syntax"; jsonshape _ : "shape"; };
enc(x:s^jsonerr):s := ^= x { invalidjson _ : "syntax"; jsonshape _ : "shape"; };
none:?s := ?0;
!{ ^{
    ^? ! print (kind (decode "{"),kind (decode "{}"),kind (decode "{\"name\":\"a\",\"name\":\"b\"}"));
    ^? ! print (kind (decode "{\"name\":\"a\",\"extra\":false}"),kind (decode "{\"name\":\"ok\"}"));
    ^? ! print (enc (json.encode (/ 1z 3z)),enc (json.encode (?1 none)));
    ()
} };
"#,
    );
    f.check_hir();
    expect_output(
        f.run(Some("run-hir")),
        "(syntax, shape, shape)\n(shape, ok)\n(shape, shape)\n",
    );

    for binding in [
        r#"x := json.decode "1";"#,
        r#"x:(?())^jsonerr := json.decode "null";"#,
        r#"x:r^jsonerr := json.decode "1";"#,
    ] {
        let source = format!("<< \"json\"; {binding} !{{ () }};");
        let f = script(&source);
        let checked = f.run(Some("check"));
        let stderr = String::from_utf8_lossy(&checked.stderr);
        assert!(
            !checked.status.success() && stderr.contains("json.decode"),
            "{stderr}"
        );
    }
}

#[test]
fn oversized_regex_and_json_inputs_fail_recoverably() {
    let pattern = "a".repeat((1 << 16) + 1);
    let doc = "a".repeat((1 << 20) + 1);
    let source = format!(
        r#"
<< "rx";
<< "json";
<< "res";
!{{ ^{{
    d:s^jsonerr := json.decode "\"{doc}\"";
    ^? ! print (res.ok (rx.find "{pattern}" "ab"),res.ok d);
    ()
}} }};
"#
    );
    let f = script(&source);
    f.check_hir();
    expect_output(f.run(Some("run-hir")), "(0b, 0b)\n");
}

#[test]
fn json_decode_bounds_total_numeric_expansion_per_document() {
    // Each zero has a cheap coefficient and a legal scale, but the scale
    // still costs digits to expand. Keep the complete inputs below 1 KiB.
    let mut nums = vec!["0e-10000"; 99];
    nums.push("0e-9900");
    let at_limit = format!("[{}]", nums.join(","));
    nums.push("0");
    let over_limit = format!("[{}]", nums.join(","));
    let source = format!(
        r#"<< "json";
<< "res";
decode(s:s):[d]^jsonerr := json.decode s;
!{{
    status := ^= res.map (\xs -> "success") (decode "{over_limit}") {{
        invalidjson _ : "syntax error";
        jsonshape note : note;
    }};
    ^= ! print (status,res.ok (decode "{at_limit}")) {{ _ : (); }};
}};"#
    );
    let f = script(&source);
    f.check_hir();
    for mode in [None, Some("run-hir")] {
        expect_output(
            f.run(mode),
            "(JSON total numeric expansion exceeds the resource limit, 1b)\n",
        );
    }
}
