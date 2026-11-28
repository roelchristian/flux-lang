use std::{
    fs,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

struct Fixture(std::path::PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn rebuilding_empty_open_rows_replays_order_through_a_first_class_call() {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let fixture = loop {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("flux-row-runtime-{}-{n}", std::process::id()));
        match fs::create_dir(&path) {
            Ok(()) => break Fixture(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => panic!("{e}"),
        }
    };
    fs::write(
        fixture.0.join("main.flx"),
        r#"
<< "csv";
<< "df";
build(t:#.{ value:q; | 'r }):#.{ value:q; | 'r } := #: :# t;
relay(t:#.{ value:q; | 'r }):#.{ value:q; | 'r } := @ &build t;
squares(t:#.{ value:q; | 'r }):#.{ value:q; squared:q; | 'r } :=
    df.extend .{ squared := ^ .value t 2n; } t;
empty:#.{ id:s; value:q; flag:b; } := #[];
!{ ^{
    ^? ! csv.write "out.csv" (relay empty);
    ^? ! csv.write "extended.csv" (squares empty);
    ()
} };
"#,
    )
    .unwrap();
    for cli in [["check"].as_slice(), ["run-hir"].as_slice(), [].as_slice()] {
        let output = Command::new(env!("CARGO_BIN_EXE_flux"))
            .current_dir(&fixture.0)
            .args(cli)
            .arg("main.flx")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{cli:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        for (filename, header) in [
            ("out.csv", "id,value,flag\n"),
            ("extended.csv", "id,value,flag,squared\n"),
        ] {
            let result = fixture.0.join(filename);
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
