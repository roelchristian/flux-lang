use crate::support::{flux, TempDir};
use std::{fs, path::Path, process::Output};

struct Script(TempDir);

impl Script {
    fn new(source: &str) -> Self {
        let dir = TempDir::new("flux-sequences");
        fs::write(dir.join("probe.flx"), source).expect("write fixture");
        Self(dir)
    }

    fn run(&self, mode: &[&str], file: impl AsRef<Path>) -> Output {
        flux()
            .current_dir(self.0.path())
            .args(mode)
            .arg(file.as_ref())
            .output()
            .expect("invoke Flux")
    }
}

fn strict(source: &str, expected: &str) {
    let script = Script::new(source);
    for mode in [&["check"][..], &["run-hir"][..], &[][..]] {
        let out = script.run(mode, "probe.flx");
        assert!(
            out.status.success(),
            "{mode:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            if mode == ["check"] { "" } else { expected },
            "{mode:?}"
        );
    }
}

fn reject(source: &str, wanted: &[&str]) {
    let script = Script::new(&format!("{source}\n!{{ () }};\n"));
    for mode in [&["check"][..], &["run-hir"][..]] {
        let out = script.run(mode, "probe.flx");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !out.status.success()
                && out.stdout.is_empty()
                && wanted.iter().any(|fragment| stderr.contains(fragment)),
            "{mode:?}: {stderr}\n{source}"
        );
    }
}

#[test]
fn unseeded_reduction_is_a_left_fold_without_an_implicit_identity() {
    strict(
        r#"
empty:[z]:=[];
fail(a:z,b:z):z := :z (/ a (- b b));
present := @/ &- [10z 3z 2z];
unit := @/ &fail [7z];
absent := @/ &fail empty;
!{
    ^= ! print (present,unit,absent) { _ : (); };
};
"#,
        "(?1 5, ?1 7, ?0)\n",
    );
}

#[test]
fn unseeded_reduction_supports_checked_references_and_tacit_calls() {
    strict(
        r#"
reduce:(z->z->z)->[z]->?z := &@/;
choose(x:z):z->z->z := &-;
tacit:z->([z]->?z) := &{ @. @/ choose };
!{ ^{ ^? ! print (@ (@ reduce &-) [12z 5z 2z],@ (@ tacit 1z) [12z 5z 2z]); () } };
"#,
        "(?1 5, ?1 5)\n",
    );
}

#[test]
fn reduction_preserves_nested_optionals_and_suspends_actions() {
    strict(
        r#"
choose_left(a:?z,b:?z):?z := a;
payload:[?z] := [?0 (?1 7z)];
either(a:!(()^ioerr),b:!(()^ioerr)):!(()^ioerr) := !{
    ^{ ^? ! a; ^? ! b; () }
};
pending := @/ &either [print "first" print "second" print "third"];
!{
    ^{ ^? ! print (@/ &choose_left payload); ^? ! (?! pending); () }
};
"#,
        "?1 ?0\nfirst\nsecond\nthird\n",
    );
}

#[test]
fn canonical_sequences_keep_legacy_order_schema_and_unicode_semantics() {
    strict(
        r#"
row :: { k:z; };
t:#row := #[row { k:=1z; } row { k:=2z; } row { k:=3z; }];
!{ ^{
    ^? ! print (.k ([:2n t),.k (:]2n t),.k (sect 2n 8n t));
    ^? ! print ([:2n "a\u{e9}z",:]1n "a\u{e9}z",sect 2n 1n "a\u{e9}z");
    ^? ! print (,. [[1z 2z] [3z]],|: [[1z 2z] [3z 4z]],rev "a\u{e9}z");
    ^? ! print (== ([:2n t) (take 2n t),
                == (:]2n t) (drop 2n t),
                == (sect 2n 8n t) (slice 2n 8n t),
                == (,. [[1z] [2z]]) (flat [[1z] [2z]]),
                == (|: [[1z] [2z]]) (tr [[1z] [2z]]));
    ()
} };
"#,
        "([1 2], [3], [2 3])\n(aé, éz, é)\n([1 2 3], [[1 3] [2 4]], zéa)\n(1b, 1b, 1b, 1b, 1b)\n",
    );
}

#[test]
fn compact_tokens_do_not_capture_other_at_bar_colon_or_comma_forms() {
    strict(
        r#"
num(x:z):z := x;
f:=&@/;
results:=(@/ &- [9z 4z 1z],@: &num [7z],/: &- 9z [4z 1z],
          \: &- 9z [4z 1z],:]1n [8z 3z],|: [[1z 2z]],,. [[1z] [2z]],
          .: 1n [4z], @ (&{ @. . . }) 7z);
!{ ^= ! print (results,@ (@ f &-) [9z 4z 1z]) { _ : (); }; };
"#,
        "((?1 4, [7], 4, [9 5 4], [3], [[1] [2]], [1 2], ?1 4, 7), ?1 4)\n",
    );
}

#[test]
fn unseeded_reduction_rejects_wrong_arity_and_callback_shapes() {
    for (source, detail) in [
        (
            r#"f := @/ &-;"#,
            &["missing operand", "expected", "unknown"][..],
        ),
        (
            r#"square(x:z):z := * x x; f := @/ &square [1z 2z];"#,
            &["unify", "expected"][..],
        ),
        (
            r#"f := @/ &- ["a" "b"];"#,
            &["num", "unify", "expected"][..],
        ),
        (
            r#"f := @/ &- #[.{a:=1z;}];"#,
            &["sequence", "unify", "expected"][..],
        ),
        (
            r#"f := @/ &- [] [];"#,
            &["missing operand", "expected", "unknown"][..],
        ),
    ] {
        reject(source, detail);
    }
}
