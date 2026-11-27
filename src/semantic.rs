//! Draft 0.4 semantic entry point and checked migration boundary.
use crate::{
    check,
    diagnostic::{Category, Diagnostic, Span},
    eval, hir, hir_eval, infer,
    parser::{Expr, Program, Stmt},
    resolve,
    types::{Prim, Type},
    Error,
};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Script,
    Inspection,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Policy {
    Compatible,
    TypedOnly,
}

#[derive(Debug)]
enum Backend<'a> {
    Typed(hir::HirProgram),
    Draft03 {
        program: &'a Program,
        reason: String,
    },
}

#[derive(Debug)]
pub struct Prepared<'a> {
    backend: Backend<'a>,
    inspection: Option<check::Inspection>,
}

impl Prepared<'_> {
    pub fn hir(&self) -> Option<&hir::HirProgram> {
        match &self.backend {
            Backend::Typed(program) => Some(program),
            Backend::Draft03 { .. } => None,
        }
    }

    pub fn migration_reason(&self) -> Option<&str> {
        match &self.backend {
            Backend::Typed(_) => None,
            Backend::Draft03 { reason, .. } => Some(reason),
        }
    }

    pub fn inspection(&self) -> Option<&check::Inspection> {
        self.inspection.as_ref()
    }

    pub fn run(&self) -> Result<eval::Output, Error> {
        match &self.backend {
            Backend::Typed(program) => hir_eval::run(program),
            Backend::Draft03 { program, .. } => eval::run(program),
        }
    }

    /// Execute with ordered writes. Unlike collected output, writes remain visible on failure.
    pub fn run_with_output(&self, sink: &mut dyn std::io::Write) -> Result<eval::Output, Error> {
        match &self.backend {
            Backend::Typed(program) => hir_eval::run_with_output(program, sink),
            Backend::Draft03 { program, .. } => {
                let mut result = eval::run(program)?;
                sink.write_all(result.stdout.as_bytes())
                    .map_err(|e| issue(Category::RuntimeDomain, 0, e.to_string()))?;
                result.stdout.clear();
                Ok(result)
            }
        }
    }

    /// Execute typed HIR with an injectable file reader and ordered output writer.
    /// Filesystem operations other than reading use the host filesystem.
    pub fn run_with_io(
        &self,
        input: &mut dyn crate::RuntimeInput,
        output: &mut dyn std::io::Write,
    ) -> Result<eval::Output, Error> {
        match &self.backend {
            Backend::Typed(program) => hir_eval::run_with_io(program, input, output),
            Backend::Draft03 { .. } => Err(issue(
                Category::MigrationBoundary,
                0,
                "injected runtime input requires typed HIR",
            )),
        }
    }

    pub fn view_with_output(
        &self,
        sink: &mut dyn std::io::Write,
    ) -> Result<eval::ReplOutput, Error> {
        match &self.backend {
            Backend::Typed(program) => hir_eval::view_with_output(program, sink),
            Backend::Draft03 { program, .. } => {
                let mut result = eval::view_entry(program)?;
                sink.write_all(result.stdout.as_bytes())
                    .map_err(|e| issue(Category::RuntimeDomain, 0, e.to_string()))?;
                result.stdout.clear();
                Ok(result)
            }
        }
    }

    pub fn view(&self) -> Result<eval::ReplOutput, Error> {
        match &self.backend {
            Backend::Typed(program) => hir_eval::view_entry(program),
            Backend::Draft03 { program, .. } => eval::view_entry(program),
        }
    }
}

fn issue(category: Category, offset: usize, message: impl Into<String>) -> Error {
    Diagnostic::new(category, Span::at(offset), message).into_error()
}

fn requires_error_hir(program: &Program) -> bool {
    fn builtin(name: &str) -> bool {
        matches!(
            crate::builtins::canonical(name),
            "io.read"
                | "io.write"
                | "io.exists"
                | "io.print"
                | "csv.read"
                | "csv.write"
                | "math.div"
        )
    }
    fn stmt(s: &Stmt) -> bool {
        match s {
            Stmt::Sum { .. } => true,
            Stmt::Def {
                param_types,
                result_type,
                body,
                ..
            } => {
                param_types
                    .iter()
                    .flatten()
                    .chain(result_type)
                    .any(|t| t.contains('^'))
                    || expr(body)
            }
            Stmt::Schema { fields, .. } => fields.iter().any(|(_, t)| t.contains('^')),
            Stmt::Effect { body, .. } => expr(body),
        }
    }
    fn expr(e: &Expr) -> bool {
        match e {
            Expr::Fallible(..) | Expr::Raise(..) | Expr::Propagate(..) | Expr::Recover(..) => true,
            Expr::Ref(name) => builtin(name),
            Expr::Call(name, xs) => builtin(name) || xs.iter().any(expr),
            Expr::Array(xs) | Expr::Tuple(xs) | Expr::Pipeline(xs) => xs.iter().any(expr),
            Expr::Cases(cases, tail) => {
                cases.iter().any(|(cond, value)| expr(cond) || expr(value)) || expr(tail)
            }
            Expr::Record(fields) | Expr::NamedRecord(_, fields) => {
                fields.iter().any(|(_, v)| expr(v))
            }
            Expr::Lambda(_, body) => expr(body),
            Expr::TypedLambda(_, ts, body) => {
                ts.iter().flatten().any(|t| t.contains('^')) || expr(body)
            }
            Expr::Block(xs, tail) | Expr::Action(xs, tail) => {
                xs.iter().any(stmt) || tail.as_deref().is_some_and(expr)
            }
            _ => false,
        }
    }
    program.defs.iter().any(stmt) || program.entry.as_ref().is_some_and(expr)
}

fn legacy(
    program: &Program,
    mode: Mode,
    policy: Policy,
    start: usize,
    reason: String,
) -> Result<Prepared<'_>, Error> {
    if policy == Policy::TypedOnly || !program.constraints.is_empty() || requires_error_hir(program)
    {
        return Err(issue(Category::MigrationBoundary, start, reason));
    }
    let inspection = match mode {
        Mode::Script => {
            check::check(program)?;
            None
        }
        Mode::Inspection => Some(check::inspect_entry(program)?),
    };
    Ok(Prepared {
        backend: Backend::Draft03 { program, reason },
        inspection,
    })
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

/// Does not fall back on resolution errors, type errors, or invalid HIR.
/// The Draft 0.3 compatibility engine is used only at named migration boundaries.
pub fn prepare_timed<'a>(
    program: &'a Program,
    mode: Mode,
    policy: Policy,
    timing: &mut Timings,
) -> Result<Prepared<'a>, Error> {
    let start = Instant::now();
    let names =
        resolve::resolve(program).map_err(|e| issue(Category::Resolution, e.start, e.message))?;
    timing.resolution = start.elapsed();
    let start = Instant::now();
    let inference = infer::infer(program, &names);
    timing.inference = start.elapsed();
    let inferred = match inference {
        Ok(value) => value,
        Err(failure) if failure.kind == infer::InferErrorKind::Unsupported => {
            return legacy(program, mode, policy, failure.start, failure.message);
        }
        Err(failure) => {
            let category = match failure.kind {
                infer::InferErrorKind::IncompatibleError => Category::IncompatibleError,
                infer::InferErrorKind::ResultNeedsHandling => Category::ResultNeedsHandling,
                _ => Category::Type,
            };
            let mut diagnostic = Diagnostic::new(
                category,
                Span::new(failure.start, failure.end),
                failure.message,
            );
            diagnostic.expected = failure.expected;
            diagnostic.actual = failure.actual;
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
        type_name: t.to_string(),
        schema: match t {
            Type::Table(row) => {
                let record = match row.as_ref() {
                    Type::Record(record) => Some(record),
                    Type::Nominal(name) => inferred.nominal_schema(name),
                    _ => None,
                };
                Some(record.map_or_else(
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
                ))
            }
            _ => None,
        },
    });
    let start = Instant::now();
    let lowered = hir::lower(program, &inferred, &names);
    timing.elaboration = start.elapsed();
    match lowered {
        Ok(hir) => Ok(Prepared {
            backend: Backend::Typed(hir),
            inspection,
        }),
        Err(boundary) if boundary.is_migration() => {
            legacy(program, mode, policy, boundary.span.start, boundary.message)
        }
        Err(boundary) => Err(issue(
            Category::InvalidHir,
            boundary.span.start,
            boundary.message,
        )),
    }
}
