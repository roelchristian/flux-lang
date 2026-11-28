use crate::support::{flux, TempDir};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
    process::{Output, Stdio},
};

fn relative(path: &str) -> &Path {
    let path = Path::new(path);
    assert!(
        !path.as_os_str().is_empty()
            && path
                .components()
                .all(|c| matches!(c, Component::Normal(_) | Component::CurDir)),
        "manifest paths must remain within the fixture tree: {path:?}"
    );
    path
}

fn walk(dir: &Path, visit: &mut impl FnMut(&Path, bool)) {
    for file in fs::read_dir(dir).unwrap() {
        let file = file.unwrap();
        let path = file.path();
        let ty = file.file_type().unwrap();
        assert!(!ty.is_symlink(), "symlink in example corpus: {path:?}");
        visit(&path, ty.is_dir());
        if ty.is_dir() {
            walk(&path, visit);
        }
    }
}

fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    walk(dir, &mut |path, is_dir| {
        if !is_dir {
            files.insert(
                path.strip_prefix(dir).unwrap().into(),
                fs::read(path).unwrap(),
            );
        }
    });
    files
}

fn verify_files(dir: &Path, expected: &BTreeMap<PathBuf, Vec<u8>>, context: &str) {
    let actual = snapshot(dir);
    let changed: BTreeSet<_> = actual
        .keys()
        .chain(expected.keys())
        .filter(|file| actual.get(*file) != expected.get(*file))
        .collect();
    assert!(
        changed.is_empty(),
        "{context}: unexpected changes to {changed:?}"
    );
}

fn run(dir: &Path, spec: &Value, mode: &str) -> Output {
    let mut cmd = flux();
    cmd.current_dir(dir.join(relative(spec["cwd"].as_str().unwrap_or("."))));
    if !mode.is_empty() {
        cmd.arg(mode);
    }
    cmd.arg(dir.join(relative(spec["entry"].as_str().unwrap())));
    if mode != "check" {
        if let Some(args) = spec["args"].as_array().filter(|args| !args.is_empty()) {
            cmd.arg("--")
                .args(args.iter().map(|arg| arg.as_str().unwrap()));
        }
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if mode != "check" {
        let input = spec["stdin"].as_str().unwrap_or("");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    }
    child.wait_with_output().unwrap()
}

fn verify(output: &Output, expected: &Value, context: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        output.status.code().map(i64::from),
        Some(expected["status"].as_i64().unwrap_or(0)),
        "{context}: {stderr}"
    );
    if let Some(expected) = expected["stdout"].as_str() {
        assert_eq!(stdout.replace("\r\n", "\n"), expected, "{context}");
    }
    if let Some(fragment) = expected["stderr_contains"].as_str() {
        assert!(
            stderr.contains(fragment),
            "{context}: wanted {fragment:?}, got {stderr:?}"
        );
    } else {
        assert!(stderr.is_empty(), "{context}: {stderr}");
    }
}

#[test]
fn every_public_example_checks_and_runs_with_its_manifest_contract() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest: Vec<Value> =
        serde_json::from_str(include_str!("../../examples/manifest.json")).unwrap();
    let mut all = BTreeSet::new();
    walk(&repo.join("examples"), &mut |path, _| {
        if path.extension().is_some_and(|e| e == "flx") {
            all.insert(path.canonicalize().unwrap());
        }
    });
    // Include library-only modules as well as executable entry points.
    // All tracked Flux source uses LF, including Windows checkouts.
    for path in &all {
        let out = flux().args(["fmt", "--check"]).arg(path).output().unwrap();
        assert!(
            out.status.success() && out.stdout.is_empty() && out.stderr.is_empty(),
            "noncanonical example {path:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let mut covered = BTreeSet::new();
    let mut entries = BTreeSet::new();
    let example_files = snapshot(&repo.join("examples"));
    for spec in manifest {
        let entry = relative(spec["entry"].as_str().unwrap());
        assert!(entry.starts_with("examples") && entries.insert(entry.to_owned()));
        assert!(
            spec["stdout"].is_string(),
            "missing expected stdout: {entry:?}"
        );
        let graph = flux_lang::modules::load(repo.join(entry))
            .unwrap_or_else(|e| panic!("{}", e.diagnostic()));
        assert!(
            graph.program.entry.is_some(),
            "library listed as executable: {entry:?}"
        );
        covered.extend(
            graph
                .sources
                .files()
                .iter()
                .map(|f| f.path.canonicalize().unwrap()),
        );

        for mode in ["", "run-hir"] {
            let dir = TempDir::new("corpus");
            for (name, contents) in &example_files {
                let file = dir.join("examples").join(name);
                fs::create_dir_all(file.parent().unwrap()).unwrap();
                fs::write(file, contents).unwrap();
            }
            let context = format!("{entry:?} [{mode}]");
            let initial = snapshot(dir.path());
            verify(
                &run(dir.path(), &spec, "check"),
                &serde_json::json!({"stdout": ""}),
                &context,
            );
            verify_files(dir.path(), &initial, &context);
            verify(&run(dir.path(), &spec, mode), &spec, &context);
            let mut expected_files = initial;
            if let Some(files) = spec["files"].as_object() {
                let cwd = relative(spec["cwd"].as_str().unwrap_or("."));
                for (name, expected) in files {
                    let expected =
                        fs::read(repo.join(relative(expected.as_str().unwrap()))).unwrap();
                    let generated = cwd.join(relative(name));
                    let bytes = fs::read(dir.join(&generated)).unwrap();
                    let canonical =
                        |bytes: &[u8]| std::str::from_utf8(bytes).unwrap().replace("\r\n", "\n");
                    assert_eq!(canonical(&bytes), canonical(&expected), "{context}: {name}");
                    assert!(expected_files
                        .insert(
                            generated
                                .components()
                                .filter_map(|c| match c {
                                    Component::Normal(part) => Some(part),
                                    _ => None,
                                })
                                .collect(),
                            bytes
                        )
                        .is_none());
                }
            }
            verify_files(dir.path(), &expected_files, &context);
            if let Some(rerun) = spec.get("rerun") {
                verify(&run(dir.path(), &spec, mode), rerun, &context);
                verify_files(dir.path(), &expected_files, &context);
            }
        }
    }
    assert_eq!(
        covered, all,
        "unverified executable or unreferenced library"
    );
}
