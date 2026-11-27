//! Draft 0.4 semantic entry point and checked migration boundary.
use crate::{
    check,
    diagnostic::{Category, Diagnostic, Span},
    eval, hir, hir_eval, infer,
    parser::{Expr, Program},
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

fn legacy(
    program: &Program,
    mode: Mode,
    policy: Policy,
    start: usize,
    reason: String,
) -> Result<Prepared<'_>, Error> {
    if policy == Policy::TypedOnly || !program.constraints.is_empty() {
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
            return Err(Diagnostic::new(
                Category::Type,
                Span::new(failure.start, failure.end),
                failure.message,
            )
            .into_error())
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
        if !matches!(t, Type::Prim(Prim::Unit | Prim::N)) && !contextual_status {
            return Err(issue(
                Category::Type,
                entry_start,
                format!("entry must yield () or n, found {t}"),
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
