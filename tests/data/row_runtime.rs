use super::script::Script;
use std::fs;

#[test]
fn rebuilding_empty_open_rows_replays_order_through_a_first_class_call() {
    let fixture = Script::new(
        "flux-row-runtime",
        "main.flx",
        r#"
<< "csv";
<< "df";
build(t:#.{ value:q; | ~r }):#.{ value:q; | ~r } := #: :# t;
relay(t:#.{ value:q; | ~r }):#.{ value:q; | ~r } := @ &build t;
squares(t:#.{ value:q; | ~r }):#.{ value:q; squared:q; | ~r } :=
    df.extend .{ squared := ^ .value t 2n; } t;
empty:#.{ id:s; value:q; flag:b; } := #[];
!{ ^{
    ^? ! csv.write "out.csv" (relay empty);
    ^? ! csv.write "extended.csv" (squares empty);
    ()
} };
"#,
    );
    for cli in [["check"].as_slice(), ["run-hir"].as_slice(), [].as_slice()] {
        let output = fixture.run(cli.first().copied());
        assert!(
            output.status.success(),
            "{cli:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        for (filename, header) in [
            ("out.csv", "id,value,flag\n"),
            ("extended.csv", "id,value,flag,squared\n"),
        ] {
            let result = fixture.path().join(filename);
            if cli == ["check"] {
                assert!(!result.exists());
            } else {
                assert_eq!(
                    fs::read_to_string(&result).unwrap().replace("\r\n", "\n"),
                    header
                );
                fs::remove_file(result).unwrap();
            }
        }
    }
}
