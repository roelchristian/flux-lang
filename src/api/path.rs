//! Path-based compilation and execution.
use crate::{eval, modules, semantic, Error, RuntimeOptions};

/// Failure from a path-based frontend or runtime operation.
#[derive(Debug)]
pub enum PathError {
    Load(modules::LoadError),
    Semantic { error: Error, rendered: String },
}

impl PathError {
    pub fn diagnostic(&self) -> String {
        match self {
            Self::Load(error) => error.diagnostic(),
            Self::Semantic { rendered, .. } => rendered.clone(),
        }
    }

    pub fn semantic(loaded: &modules::LoadedProgram, error: Error) -> Self {
        let rendered = loaded.diagnostic(&error);
        Self::Semantic { error, rendered }
    }
}

impl From<modules::LoadError> for PathError {
    fn from(error: modules::LoadError) -> Self {
        Self::Load(error)
    }
}

impl std::fmt::Display for PathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.diagnostic())
    }
}

impl std::error::Error for PathError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Load(error) => Some(error),
            Self::Semantic { error, .. } => Some(error),
        }
    }
}

fn prepare_path(loaded: &modules::LoadedProgram) -> Result<semantic::Prepared<'_>, PathError> {
    semantic::prepare_modules(
        &loaded.program,
        semantic::Mode::Script,
        semantic::Policy::TypedOnly,
        &loaded.module_inputs,
    )
    .map_err(|error| PathError::semantic(loaded, error))
}

/// Load and type-check a file and its local dependency graph without evaluating.
pub fn check_path(path: impl AsRef<std::path::Path>) -> Result<modules::LoadedProgram, PathError> {
    let loaded = modules::load(path)?;
    prepare_path(&loaded)?;
    Ok(loaded)
}

/// Check the complete graph and return the root module's explicit public interface.
pub fn interface_path(
    path: impl AsRef<std::path::Path>,
) -> Result<semantic::ModuleInterface, PathError> {
    let loaded = modules::load(path)?;
    let interfaces = semantic::check_module_interfaces(&loaded.program, &loaded.module_inputs)
        .map_err(|error| PathError::semantic(&loaded, error))?;
    interfaces
        .into_iter()
        .find(|interface| interface.path == loaded.root)
        .ok_or_else(|| PathError::semantic(&loaded, Error::new(0, "missing root module interface")))
}

/// Load, check and execute one file using typed HIR.
pub fn run_path(path: impl AsRef<std::path::Path>) -> Result<eval::Output, PathError> {
    run_path_with_options(path, RuntimeOptions::default())
}

/// Load, check and run a file with explicit host resources and optimization policy.
pub fn run_path_with_options(
    path: impl AsRef<std::path::Path>,
    options: RuntimeOptions,
) -> Result<eval::Output, PathError> {
    let loaded = modules::load(path)?;
    prepare_path(&loaded)?
        .run_with_options(options)
        .map_err(|error| PathError::semantic(&loaded, error))
}
