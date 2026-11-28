//! In-memory compilation and execution.
use crate::{check, eval, parser, semantic, Error, RuntimeOptions};

pub fn check_source(source: &str) -> Result<parser::Program, Error> {
    let program = parser::parse(source)?;
    check::check(&program)?;
    Ok(program)
}

pub fn run_source(source: &str) -> Result<eval::Output, Error> {
    eval::run(&parser::parse(source)?)
}

/// Check and run source with explicit host resources and optimization policy.
pub fn run_source_with_options(
    source: &str,
    options: RuntimeOptions,
) -> Result<eval::Output, Error> {
    semantic::prepare(
        &parser::parse(source)?,
        semantic::Mode::Script,
        semantic::Policy::TypedOnly,
    )?
    .run_with_options(options)
}
