use crate::support::{flux, TempDir};
use std::{fs, path::Path, process::Output};

fn run(dir: &Path, script: &str) -> Output {
    fs::write(dir.join("test.flx"), script).unwrap();
    flux().current_dir(dir).arg("test.flx").output().unwrap()
}

fn bytes_text(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).unwrap()
}

#[test]
fn revised_examples_run_and_do_not_clobber() {
    let files = TempDir::new("aesthetics");
    let pipeline = run(
        files.path(),
        include_str!("../fixtures/pre-corpus/pipeline.flx"),
    );
    assert!(
        pipeline.status.success(),
        "{}",
        bytes_text(&pipeline.stderr)
    );
    assert_eq!(bytes_text(&pipeline.stdout), "337965\n");
    fs::create_dir(files.join("examples")).unwrap();
    fs::write(
        files.join("examples/monitoring.csv"),
        include_str!("../fixtures/pre-corpus/monitoring.csv"),
    )
    .unwrap();
    let source = include_str!("../fixtures/pre-corpus/monitoring.flx");
    let output = run(files.path(), source);
    assert!(output.status.success(), "{}", bytes_text(&output.stderr));
    let out_path = files.join("summary.csv");
    let committed = fs::read_to_string(&out_path).unwrap();
    assert_eq!(
        committed,
        "station,count,mean,median,sd\r\nnorth,2,15,15,3\r\n"
    );
    let duplicate = run(files.path(), source);
    assert!(!duplicate.status.success());
    assert_eq!(fs::read_to_string(&out_path).unwrap(), committed);
}

#[test]
fn column_construction_validates_shape_and_preserves_order() {
    let files = TempDir::new("aesthetics");
    let base = r#"
<< "io";
<< "df";
!{
    t := #: .{ value := [10 20]; city := ["north" "south"]; };
    u := df.extend .{ amount := + .value t 2; } t;
    ^= ! io.print .amount u { _ : (); };
    ^= ! io.print .city u { _ : (); };
};
"#;
    let output = run(files.path(), base);
    assert!(output.status.success(), "{}", bytes_text(&output.stderr));
    assert_eq!(bytes_text(&output.stdout), "[12 22]\n[north south]\n");
    let bad = base.replace("[10 20]", "[10]");
    let error = run(files.path(), &bad);
    assert!(!error.status.success());
    assert!(bytes_text(&error.stderr).to_lowercase().contains("length"));
}
