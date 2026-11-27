use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        loop {
            let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "flux-aesthetics-{}-{nonce}-{id}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot create test directory: {error}"),
            }
        }
    }
    fn run(&self, script: &str) -> Output {
        fs::write(self.0.join("test.flx"), script).unwrap();
        Command::new(env!("CARGO_BIN_EXE_flux"))
            .current_dir(&self.0)
            .arg("test.flx")
            .output()
            .unwrap()
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn bytes_text(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).unwrap()
}

#[test]
fn revised_examples_run_and_do_not_clobber() {
    let files = Temp::new();
    let pipeline = files.run(include_str!("../examples/pipeline.flx"));
    assert!(
        pipeline.status.success(),
        "{}",
        bytes_text(&pipeline.stderr)
    );
    assert_eq!(bytes_text(&pipeline.stdout), "337965\n");
    fs::create_dir(files.0.join("examples")).unwrap();
    fs::write(
        files.0.join("examples/monitoring.csv"),
        include_str!("../examples/monitoring.csv"),
    )
    .unwrap();
    let source = include_str!("../examples/monitoring.flx");
    let output = files.run(source);
    assert!(output.status.success(), "{}", bytes_text(&output.stderr));
    let out_path = files.0.join("summary.csv");
    let committed = fs::read_to_string(&out_path).unwrap();
    assert_eq!(
        committed,
        "station,count,mean,median,sd\r\nnorth,2,15,15,3\r\n"
    );
    let duplicate = files.run(source);
    assert!(!duplicate.status.success());
    assert_eq!(fs::read_to_string(&out_path).unwrap(), committed);
}

#[test]
fn column_construction_validates_shape_and_preserves_order() {
    let files = Temp::new();
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
    let output = files.run(base);
    assert!(output.status.success(), "{}", bytes_text(&output.stderr));
    assert_eq!(bytes_text(&output.stdout), "[12 22]\n[north south]\n");
    let bad = base.replace("[10 20]", "[10]");
    let error = files.run(&bad);
    assert!(!error.status.success());
    assert!(bytes_text(&error.stderr).to_lowercase().contains("length"));
}
