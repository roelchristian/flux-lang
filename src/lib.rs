//! Flux compiler and typed reference runtime.

mod api;
mod core;
#[path = "runtime/host/input.rs"]
mod csv;
#[path = "diagnostics/mod.rs"]
pub mod diagnostic;
#[path = "runtime/host/fault.rs"]
mod host_fault;
#[path = "legacy/mod.rs"]
#[cfg(any(test, feature = "legacy-oracle"))]
mod legacy;
#[path = "compiler/symbol_name.rs"]
mod symbol_name;
mod tooling;

#[path = "core/registry/mod.rs"]
pub mod builtins;
#[path = "api/check.rs"]
pub mod check;
#[path = "api/eval.rs"]
pub mod eval;
#[path = "compiler/hir/mod.rs"]
pub mod hir;
#[path = "runtime/mod.rs"]
pub mod hir_eval;
#[path = "compiler/infer/mod.rs"]
pub mod infer;
#[path = "compiler/syntax/lexer.rs"]
pub mod lexer;
#[path = "compiler/modules.rs"]
pub mod modules;
#[path = "compiler/syntax/parser/mod.rs"]
pub mod parser;
#[path = "compiler/resolve/mod.rs"]
pub mod resolve;
#[path = "compiler/driver.rs"]
pub mod semantic;
#[path = "compiler/types/mod.rs"]
pub mod types;

pub use api::{
    check_path, check_source, interface_path, run_path, run_path_with_options, run_source,
    run_source_with_options, PathError,
};

pub use csv::{FsInput, OpenedInput, RuntimeInput};
pub use diagnostic::error::Error;
pub use hir_eval::{ResourcePolicy, RuntimeOptions};
pub use tooling::{documentation_comments, format_path, format_source, DocComment};

pub(crate) use core::standard_errors;
pub(crate) use symbol_name::visible_symbol;

#[cfg(feature = "legacy-oracle")]
pub mod oracle {
    pub use crate::legacy::check::check;
    pub use crate::legacy::eval::{run, Output};
}
