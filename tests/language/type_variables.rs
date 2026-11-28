use super::fixture::{rejects, Fixture};
use crate::support::flux;

#[test]
fn callable_schemes_and_numeric_constraints_use_tildes() {
    Fixture::new(
        r#"
id/1:~value->~value := &.;
square/1:~number->~number | num ~number := &{ @* * . . };
!{ ^{
    ^? ! print (id "text",id 2z,square 3n,square 0.5q,square 1.20d);
    ()
} };
"#,
    )
    .accepts("", "(text, 2, 9, 1/4, 1.4400)\n");
}

#[test]
fn exported_generic_sums_and_open_rows_have_tilde_schemes() {
    let fixture = Fixture::new(
        r#"
<< "values.flx" : v;
a := v.item .{note:="a"; value:=1z;};
b := v.item .{value:="b"; flag:=1b;};
!{ ^{ ^? ! print (v.get a,v.get b); () } };
"#,
    );
    fixture.write(
        "values.flx",
        r#"
>> (box, get);
box[~item] :: { | item ~item; };
get(x:box[.{value:~item; | ~rest}]):~item := ?= x { item v : .value v; };
"#,
    );
    fixture.accepts("", "(1, b)\n");
    let out = flux()
        .current_dir(fixture.path())
        .args(["interface", "values.flx"])
        .output()
        .expect("inspect exported type schemes");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("box[~") && stdout.contains("| ~"),
        "{out:?}"
    );
    assert!(!stdout.contains("'t") && !stdout.contains("'r"), "{stdout}");
}

#[test]
fn diagnoses_legacy_type_variables_in_all_public_type_positions() {
    for (source, replacement) in [
        ("id(x:'a):'a := x;", "~a"),
        ("box['a] :: { | item 'a; };", "~a"),
        ("box[~a] :: { | item 'a; };", "~a"),
        ("id(x:.{value:z; | 'r}):z := .value x;", "~r"),
        ("square(x:~a):~a | num 'a := * x x;", "~a"),
    ] {
        rejects(source, &["apostrophe-prefixed type variables", replacement]);
    }
}

#[test]
fn rejects_malformed_tilde_type_variable_tokens() {
    for source in [
        "id(x:~ a):z := 0z;",
        "id(x:~):z := 0z;",
        "id(x:~A):z := 0z;",
        "id(x:~_):z := 0z;",
        "bad[~ a] :: { | item ~a; };",
    ] {
        rejects(source, &["main.flx:1:"]);
    }
}

#[test]
fn quote_characters_and_prefix_tilde_remain_distinct_from_type_syntax() {
    let fixture = Fixture::new(
        r#"
echo(x:~a):~a := x;
a := 1b;
!{ ^{
    ^? ! print (echo 'a',echo '~',echo '\'',echo '\u{03BB}',~ 1b,~0b,~a,~ a);
    ()
} };
"#,
    );
    fixture.accepts("", "(a, ~, ', λ, 0b, 1b, 0b, 0b)\n");
    let out = fixture.cli("fmt", "");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let source = String::from_utf8(out.stdout).unwrap();
    for token in ["~a", "'a'", "'~'", "'\\''", "'\\u{03BB}'"] {
        assert!(source.contains(token), "{token:?}: {source}");
    }
    Fixture::new(&source).accepts("", "(a, ~, ', λ, 0b, 1b, 0b, 0b)\n");
}
