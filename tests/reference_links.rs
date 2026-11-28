//! Keep the single active language reference attached to the executable corpus.
use std::{collections::BTreeSet, fs, path::Path};

#[test]
fn canonical_reference_links_existing_registered_examples() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root.join("examples/manifest.json")).unwrap())
            .unwrap();
    let examples = manifest
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            root.join(entry["entry"].as_str().unwrap())
                .canonicalize()
                .unwrap()
        })
        .collect::<BTreeSet<_>>();
    let reference = Path::new("docs/specification.md");
    let mut linked = BTreeSet::new();
    let mut documents = vec![
        root.join("README.md"),
        root.join("CHANGELOG.md"),
        root.join("examples/README.md"),
    ];
    documents.extend(
        fs::read_dir(root.join("docs"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "md")),
    );
    for path in documents {
        let path = path.strip_prefix(root).unwrap();
        let content = fs::read_to_string(root.join(path)).unwrap();
        for line in content.lines() {
            for rest in line.split("](").skip(1) {
                let Some((destination, _)) = rest.split_once(')') else {
                    continue;
                };
                if destination.contains("://") {
                    continue;
                }
                let target = destination.split('#').next().unwrap();
                if target.is_empty() {
                    continue;
                }
                let resolved = root.join(path.parent().unwrap()).join(target);
                assert!(resolved.exists(), "{path:?}: missing target {target}");
                if path == reference && target.ends_with(".flx") {
                    let example = resolved.canonicalize().unwrap();
                    assert!(examples.contains(&example), "unregistered example {target}");
                    linked.insert(example);
                }
            }
        }
    }
    assert!(
        linked.len() >= 12,
        "the reference should ground its rules in checked examples"
    );
}
