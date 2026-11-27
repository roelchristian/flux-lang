//! Single production semantic frontend and typed HIR execution.
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
    /// API alias retained for older callers. Does not enable a legacy backend.
    Compatible,
    TypedOnly,
}

#[derive(Debug)]
pub struct Prepared<'a> {
    hir: hir::HirProgram,
    inspection: Option<check::Inspection>,
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

    pub fn inspection(&self) -> Option<&check::Inspection> {
        self.inspection.as_ref()
    }

    pub fn run(&self) -> Result<eval::Output, Error> {
        hir_eval::run(&self.hir)
    }

    pub fn run_with_output(&self, sink: &mut dyn std::io::Write) -> Result<eval::Output, Error> {
        hir_eval::run_with_output(&self.hir, sink)
    }

    pub fn run_with_io(
        &self,
        input: &mut dyn crate::RuntimeInput,
        output: &mut dyn std::io::Write,
    ) -> Result<eval::Output, Error> {
        hir_eval::run_with_io(&self.hir, input, output)
    }

    pub fn view_with_output(
        &self,
        sink: &mut dyn std::io::Write,
    ) -> Result<eval::ReplOutput, Error> {
        hir_eval::view_with_output(&self.hir, sink)
    }

    pub fn view(&self) -> Result<eval::ReplOutput, Error> {
        hir_eval::view_entry(&self.hir)
    }
}

fn issue(category: Category, offset: usize, message: impl Into<String>) -> Error {
    Diagnostic::new(category, Span::at(offset), message).into_error()
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
    _policy: Policy,
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
            hir,
            inspection,
            source: std::marker::PhantomData,
        }),
        Err(boundary) => Err(Diagnostic::new(
            if boundary.is_migration() {
                Category::MigrationBoundary
            } else {
                Category::InvalidHir
            },
            boundary.span,
            boundary.message,
        )
        .into_error()),
    }
}
