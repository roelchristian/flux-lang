//! File-backed source module graph for the single typed Flux frontend.
use crate::{
    builtins,
    diagnostic::{Category, Diagnostic, Related, Span},
    parser::{self, ImportedModule, ModuleMember, Program, Stmt},
    semantic::ModuleInput,
    Error,
};
use std::{
    collections::HashMap,
    fmt, fs,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

const MAX_DEPTH: usize = 128;
const MAX_FILES: usize = 1024;
const MAX_SOURCE_BYTES: usize = 8 * 1024 * 1024;
const MAX_GRAPH_BYTES: usize = 32 * 1024 * 1024;

/// Original file text and its position in the graph's virtual byte-address space.
#[derive(Clone, Debug)]
pub struct SourceFile {
    pub path: PathBuf,
    pub base: usize,
    pub text: Arc<str>,
    label: String,
}
#[derive(Clone, Debug, Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}
impl SourceMap {
    pub fn files(&self) -> &[SourceFile] {
        &self.files
    }
    fn file_at(&self, global: usize) -> Option<&SourceFile> {
        let i = self
            .files
            .partition_point(|file| file.base <= global)
            .checked_sub(1)?;
        let file = &self.files[i];
        (global - file.base <= file.text.len()).then_some(file)
    }
    /// Resolve a global byte offset, including a file's own EOF, to original source.
    pub fn locate(&self, global: usize) -> Option<(&str, &str, usize)> {
        let file = self.file_at(global)?;
        Some((&file.label, &file.text, global - file.base))
    }
    pub fn diagnostic(&self, error: &Error) -> String {
        error.diagnostic_with(|offset| self.locate(offset))
    }
}

/// Parsing alone does not execute or typecheck initializers. Use
/// `semantic::prepare_modules` to check this graph and produce one typed HIR.
#[derive(Debug)]
pub struct LoadedProgram {
    pub program: Program,
    pub sources: SourceMap,
    pub root: PathBuf,
    pub module_inputs: Vec<ModuleInput>,
    root_text: Arc<str>,
}
impl LoadedProgram {
    pub fn root_source(&self) -> &str {
        &self.root_text
    }
    pub fn diagnostic(&self, error: &Error) -> String {
        self.sources.diagnostic(error)
    }
}

#[derive(Debug)]
pub struct LoadError {
    pub error: Error,
    pub sources: SourceMap,
    pub path: PathBuf,
}
impl LoadError {
    pub fn diagnostic(&self) -> String {
        if self.sources.locate(self.error.start).is_some() {
            self.sources.diagnostic(&self.error)
        } else {
            format!("{}: {}", self.path.display(), self.error.message)
        }
    }
}
impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.diagnostic())
    }
}
impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

fn issue(at: usize, message: impl Into<String>) -> Error {
    Diagnostic::new(Category::Resolution, Span::at(at), message).into_error()
}

#[derive(Debug)]
struct LoadedFile {
    path: PathBuf,
    program: Program,
    members: Vec<ModuleMember>,
    deps: Vec<PathBuf>,
}
struct Active {
    path: PathBuf,
    import: Option<usize>,
}
#[derive(Default)]
struct Loader {
    sources: SourceMap,
    next_base: usize,
    graph_bytes: usize,
    active: Vec<Active>,
    files: Vec<LoadedFile>,
    complete: HashMap<PathBuf, usize>,
    root_text: Option<Arc<str>>,
}

/// Read a bounded, regular UTF-8 source fragment without parsing or executing it.
pub fn read_fragment(path: impl AsRef<Path>) -> Result<(PathBuf, String), LoadError> {
    let requested = path.as_ref();
    let root = fs::canonicalize(requested).map_err(|e| LoadError {
        error: issue(0, format!("cannot resolve source fragment: {e}")),
        sources: SourceMap::default(),
        path: requested.to_owned(),
    })?;
    let text = read_source(&root, 0).map_err(|error| LoadError {
        error,
        sources: SourceMap::default(),
        path: root.clone(),
    })?;
    Ok((root, text.to_string()))
}

/// Load a root program and its explicit local source imports.
pub fn load(path: impl AsRef<Path>) -> Result<LoadedProgram, LoadError> {
    let path = path.as_ref();
    let mut loader = Loader::default();
    let begin =
        || fs::canonicalize(path).map_err(|e| issue(0, format!("cannot resolve root file: {e}")));
    let result = begin().and_then(|root| {
        let root_index = loader.visit(root.clone(), None, None, None)?;
        Ok((root, root_index))
    });
    finish(path, loader, result)
}

/// Load a source string with an explicit path identity and import base. The
/// root file itself is not opened; only its declared dependencies are.
pub fn load_virtual(path: impl AsRef<Path>, source: &str) -> Result<LoadedProgram, LoadError> {
    let path = path.as_ref();
    let mut loader = Loader::default();
    let result = virtual_path(path).and_then(|root| {
        let root_index = loader.visit(root.clone(), None, Some(source.into()), None)?;
        Ok((root, root_index))
    });
    finish(path, loader, result)
}

/// A single virtual lexical module assembled from pieces that retain their
/// own source locations and directories for resolving relative imports.
/// This is useful for transactional REPL submissions and loaded declarations.
pub fn load_virtual_parts(
    path: impl AsRef<Path>,
    parts: &[(PathBuf, String)],
) -> Result<LoadedProgram, LoadError> {
    let path = path.as_ref();
    if parts.is_empty() {
        return load_virtual(path, "");
    }
    let mut loader = Loader::default();
    let result = virtual_path(path).and_then(|root| {
        let bytes = parts
            .iter()
            .map(|(_, s)| s.len() + 1)
            .try_fold(0usize, usize::checked_add)
            .filter(|n| *n <= MAX_SOURCE_BYTES + 1)
            .ok_or_else(|| issue(0, "virtual root exceeds source byte limit"))?;
        if parts.len() > MAX_FILES {
            return Err(issue(0, "virtual root source part count limit exceeded"));
        }
        let mut text = String::with_capacity(bytes);
        let base = loader.next_base;
        for (i, (origin, piece)) in parts.iter().enumerate() {
            if i > 0 {
                text.push('\n');
            }
            let origin = virtual_path(origin)?;
            loader.source(origin, Arc::from(piece.as_str()), base + text.len())?;
            text.push_str(piece);
        }
        let idx = loader.visit(root.clone(), None, Some(Arc::from(text)), Some(base))?;
        Ok((root, idx))
    });
    finish(path, loader, result)
}

fn module_input(f: &LoadedFile) -> ModuleInput {
    ModuleInput {
        path: f.path.clone(),
        dependencies: f.deps.clone(),
        exports: f.members.clone(),
        nominal_types: f
            .program
            .defs
            .iter()
            .filter_map(|d| match d {
                Stmt::Schema { name, .. } | Stmt::Sum { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect(),
    }
}

fn finish(
    path: &Path,
    mut loader: Loader,
    result: Result<(PathBuf, usize), Error>,
) -> Result<LoadedProgram, LoadError> {
    let (root, i) = match result {
        Ok(x) => x,
        Err(error) => {
            return Err(LoadError {
                error,
                sources: loader.sources,
                path: path.to_owned(),
            })
        }
    };
    debug_assert_eq!(i + 1, loader.files.len());
    let root_text = loader
        .root_text
        .take()
        .expect("successful load records root source");
    let inputs = loader.files.iter().map(module_input).collect();
    let main = loader.files.pop().expect("successful load includes a root");
    let mut program = main.program;
    program.merge_libraries(loader.files.into_iter().map(|f| f.program));

    Ok(LoadedProgram {
        program,
        sources: loader.sources,
        root,
        module_inputs: inputs,
        root_text,
    })
}

fn virtual_path(path: &Path) -> Result<PathBuf, Error> {
    if let Ok(real) = fs::canonicalize(path) {
        return Ok(real);
    }
    let name = path
        .file_name()
        .ok_or_else(|| issue(0, "virtual root requires a file name"))?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::canonicalize(parent)
        .map(|dir| dir.join(name))
        .map_err(|e| {
            issue(
                0,
                format!(
                    "cannot resolve virtual root directory {}: {e}",
                    parent.display()
                ),
            )
        })
}

fn canonical_import(from: &Path, name: &str, at: usize) -> Result<PathBuf, Error> {
    let requested = Path::new(name);
    if name.contains([':', '\\', '\0'])
        || requested.is_absolute()
        || requested
            .components()
            .any(|c| matches!(c, Component::Prefix(_) | Component::RootDir))
        || requested.extension().and_then(|s| s.to_str()) != Some("flx")
    {
        return Err(issue(at,format!("source import {name:?} must be a local relative .flx path; absolute, remote and URL imports are not supported")));
    }
    let parent = from
        .parent()
        .ok_or_else(|| issue(at, "importing file has no directory"))?;
    let path = parent.join(requested);
    fs::canonicalize(&path).map_err(|e| {
        issue(
            at,
            format!(
                "cannot resolve source import {name:?} from {}: {e}",
                from.display()
            ),
        )
    })
}

fn read_source(path: &Path, at: usize) -> Result<Arc<str>, Error> {
    let unreadable =
        |e: std::io::Error| issue(at, format!("cannot read source {}: {e}", path.display()));
    let meta = fs::metadata(path).map_err(unreadable)?;
    if !meta.is_file() {
        return Err(issue(
            at,
            format!("source {} is not a regular file", path.display()),
        ));
    }
    if meta.len() > MAX_SOURCE_BYTES as u64 {
        return Err(issue(
            at,
            format!(
                "source {} exceeds the {} byte limit",
                path.display(),
                MAX_SOURCE_BYTES
            ),
        ));
    }
    let f = fs::File::open(path).map_err(unreadable)?;
    if !f.metadata().map_err(unreadable)?.is_file() {
        return Err(issue(
            at,
            format!("source {} is not a regular file", path.display()),
        ));
    }
    let mut raw = Vec::new();
    f.take(MAX_SOURCE_BYTES as u64 + 1)
        .read_to_end(&mut raw)
        .map_err(unreadable)?;
    if raw.len() > MAX_SOURCE_BYTES {
        return Err(issue(
            at,
            format!(
                "source {} exceeds the {} byte limit",
                path.display(),
                MAX_SOURCE_BYTES
            ),
        ));
    }
    let text = String::from_utf8(raw)
        .map_err(|_| issue(at, format!("source {} is not UTF-8", path.display())))?;
    Ok(Arc::from(text))
}

fn namespace(path: &Path) -> String {
    // Injective path byte encoding, not a hash or discovery-order identifier.
    // The parser separately marks linked names unspellable by source code.
    const HEX: &[u8] = b"0123456789abcdef";
    let bytes = path.as_os_str().as_encoded_bytes();
    let mut tag = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        tag.push(HEX[(b >> 4) as usize] as char);
        tag.push(HEX[(b & 15) as usize] as char);
    }
    tag
}

impl Loader {
    fn source(&mut self, path: PathBuf, text: Arc<str>, at: usize) -> Result<usize, Error> {
        if text.len() > MAX_SOURCE_BYTES {
            return Err(issue(
                at,
                format!(
                    "source {} exceeds the {} byte limit",
                    path.display(),
                    MAX_SOURCE_BYTES
                ),
            ));
        }
        self.graph_bytes = self.graph_bytes.saturating_add(text.len());
        if self.graph_bytes > MAX_GRAPH_BYTES {
            return Err(issue(at, "source module graph exceeds total byte limit"));
        }
        let base = self.next_base;
        // A distinct byte position for each file's EOF, including empty files.
        self.next_base += text.len() + 1;
        let label = path.display().to_string();
        self.sources.files.push(SourceFile {
            path,
            base,
            text,
            label,
        });
        Ok(base)
    }

    fn visit(
        &mut self,
        path: PathBuf,
        import: Option<usize>,
        source: Option<Arc<str>>,
        overlay_base: Option<usize>,
    ) -> Result<usize, Error> {
        if let Some(&index) = self.complete.get(&path) {
            return Ok(index);
        }
        let at = import.unwrap_or(0);
        if let Some(first) = self.active.iter().position(|x| x.path == path) {
            let mut chain = self.active[first..]
                .iter()
                .map(|x| x.path.display().to_string())
                .collect::<Vec<_>>();
            chain.push(path.display().to_string());
            let mut d = Diagnostic::new(
                Category::Resolution,
                Span::at(at),
                format!("source import cycle: {}", chain.join(" -> ")),
            );
            d.related
                .extend(self.active[first + 1..].iter().filter_map(|x| {
                    x.import.map(|i| Related {
                        span: Span::at(i),
                        label: format!("imports {}", x.path.display()),
                    })
                }));
            return Err(d.into_error());
        }
        if self.active.len() >= MAX_DEPTH {
            return Err(issue(at, "source import depth limit exceeded"));
        }
        if self.sources.files.len() >= MAX_FILES {
            return Err(issue(at, "source module file count limit exceeded"));
        }
        let text = match source {
            Some(source) => source,
            None => read_source(&path, at)?,
        };
        let base = if let Some(base) = overlay_base {
            base
        } else {
            self.source(path.clone(), text.clone(), at)?
        };
        if import.is_none() {
            self.root_text = Some(text.clone());
        }
        self.active.push(Active {
            path: path.clone(),
            import,
        });
        let mut dependencies = Vec::new();
        let mut interfaces = Vec::new();
        for import in parser::discover_imports(&text, base)? {
            if builtins::module_items(&import.path).is_some() {
                continue;
            }
            let from = self
                .sources
                .file_at(import.start)
                .map_or(path.as_path(), |src| src.path.as_path());
            let dep = canonical_import(from, &import.path, import.start)?;
            let index = self.visit(dep.clone(), Some(import.start), None, None)?;
            if !dependencies.contains(&dep) {
                dependencies.push(dep);
            }
            interfaces.push(ImportedModule {
                path: import.path,
                members: self.files[index].members.clone(),
            });
        }
        let tag = namespace(&path);
        let program = parser::parse_module(&text, base, Some(&tag), &interfaces)?;
        if import.is_some() && program.entry.is_some() {
            return Err(issue(
                program.entry_start.unwrap_or(base),
                format!(
                    "source library {} cannot contain an executable script entry",
                    path.display()
                ),
            ));
        }
        let members = program.module_members();
        self.active.pop();
        let index = self.files.len();
        self.files.push(LoadedFile {
            path: path.clone(),
            program,
            members,
            deps: dependencies,
        });
        self.complete.insert(path, index);
        Ok(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic::{self, Mode, Policy};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            loop {
                let dir = std::env::temp_dir().join(format!(
                    "flux-043-loader-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                match fs::create_dir(&dir) {
                    Ok(()) => return Self(dir),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(e) => panic!("{e}"),
                }
            }
        }
        fn put(&self, path: &str, source: &str) -> PathBuf {
            let path = self.0.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, source).unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn local_diamond_has_one_canonical_initializer_and_typed_root() {
        let fx = Fixture::new();
        fx.put("shared.flx", "k:n := 4n; >> (k);");
        fx.put(
            "left.flx",
            r#" << "./shared.flx" : s; value:n := s.k; >> (value); "#,
        );
        fx.put(
            "sub/right.flx",
            r#" << "../shared.flx" : s; value:n := + s.k 1n; >> (value); "#,
        );
        let main = fx.put(
            "main.flx",
            r#"
            << "./left.flx" : a;
            << "./sub/right.flx" : b;
            !{ + a.value b.value };
        "#,
        );
        let loaded = load(main).unwrap();
        assert_eq!(loaded.module_inputs.len(), 4);
        assert_eq!(loaded.sources.files().len(), 4);
        assert_eq!(loaded.program.defs.len(), 3);
        let ready = semantic::prepare_modules(
            &loaded.program,
            Mode::Script,
            Policy::TypedOnly,
            &loaded.module_inputs,
        )
        .unwrap();
        assert_eq!(ready.run().unwrap().status, 9);
    }

    #[test]
    fn original_file_paths_report_cycles_and_executable_library() {
        let fx = Fixture::new();
        let a = fx.put("a.flx", "\n<< \"./b.flx\";");
        fx.put("b.flx", "\n\n<< \"./a.flx\";");
        let cycle = load(&a).unwrap_err().diagnostic();
        assert!(
            cycle.contains("b.flx:3:")
                && cycle.contains("source import cycle")
                && cycle.contains("a.flx:2:"),
            "{cycle}"
        );

        fx.put("b.flx", "\n!{ (); };");
        let entry = load(&a).unwrap_err().diagnostic();
        assert!(
            entry.contains("b.flx:2:") && entry.contains("executable script entry"),
            "{entry}"
        );
    }

    #[test]
    fn rejects_remote_absolute_and_nonregular_source_imports() {
        let fx = Fixture::new();
        let exported = fx.put("a.flx", "x:n := 1n; >> (x);");
        let main = fx.0.join("root.flx");
        for target in [
            "https://example.invalid/a.flx".into(),
            exported.display().to_string(),
        ] {
            let source = format!("<< \"{target}\" : a;");
            let err = load_virtual(&main, &source).unwrap_err().diagnostic();
            assert!(err.contains("local relative .flx path"), "{err}");
        }
        fs::create_dir(fx.0.join("dir.flx")).unwrap();
        let err = load_virtual(main, r#" << "./dir.flx" : d; "#)
            .unwrap_err()
            .diagnostic();
        assert!(err.contains("not a regular file"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn canonical_symlink_imports_are_parsed_once() {
        let fx = Fixture::new();
        let real = fx.put("lib.flx", "x:n := 1n; >> (x);");
        std::os::unix::fs::symlink(real, fx.0.join("other.flx")).unwrap();
        let main = fx.put(
            "main.flx",
            r#"
            << "lib.flx" : a;
            << "other.flx" : b;
            !{ + a.x b.x };
        "#,
        );
        let loaded = load(main).unwrap();
        assert_eq!(loaded.sources.files().len(), 2);
        assert_eq!(loaded.module_inputs.last().unwrap().dependencies.len(), 1);
        assert_eq!(loaded.program.defs.len(), 1);
    }

    #[test]
    fn virtual_parts_resolve_identical_spelling_from_each_origins_directory() {
        let fx = Fixture::new();
        fx.put("one/util.flx", "val:n := 4n; >> (val);");
        fx.put("two/util.flx", "val:n := 5n; >> (val);");
        let main = fx.0.join("virtual.flx");
        let a = fx.0.join("one/chunk.flx");
        let b = fx.0.join("two/chunk.flx");
        let parts = [
            (a, r#" << "./util.flx" : a; "#.into()),
            (b, r#" << "./util.flx" : b; !{ + a.val b.val }; "#.into()),
        ];
        let loaded = load_virtual_parts(main, &parts).unwrap();
        assert_eq!(loaded.module_inputs.len(), 3);
        assert_eq!(loaded.sources.files().len(), 4);
        let ready = semantic::prepare_modules(
            &loaded.program,
            Mode::Script,
            Policy::TypedOnly,
            &loaded.module_inputs,
        )
        .unwrap();
        assert_eq!(ready.run().unwrap().status, 9);
    }
}
