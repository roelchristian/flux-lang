//! Single production semantic frontend and typed HIR execution.
use crate::{
    check,
    diagnostic::{Category, Diagnostic, Span},
    eval, hir, hir_eval, infer,
    parser::{Expr, MemberKind, ModuleMember, Program, Stmt, TypeAnnotation, TypeAnnotationKind},
    resolve::{self, Resolution, SymbolId},
    types::{NominalId, Prim, Row, Scheme, Type},
    Error,
};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Script,
    Inspection,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Policy {
    /// API alias retained for older callers. Does not enable a legacy backend.
    Compatible,
    TypedOnly,
}

/// One file's parsed public surface and direct, canonical source dependencies.
#[derive(Clone, Debug)]
pub struct ModuleInput {
    pub path: PathBuf,
    pub dependencies: Vec<PathBuf>,
    pub exports: Vec<ModuleMember>,
    /// Link names of all nominal declarations owned by this module, including
    /// private declarations. Imported names do not belong here.
    pub nominal_types: Vec<String>,
}

/// A checked member of a module interface. Its symbol and nominal identity are
/// scoped to this checked linked program, not interchangeable with another
/// compilation's numeric identifiers.
#[derive(Clone, Debug)]
pub struct CheckedModuleMember {
    pub member: ModuleMember,
    pub symbol: SymbolId,
    pub scheme: Scheme,
    pub nominal: Option<NominalId>,
    pub row: Option<Row>,
    pub constructor: Option<infer::Constructor>,
}

#[derive(Clone, Debug)]
pub struct ModuleInterface {
    pub path: PathBuf,
    pub dependencies: Vec<PathBuf>,
    pub exports: Vec<CheckedModuleMember>,
}

#[derive(Debug)]
pub struct Prepared<'a> {
    hir: hir::HirProgram,
    inspection: Option<check::Inspection>,
    module_interfaces: Vec<ModuleInterface>,
    source: std::marker::PhantomData<&'a Program>,
}

impl Prepared<'_> {
    pub fn hir(&self) -> Option<&hir::HirProgram> {
        Some(&self.hir)
    }

    /// Retained for callers of the previous API. Production plans are always typed.
    pub fn migration_reason(&self) -> Option<&str> {
        None
    }

    pub fn module_interfaces(&self) -> &[ModuleInterface] {
        &self.module_interfaces
    }
    pub fn inspection(&self) -> Option<&check::Inspection> {
        self.inspection.as_ref()
    }

    pub fn run(&self) -> Result<eval::Output, Error> {
        hir_eval::run_prevalidated(&self.hir)
    }

    pub fn run_with_output(&self, sink: &mut dyn std::io::Write) -> Result<eval::Output, Error> {
        self.run_with_io(&mut crate::FsInput, sink)
    }

    pub fn run_with_io(
        &self,
        input: &mut dyn crate::RuntimeInput,
        output: &mut dyn std::io::Write,
    ) -> Result<eval::Output, Error> {
        self.run_with_host(input, &mut std::io::empty(), &[], output)
    }

    pub fn run_with_host(
        &self,
        input: &mut dyn crate::RuntimeInput,
        stdin: &mut dyn std::io::BufRead,
        arguments: &[String],
        output: &mut dyn std::io::Write,
    ) -> Result<eval::Output, Error> {
        hir_eval::run_with_host_prevalidated(&self.hir, input, stdin, arguments, output)
    }

    pub fn view_with_host(
        &self,
        input: &mut dyn crate::RuntimeInput,
        stdin: &mut dyn std::io::BufRead,
        arguments: &[String],
        output: &mut dyn std::io::Write,
    ) -> Result<eval::ReplOutput, Error> {
        hir_eval::view_with_host_prevalidated(&self.hir, input, stdin, arguments, output)
    }

    pub fn view_with_output(
        &self,
        sink: &mut dyn std::io::Write,
    ) -> Result<eval::ReplOutput, Error> {
        self.view_with_host(&mut crate::FsInput, &mut std::io::empty(), &[], sink)
    }

    pub fn view(&self) -> Result<eval::ReplOutput, Error> {
        hir_eval::view_entry_prevalidated(&self.hir)
    }
}

/// Source-facing diagnostics never expose the parser's unspellable linkage names.
pub(crate) fn public_text(mut text: &str) -> String {
    if !text.contains('\0') {
        return text.to_owned();
    }
    let mut displayed = String::with_capacity(text.len());
    while let Some((plain, linked)) = text.split_once('\0') {
        displayed.push_str(plain);
        let Some((tag, rest)) = linked.split_once(':') else {
            displayed.push_str(linked);
            return displayed;
        };
        if tag.len() % 2 == 0 && tag.bytes().all(|b| b.is_ascii_hexdigit()) {
            let decoded = (0..tag.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&tag[i..i + 2], 16))
                .collect::<Result<Vec<_>, _>>();
            if let Ok(bytes) = decoded {
                let path = String::from_utf8_lossy(&bytes);
                if let Some(stem) = std::path::Path::new(path.as_ref()).file_stem() {
                    displayed.push_str(&stem.to_string_lossy());
                    displayed.push('.');
                }
            }
        }
        text = rest;
    }
    displayed.push_str(text);
    displayed
}
fn issue(category: Category, offset: usize, message: impl Into<String>) -> Error {
    Diagnostic::new(category, Span::at(offset), public_text(&message.into())).into_error()
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Timings {
    pub resolution: Duration,
    pub inference: Duration,
    pub elaboration: Duration,
}

pub fn prepare(program: &Program, mode: Mode, policy: Policy) -> Result<Prepared<'_>, Error> {
    prepare_timed(program, mode, policy, &mut Timings::default())
}

/// Resolves, checks and lowers exactly once. Unsupported operations are diagnostics.
pub fn prepare_timed<'a>(
    program: &'a Program,
    mode: Mode,
    policy: Policy,
    timing: &mut Timings,
) -> Result<Prepared<'a>, Error> {
    prepare_modules_timed(program, mode, policy, &[], timing)
}
pub fn prepare_modules<'a>(
    program: &'a Program,
    mode: Mode,
    policy: Policy,
    modules: &[ModuleInput],
) -> Result<Prepared<'a>, Error> {
    prepare_modules_timed(program, mode, policy, modules, &mut Timings::default())
}
pub fn check_module_interfaces(
    program: &Program,
    modules: &[ModuleInput],
) -> Result<Vec<ModuleInterface>, Error> {
    let (resolution, inferred) = resolve_and_infer(program, &mut Timings::default())?;
    checked_interfaces(program, modules, &resolution, &inferred)
}
pub fn prepare_modules_timed<'a>(
    program: &'a Program,
    mode: Mode,
    _policy: Policy,
    modules: &[ModuleInput],
    timing: &mut Timings,
) -> Result<Prepared<'a>, Error> {
    let (names, inferred) = resolve_and_infer(program, timing)?;
    let module_interfaces = checked_interfaces(program, modules, &names, &inferred)?;
    let entry_type = program.entry.as_ref().and_then(|e| inferred.type_of(e));
    let result_type = entry_type.map(|t| match t {
        Type::Action(result) => result.as_ref(),
        other => other,
    });
    let entry_start = program.entry_start.unwrap_or(0);
    if let (Mode::Script, Some(t)) = (mode, result_type) {
        let contextual_status = matches!(
            (&program.entry, t),
            (Some(Expr::Action(_, Some(tail))), Type::Prim(Prim::Z))
                if matches!(tail.as_ref(), Expr::Int(value) if *value >= 0)
                   || matches!(tail.as_ref(), Expr::Num(raw) if raw.bytes().all(|b| b.is_ascii_digit()))
        );
        let valid_result = matches!(t, Type::Result(ok, _) if matches!(ok.as_ref(), Type::Prim(Prim::Unit | Prim::N)));
        if !matches!(t, Type::Prim(Prim::Unit | Prim::N)) && !contextual_status && !valid_result {
            return Err(issue(
                Category::Type,
                entry_start,
                format!("entry must yield (), n, ()^e or n^e, found {t}"),
            ));
        }
    }
    let inspection = result_type.map(|t| check::Inspection {
        type_name: public_text(&t.to_string()),
        schema: match t {
            Type::Table(row) => {
                let record = match row.as_ref() {
                    Type::Record(record) => Some(record),
                    Type::Nominal(name) => inferred.nominal_schema(name),
                    _ => None,
                };
                Some(public_text(&record.map_or_else(
                    || row.to_string(),
                    |record| {
                        format!(
                            ".{{ {} }}",
                            record
                                .fields
                                .iter()
                                .map(|(n, t)| format!("{n}:{t}"))
                                .collect::<Vec<_>>()
                                .join("; ")
                        )
                    },
                )))
            }
            _ => None,
        },
    });
    let start = Instant::now();
    let lowered = hir::lower_live(program, &inferred, &names)
        .map_err(|boundary| {
            Diagnostic::new(
                if boundary.is_migration() {
                    Category::MigrationBoundary
                } else {
                    Category::InvalidHir
                },
                boundary.span,
                public_text(&boundary.message),
            )
            .into_error()
        })
        .and_then(|hir| {
            hir::validate(&hir)?;
            Ok(hir)
        });
    timing.elaboration = start.elapsed();
    Ok(Prepared {
        hir: lowered?,
        inspection,
        module_interfaces,
        source: std::marker::PhantomData,
    })
}

fn resolve_and_infer(
    program: &Program,
    timing: &mut Timings,
) -> Result<(Resolution, infer::InferResult), Error> {
    let start = Instant::now();
    let names = resolve::resolve_live(program)
        .map_err(|e| issue(Category::Resolution, e.start, e.message))?;
    timing.resolution = start.elapsed();
    let start = Instant::now();
    let inference = infer::infer_live(program, &names);
    timing.inference = start.elapsed();
    let inferred = match inference {
        Ok(value) => value,
        Err(failure) => {
            let category = match failure.kind {
                infer::InferErrorKind::Unsupported => Category::MigrationBoundary,
                infer::InferErrorKind::IncompatibleError => Category::IncompatibleError,
                infer::InferErrorKind::ResultNeedsHandling => Category::ResultNeedsHandling,
                _ => Category::Type,
            };
            let mut diagnostic = Diagnostic::new(
                category,
                Span::new(failure.start, failure.end),
                public_text(&failure.message),
            );
            diagnostic.expected = failure.expected.as_deref().map(public_text);
            diagnostic.actual = failure.actual.as_deref().map(public_text);
            diagnostic.constraint = failure.constraint.as_deref().map(public_text);
            diagnostic.suggestion = match category {
                Category::IncompatibleError => {
                    Some("recover and convert the error explicitly before propagating".into())
                }
                Category::ResultNeedsHandling => {
                    Some("use ^= to recover or ^? inside a fallible boundary".into())
                }
                _ => None,
            };
            return Err(diagnostic.into_error());
        }
    };
    Ok((names, inferred))
}
fn annotated(t: &TypeAnnotation) -> bool {
    use TypeAnnotationKind::*;
    match &t.kind {
        Hole => false,
        Optional(t) | Action(t) | Table(t) | Sequence(t) => annotated(t),
        Result(a, b) | Dictionary(a, b) => annotated(a) && annotated(b),
        Function(xs, out) => xs.iter().all(annotated) && annotated(out),
        PositionalTable(xs) | Tuple(xs) | NamedApply(_, xs) => xs.iter().all(annotated),
        Record(fields) | OpenRecord(fields, _) => fields.iter().all(|(_, t)| annotated(t)),
        Primitive(_) | Named(_) | Variable(_) => true,
    }
}
fn private_nominal<'a>(ty: &'a Type, private: &HashSet<&str>) -> Option<&'a NominalId> {
    use Type::*;
    match ty {
        Nominal(id) | Applied(id, _) if private.contains(id.name.as_str()) => Some(id),
        Applied(_, args) => args.iter().find_map(|t| private_nominal(t, private)),
        Seq(t) | Optional(t) | Action(t) | Table(t) => private_nominal(t, private),
        Dict(a, b) | Fun(a, b) | Result(a, b) => {
            private_nominal(a, private).or_else(|| private_nominal(b, private))
        }
        Tuple(ts) => ts.iter().find_map(|t| private_nominal(t, private)),
        Record(row) => row
            .fields
            .iter()
            .find_map(|(_, t)| private_nominal(t, private)),
        _ => None,
    }
}
fn checked_interfaces(
    program: &Program,
    modules: &[ModuleInput],
    names: &Resolution,
    inferred: &infer::InferResult,
) -> Result<Vec<ModuleInterface>, Error> {
    if modules.is_empty() {
        return Ok(vec![]);
    }
    let requested = modules
        .iter()
        .flat_map(|m| m.exports.iter().map(|e| e.linked.as_str()))
        .collect::<HashSet<_>>();
    let mut available = HashMap::with_capacity(requested.len());
    let declarations = if requested.is_empty() {
        &[][..]
    } else {
        program.defs.as_slice()
    };
    for stmt in declarations {
        if let Some(id) = names.declaration(stmt) {
            let (name, kind, explicit) = match stmt {
                Stmt::Def {
                    name,
                    params,
                    param_types,
                    result_type,
                    ..
                } => {
                    let kind = if params.is_empty() {
                        MemberKind::Value
                    } else {
                        MemberKind::Callable(params.len())
                    };
                    let explicit = result_type.as_ref().is_some_and(annotated)
                        && params.len() == param_types.len()
                        && param_types
                            .iter()
                            .all(|t| t.as_ref().is_some_and(annotated));
                    (name, kind, explicit)
                }
                Stmt::Schema { name, fields, .. } => (
                    name,
                    MemberKind::Record,
                    fields.iter().all(|(_, t)| annotated(t)),
                ),
                Stmt::Sum { name, variants, .. } => {
                    for variant in variants
                        .iter()
                        .filter(|v| requested.contains(v.name.as_str()))
                    {
                        if let Some(id) = names.variant(variant) {
                            available.insert(
                                variant.name.as_str(),
                                (
                                    id,
                                    MemberKind::Constructor(variant.payload.len()),
                                    variant.payload.iter().all(annotated),
                                ),
                            );
                        }
                    }
                    (name, MemberKind::Sum, true)
                }
                Stmt::Effect { .. } => continue,
            };
            if requested.contains(name.as_str()) {
                available.insert(name.as_str(), (id, kind, explicit));
            }
        }
    }
    let mut paths = HashSet::new();
    let mut interfaces = Vec::with_capacity(modules.len());
    for module in modules {
        if !paths.insert(&module.path) {
            return Err(issue(
                Category::Resolution,
                0,
                format!("duplicate module interface for {}", module.path.display()),
            ));
        }
        let visible_nominal = module
            .exports
            .iter()
            .filter(|m| matches!(m.kind, MemberKind::Record | MemberKind::Sum))
            .map(|m| m.linked.as_str())
            .collect::<HashSet<_>>();
        let private = module
            .nominal_types
            .iter()
            .map(String::as_str)
            .filter(|name| !visible_nominal.contains(name))
            .collect::<HashSet<_>>();
        let mut surface = HashSet::new();
        let mut exports = Vec::with_capacity(module.exports.len());
        for m in &module.exports {
            let (symbol, kind, explicit) = available.get(m.linked.as_str()).ok_or_else(|| {
                issue(
                    Category::Resolution,
                    0,
                    format!(
                        "{}: declared export {} has no linked declaration",
                        module.path.display(),
                        m.name
                    ),
                )
            })?;
            let start = names.symbol(*symbol).start;
            if !surface.insert(&m.name) {
                return Err(issue(
                    Category::Resolution,
                    start,
                    format!("duplicate interface export {}", m.name),
                ));
            }
            if *kind != m.kind {
                return Err(issue(
                    Category::Type,
                    start,
                    format!(
                        "export {} has a different declaration kind or callable arity",
                        m.name
                    ),
                ));
            }
            if !explicit {
                return Err(issue(
                    Category::Type,
                    start,
                    format!("export {} requires a complete explicit signature", m.name),
                ));
            }
            let scheme = inferred.binding_type(*symbol).cloned().ok_or_else(|| {
                issue(
                    Category::Type,
                    start,
                    format!("export {} has no checked type scheme", m.name),
                )
            })?;
            let constructor = inferred.constructor(*symbol).cloned();
            let nominal = match m.kind {
                MemberKind::Record | MemberKind::Sum => match &scheme.ty {
                    Type::Nominal(n) | Type::Applied(n, _) => Some(n.clone()),
                    _ => None,
                },
                MemberKind::Constructor(_) => constructor.as_ref().map(|c| c.owner.clone()),
                _ => None,
            };
            let row = if matches!(m.kind, MemberKind::Record) {
                nominal
                    .as_ref()
                    .and_then(|n| inferred.nominal_schema(n))
                    .cloned()
            } else {
                None
            };
            let hidden = private_nominal(&scheme.ty, &private).or_else(|| {
                row.as_ref().and_then(|r| {
                    r.fields
                        .iter()
                        .find_map(|(_, t)| private_nominal(t, &private))
                })
            });
            if let Some(id) = hidden {
                return Err(issue(Category::Type,start,format!(
                    "export {} exposes private nominal type {}; export that type or change the signature",
                    m.name,id.name
                )));
            }
            exports.push(CheckedModuleMember {
                member: m.clone(),
                symbol: *symbol,
                scheme,
                nominal,
                row,
                constructor,
            });
        }
        interfaces.push(ModuleInterface {
            path: module.path.clone(),
            dependencies: module.dependencies.clone(),
            exports,
        });
    }
    Ok(interfaces)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;

    fn member(name: &str, kind: MemberKind) -> ModuleMember {
        ModuleMember {
            name: name.into(),
            linked: name.into(),
            kind,
        }
    }
    fn input(exports: Vec<ModuleMember>, nominal_types: &[&str]) -> ModuleInput {
        ModuleInput {
            path: PathBuf::from("/fixture/ledger.flx"),
            dependencies: vec![PathBuf::from("/fixture/other.flx")],
            exports,
            nominal_types: nominal_types.iter().map(|n| (*n).into()).collect(),
        }
    }

    #[test]
    fn module_interfaces_use_resolved_symbols_and_checked_schemes() {
        let p = parser::parse(
            r#"
            row :: { value:z; };
            issue :: { | bad row; };
            poly(x:'a):'a := x;
            callback:z->z := &poly;
            hidden := 7;
        "#,
        )
        .unwrap();
        let metadata = input(
            vec![
                member("row", MemberKind::Record),
                member("issue", MemberKind::Sum),
                member("bad", MemberKind::Constructor(1)),
                member("poly", MemberKind::Callable(1)),
                member("callback", MemberKind::Value),
            ],
            &["row", "issue"],
        );
        let checked = prepare_modules(
            &p,
            Mode::Script,
            Policy::TypedOnly,
            std::slice::from_ref(&metadata),
        )
        .unwrap();
        let public = checked.module_interfaces().first().unwrap();
        assert_eq!(public.path, metadata.path);
        assert_eq!(public.dependencies, metadata.dependencies);
        assert_eq!(public.exports.len(), 5);
        let row = public.exports[0].row.as_ref().unwrap();
        assert_eq!(row.fields[0], ("value".into(), Type::Prim(Prim::Z)));
        let constructor = public.exports[2].constructor.as_ref().unwrap();
        assert_eq!(Some(&constructor.owner), public.exports[1].nominal.as_ref());
        assert_eq!(
            constructor.payload,
            [Type::Nominal(public.exports[0].nominal.clone().unwrap())]
        );
        assert_eq!(public.exports[3].scheme.vars.len(), 1);
        assert_eq!(public.exports[4].member.kind, MemberKind::Value);
        assert!(matches!(public.exports[4].scheme.ty, Type::Fun(..)));
    }

    #[test]
    fn rejects_export_with_private_nominal_in_signature_or_record_layout() {
        for (src, public) in [
            (
                "hidden :: { v:z; }; make(x:z):hidden := hidden { v:=x; };",
                member("make", MemberKind::Callable(1)),
            ),
            (
                "hidden :: { v:z; }; container :: { child:hidden; };",
                member("container", MemberKind::Record),
            ),
        ] {
            let p = parser::parse(src).unwrap();
            let meta = input(vec![public], &["hidden", "container"]);
            let e = check_module_interfaces(&p, &[meta]).unwrap_err();
            assert!(e.message.contains("private nominal type hidden"), "{e:?}");
        }
    }

    #[test]
    fn permits_signatures_over_imported_public_type_without_reexport() {
        let p = parser::parse("upstream :: { n:z; }; local(x:upstream):z := .n x;").unwrap();
        let mut a = input(vec![member("upstream", MemberKind::Record)], &["upstream"]);
        a.path = PathBuf::from("/fixture/other.flx");
        a.dependencies.clear();
        let b = input(vec![member("local", MemberKind::Callable(1))], &[]);
        let result = check_module_interfaces(&p, &[a, b]).unwrap();
        let upstream = result[0].exports[0].nominal.as_ref().unwrap();
        assert_eq!(result[1].exports.len(), 1);
        assert_eq!(
            result[1].exports[0].scheme.ty,
            Type::Fun(
                Box::new(Type::Nominal(upstream.clone())),
                Box::new(Type::Prim(Prim::Z))
            )
        );
    }
}
