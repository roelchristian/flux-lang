use crate::support::{flux, TempDir};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

pub(super) struct Script {
    directory: TempDir,
    entry: &'static str,
}

impl Script {
    pub(super) fn new(prefix: &str, entry: &'static str, source: &str) -> Self {
        let script = Self {
            directory: TempDir::new(prefix),
            entry,
        };
        script.write(entry, source);
        script
    }

    pub(super) fn path(&self) -> &Path {
        self.directory.path()
    }

    pub(super) fn write(&self, filename: &str, source: &str) {
        fs::write(self.path().join(filename), source).expect("write fixture");
    }

    pub(super) fn command(&self, mode: Option<&str>) -> Command {
        let mut command = flux();
        command.current_dir(self.path()).args(mode).arg(self.entry);
        command
    }

    pub(super) fn run(&self, mode: Option<&str>) -> Output {
        self.command(mode).output().expect("invoke Flux")
    }

    pub(super) fn check_hir(&self) {
        let out = self.run(Some("check"));
        assert!(
            out.status.success() && out.stdout.is_empty(),
            "check: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let out = self.run(Some("hir"));
        assert!(
            out.status.success() && String::from_utf8_lossy(&out.stdout).contains("HirProgram"),
            "hir: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    pub(super) fn accepts(&self, expected: &str) {
        for mode in [Some("check"), Some("hir"), Some("run-hir"), None] {
            let out = self.run(mode);
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(
                out.status.success() && stderr.is_empty(),
                "{mode:?}: {stderr}"
            );
            match mode {
                Some("check") => assert!(stdout.is_empty(), "{stdout}"),
                Some("hir") => assert!(stdout.contains("HirProgram"), "{stdout}"),
                _ => assert_eq!(stdout, expected, "{mode:?}"),
            }
        }
    }
}

pub(super) fn rejects(script: &Script, source: &str, words: &[&str]) {
    let out = script.run(Some("check"));
    let error = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success() && out.stdout.is_empty(), "{source}");
    assert!(
        words.iter().all(|word| error.contains(word)),
        "expected {words:?}: {error}"
    );
}
