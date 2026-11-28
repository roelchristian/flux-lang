use crate::support::{flux, TempDir};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Output, Stdio},
};

pub(super) struct Fixture(TempDir);

impl Fixture {
    pub(super) fn new(source: &str) -> Self {
        let fixture = Self(TempDir::new("flux-language"));
        fixture.write("main.flx", source);
        fixture
    }

    pub(super) fn path(&self) -> &Path {
        self.0.path()
    }

    pub(super) fn write(&self, filename: &str, source: &str) {
        fs::write(self.path().join(filename), source).expect("module fixture");
    }

    pub(super) fn cli(&self, mode: &str, input: &str) -> Output {
        let mut command = flux();
        command.current_dir(self.path());
        if !mode.is_empty() {
            command.arg(mode);
        }
        let mut child = command
            .arg("main.flx")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("execute Flux");
        if !input.is_empty() {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(input.as_bytes())
                .unwrap();
        }
        child.wait_with_output().expect("wait for Flux")
    }

    pub(super) fn accepts(&self, input: &str, expected: &str) {
        for mode in ["check", "hir", "run-hir", ""] {
            let out = self.cli(
                mode,
                if mode == "check" || mode == "hir" {
                    ""
                } else {
                    input
                },
            );
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(
                out.status.success() && stderr.is_empty(),
                "{mode}: {stderr}"
            );
            match mode {
                "check" => assert!(stdout.is_empty(), "{stdout}"),
                "hir" => assert!(stdout.contains("HirProgram"), "{stdout}"),
                _ => assert_eq!(stdout, expected, "{mode}"),
            }
        }
    }
}

pub(super) fn rejects(source: &str, words: &[&str]) {
    let out = Fixture::new(source).cli("check", "");
    let error = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success() && out.stdout.is_empty(), "{source}");
    assert!(
        words.iter().all(|word| error.contains(word)),
        "expected {words:?}: {error}"
    );
}
