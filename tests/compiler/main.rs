#[path = "../support/mod.rs"]
mod support;

mod builtin_coverage;
mod hir_validation;
mod inference;
mod match_hir;
mod modules;
mod regressions;
mod syntax_identity;

use std::{fs, process::Output};
use support::{flux, TempDir};

fn script(prefix: &str, source: &str) -> TempDir {
    let dir = TempDir::new(prefix);
    fs::write(dir.join("test.flx"), source).expect("write compiler test script");
    dir
}

fn run_script(dir: &TempDir, args: &[&str]) -> Output {
    flux()
        .current_dir(dir.path())
        .args(args)
        .arg("test.flx")
        .output()
        .expect("invoke Flux CLI")
}

#[cfg(feature = "legacy-oracle")]
mod legacy_api;
