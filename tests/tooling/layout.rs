use crate::support::{flux, TempDir};
use flux_lang::lexer::lex;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Output,
};

fn write(dir: &Path, name: &str, data: impl AsRef<[u8]>) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, data).expect("write fixture");
    path
}

fn cli(dir: &Path, args: &[&str], path: &Path) -> Output {
    flux()
        .current_dir(dir)
        .args(args)
        .arg(path)
        .output()
        .expect("invoke Flux")
}

fn success(out: Output) -> String {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("text output")
}

fn tokens(s: &str) -> Vec<String> {
    lex(s)
        .expect("lex fixture")
        .into_iter()
        .map(|token| token.text)
        .collect()
}

const MESSY: &str = r#"-- Preserve the comment; &. and { are text here.
square:z->z:=&{@* * . .}; -- attached to the preceding declaration
id:=&.;
!{ ^{
    values:=@: square [2z 3z];
label:=@ id "literal -- spaces  { } &. \"";
^? ! print (label,values);
() } };
"#;

#[test]
fn check_mode_detects_changes_and_never_rewrites() {
    let fixture = TempDir::new("layout");
    let before = MESSY.replace('\n', "\r\n");
    let original = write(fixture.path(), "source.flx", &before);
    let diff = cli(fixture.path(), &["fmt", "--check"], &original);
    assert_eq!(diff.status.code(), Some(1), "{diff:?}");
    assert_eq!(fs::read(&original).unwrap(), before.as_bytes());

    let formatted = success(cli(fixture.path(), &["fmt"], &original));
    assert_ne!(formatted, before);
    assert_eq!(fs::read(&original).unwrap(), before.as_bytes());
    assert!(formatted.ends_with('\n') && !formatted.contains('\r'));

    let canonical = write(fixture.path(), "canonical.flx", &formatted);
    assert!(cli(fixture.path(), &["fmt", "--check"], &canonical)
        .status
        .success());
    assert_eq!(fs::read(&canonical).unwrap(), formatted.as_bytes());
    assert_eq!(
        success(cli(fixture.path(), &["fmt"], &canonical)),
        formatted
    );
}

#[test]
fn preserves_tokens_and_evaluation_across_reformatting() {
    let fixture = TempDir::new("layout");
    let original = write(fixture.path(), "source.flx", MESSY);
    let formatted = success(cli(fixture.path(), &["fmt"], &original));
    assert_eq!(tokens(MESSY), tokens(&formatted));
    assert!(formatted.contains("id := &.;"), "{formatted}");
    assert!(
        formatted.contains("-- Preserve the comment; &. and { are text here."),
        "{formatted}"
    );
    assert!(
        formatted.contains("-- attached to the preceding declaration"),
        "{formatted}"
    );
    let staged = write(fixture.path(), "formatted.flx", formatted);
    for path in [&original, &staged] {
        success(cli(fixture.path(), &["check"], path));
        assert_eq!(
            success(cli(fixture.path(), &["run-hir"], path)),
            "(literal -- spaces  { } &. \", [4 9])\n"
        );
    }
}

#[test]
fn expands_multi_statement_blocks_with_four_space_nesting() {
    let fixture = TempDir::new("layout");
    let source = r#"!{ ^{ x:=2z; y:=+ x 3z; ^? ! print y; () } };"#;
    let original = write(fixture.path(), "source.flx", source);
    let formatted = success(cli(fixture.path(), &["fmt"], &original));
    assert_eq!(tokens(source), tokens(&formatted));
    let lines = formatted.lines().collect::<Vec<_>>();
    assert!(lines.contains(&"!{"), "{formatted}");
    assert!(lines.contains(&"    ^{"), "{formatted}");
    for stmt in ["x := 2z;", "y := + x 3z;", "^? ! print y;"] {
        assert!(
            lines.contains(&format!("        {stmt}").as_str()),
            "{formatted}"
        );
    }
    assert!(
        lines.contains(&"    }") && lines.contains(&"};"),
        "{formatted}"
    );
    let rewritten = write(fixture.path(), "expanded.flx", formatted);
    assert_eq!(
        success(cli(fixture.path(), &["run-hir"], &rewritten)),
        "5\n"
    );
}

#[test]
fn multiline_tacit_and_guarded_cases_keep_nested_structure() {
    let fixture = TempDir::new("layout");
    let source = r#"square:z->z:=&{
-- Compose the arithmetic beside the source comment.
@* * . .
};
measure(x:z):z:=?{ > x 10z : @ square x; _ : x; };
!{ ^= ! print (measure 3z,measure 11z) { _ : (); }; };
"#;
    let original = write(fixture.path(), "source.flx", source);
    let formatted = success(cli(fixture.path(), &["fmt"], &original));
    assert_eq!(tokens(source), tokens(&formatted));
    assert!(formatted
        .lines()
        .any(|line| line.starts_with("    -- Compose")));
    assert!(
        formatted.lines().any(|line| line.starts_with("    @* *")),
        "{formatted}"
    );
    let rewritten = write(fixture.path(), "formatted.flx", formatted);
    assert_eq!(
        success(cli(fixture.path(), &["run-hir"], &rewritten)),
        "(3, 121)\n"
    );
}

#[test]
fn preserves_comment_anchors_literals_and_adjacent_signs_in_lf_and_crlf() {
    // Lexing filters out comments; check each comment's original gap separately.
    fn anchors(source: &str) -> Vec<(usize, String)> {
        let lexed = lex(source).unwrap();
        let mut result = Vec::new();
        let mut previous = 0;
        for (index, next) in lexed
            .iter()
            .map(|t| t.start)
            .chain(std::iter::once(source.len()))
            .enumerate()
        {
            for line in source[previous..next].lines() {
                let line = line.trim_matches([' ', '\t']);
                if line.starts_with("--") {
                    result.push((index, line.to_owned()));
                }
            }
            if let Some(token) = lexed.get(index) {
                previous = token.end;
            }
        }
        result
    }

    let fixture = TempDir::new("layout");
    let source = r#"--- Difference
difference := - -2z 3z; -- a negative operand, not a comment
a := .{
    note := "-- literal; [] {}";
    -- Stays above the record's closing margin.
};
!{ ^{ ^? ! print (difference,.note a); () } };
-- End.
"#;
    for (name, bytes) in [
        ("lf.flx", source.to_owned()),
        ("crlf.flx", source.replace('\n', "\r\n")),
    ] {
        let path = write(fixture.path(), name, &bytes);
        let before = cli(fixture.path(), &["fmt", "--check"], &path);
        assert_eq!(before.status.code(), Some(1));
        assert!(before.stdout.is_empty());
        assert_eq!(fs::read(&path).unwrap(), bytes.as_bytes());
        let formatted = success(cli(fixture.path(), &["fmt"], &path));
        assert_eq!(tokens(source), tokens(&formatted));
        assert_eq!(anchors(source), anchors(&formatted));
        assert!(formatted.contains("\n    -- Stays above"), "{formatted}");
        let formatted_path = write(fixture.path(), "canonical.flx", &formatted);
        assert!(cli(fixture.path(), &["fmt", "--check"], &formatted_path)
            .status
            .success());
        assert_eq!(
            success(cli(fixture.path(), &["run-hir"], &formatted_path)),
            "(-5, -- literal; [] {})\n"
        );
    }
}

#[test]
fn keeps_small_tacit_forms_compact_and_divides_long_forks_into_branches() {
    let fixture = TempDir::new("layout");
    let original = write(
        fixture.path(),
        "tacit.flx",
        r#"sq := &{ @* * . . };
point :: { x:q; y:q; };
mag:point->q := &{
    @* +
        @. sq .x
        @. sq .y
};
"#,
    );
    let formatted = success(cli(fixture.path(), &["fmt"], &original));
    assert!(formatted.contains("sq := &{ @* * . . };"), "{formatted}");
    assert!(
        formatted.contains("    @* +\n        @. sq .x\n        @. sq .y"),
        "{formatted}"
    );
    let path = write(fixture.path(), "formatted.flx", &formatted);
    assert_eq!(success(cli(fixture.path(), &["fmt"], &path)), formatted);
}
