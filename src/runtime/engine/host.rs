//! Host I/O adapters for evaluation and REPL inspection.
use super::{
    control::{uncaught, Control},
    Eval, Frame,
};
use crate::{
    eval::Output,
    hir::{HirProgram, Span},
    hir_eval::{
        error, inspect, invariant, resources, wide, FsInput, RuntimeInput, RuntimeOptions, R, V,
    },
    Error,
};
use num_traits::ToPrimitive;
use std::io::{BufRead, Write};

fn evaluate(
    program: &HirProgram,
    input: &mut dyn RuntimeInput,
    stdin: &mut dyn BufRead,
    args: &[String],
    output: &mut dyn Write,
    options: RuntimeOptions,
) -> R<V> {
    let _resources = resources::enter(options).map_err(Control::Fatal)?;
    let mut eval = Eval {
        p: program,
        frames: vec![Frame::default()],
        input,
        stdin,
        args,
        output,
        fuel: options.resources.max_work,
        depth: 0,
        max_depth: options.resources.max_depth,
        effects: 0,
    };
    for group in &program.groups {
        for d in group {
            if !d.formals.is_empty() {
                eval.define_fun(d, 0);
            }
        }
        for d in group {
            if d.formals.is_empty() {
                let v = eval.eval(d.body, 0)?;
                eval.put(0, d.symbol, v);
            }
        }
    }
    if let Some(id) = program.entry {
        let v = eval.eval(id, 0)?;
        eval.execute(v, program.node(id).span)
    } else {
        Ok(V::Unit)
    }
}

/// Execute using injected files and output; line input is EOF and arguments are empty.
pub fn run_with_io(
    program: &HirProgram,
    input: &mut dyn RuntimeInput,
    sink: &mut dyn Write,
) -> Result<Output, Error> {
    run_with_host(program, input, &mut std::io::empty(), &[], sink)
}

/// Execute with explicit host file input, line input, script arguments, and output.
pub fn run_with_host(
    program: &HirProgram,
    input: &mut dyn RuntimeInput,
    stdin: &mut dyn BufRead,
    args: &[String],
    sink: &mut dyn Write,
) -> Result<Output, Error> {
    run_with_host_options(program, input, stdin, args, sink, RuntimeOptions::default())
}

/// Execute a checked program with explicit host resources and optimization policy.
pub fn run_with_host_options(
    program: &HirProgram,
    input: &mut dyn RuntimeInput,
    stdin: &mut dyn BufRead,
    args: &[String],
    sink: &mut dyn Write,
    options: RuntimeOptions,
) -> Result<Output, Error> {
    crate::hir::validate(program)?;
    run_with_host_options_prevalidated(program, input, stdin, args, sink, options)
}

pub(crate) fn run_with_host_options_prevalidated(
    program: &HirProgram,
    input: &mut dyn RuntimeInput,
    stdin: &mut dyn BufRead,
    args: &[String],
    sink: &mut dyn Write,
    options: RuntimeOptions,
) -> Result<Output, Error> {
    let mut value =
        evaluate(program, input, stdin, args, sink, options).map_err(Control::into_error)?;
    let span = program
        .entry
        .map(|id| program.node(id).span)
        .unwrap_or_default();
    if let V::Outcome(v) = value {
        value = match v {
            Ok(v) => v.as_ref().clone(),
            Err(f) => return Err(uncaught(program, &f)),
        };
    }
    let status = match value {
        V::Unit => 0,
        V::Wide(_) => wide::integral(&value)
            .ok()
            .and_then(|n| n.to_u8())
            .ok_or_else(|| error(span, "entry exit status must be from 0 to 255").into_error())?,
        V::Exact(n) if n.d == 1 => u8::try_from(n.n)
            .map_err(|_| error(span, "entry exit status must be from 0 to 255").into_error())?,
        _ => {
            return Err(
                invariant(span, "entry must return unit or a natural exit status").into_error(),
            )
        }
    };
    Ok(Output {
        stdout: String::new(),
        status,
    })
}
pub fn run_with_output(program: &HirProgram, sink: &mut dyn Write) -> Result<Output, Error> {
    run_with_io(program, &mut FsInput, sink)
}

pub fn run(program: &HirProgram) -> Result<Output, Error> {
    run_with_options(program, RuntimeOptions::default())
}

/// Execute with captured stdout and explicit runtime options.
pub fn run_with_options(program: &HirProgram, options: RuntimeOptions) -> Result<Output, Error> {
    crate::hir::validate(program)?;
    run_options_prevalidated(program, options)
}

pub(crate) fn run_options_prevalidated(
    program: &HirProgram,
    options: RuntimeOptions,
) -> Result<Output, Error> {
    let mut sink = Vec::new();
    let mut out = run_with_host_options_prevalidated(
        program,
        &mut FsInput,
        &mut std::io::empty(),
        &[],
        &mut sink,
        options,
    )?;
    out.stdout = String::from_utf8(sink)
        .map_err(|_| invariant(Span::default(), "output sink was not UTF-8").into_error())?;
    Ok(out)
}

/// Inspect an entry with EOF line input and no script arguments.
pub fn view_with_io(
    program: &HirProgram,
    input: &mut dyn RuntimeInput,
    sink: &mut dyn Write,
) -> Result<crate::eval::ReplOutput, Error> {
    view_with_host(program, input, &mut std::io::empty(), &[], sink)
}

/// Inspect an entry using explicit host streams and script arguments.
pub fn view_with_host(
    program: &HirProgram,
    input: &mut dyn RuntimeInput,
    stdin: &mut dyn BufRead,
    args: &[String],
    sink: &mut dyn Write,
) -> Result<crate::eval::ReplOutput, Error> {
    view_with_host_options(program, input, stdin, args, sink, RuntimeOptions::default())
}

/// Inspect an entry with explicit host resources and optimization policy.
pub fn view_with_host_options(
    program: &HirProgram,
    input: &mut dyn RuntimeInput,
    stdin: &mut dyn BufRead,
    args: &[String],
    sink: &mut dyn Write,
    options: RuntimeOptions,
) -> Result<crate::eval::ReplOutput, Error> {
    crate::hir::validate(program)?;
    view_with_host_options_prevalidated(program, input, stdin, args, sink, options)
}

pub(crate) fn view_with_host_options_prevalidated(
    program: &HirProgram,
    input: &mut dyn RuntimeInput,
    stdin: &mut dyn BufRead,
    args: &[String],
    sink: &mut dyn Write,
    options: RuntimeOptions,
) -> Result<crate::eval::ReplOutput, Error> {
    let value =
        evaluate(program, input, stdin, args, sink, options).map_err(Control::into_error)?;
    Ok(crate::eval::ReplOutput {
        value: inspect(&value),
        stdout: String::new(),
    })
}

pub fn view_with_output(
    program: &HirProgram,
    sink: &mut dyn Write,
) -> Result<crate::eval::ReplOutput, Error> {
    view_with_io(program, &mut FsInput, sink)
}

pub fn view_entry(program: &HirProgram) -> Result<crate::eval::ReplOutput, Error> {
    view_with_options(program, RuntimeOptions::default())
}

/// Inspect an entry with captured stdout and explicit runtime options.
pub fn view_with_options(
    program: &HirProgram,
    options: RuntimeOptions,
) -> Result<crate::eval::ReplOutput, Error> {
    crate::hir::validate(program)?;
    view_options_prevalidated(program, options)
}

pub(crate) fn view_options_prevalidated(
    program: &HirProgram,
    options: RuntimeOptions,
) -> Result<crate::eval::ReplOutput, Error> {
    let mut sink = Vec::new();
    let mut out = view_with_host_options_prevalidated(
        program,
        &mut FsInput,
        &mut std::io::empty(),
        &[],
        &mut sink,
        options,
    )?;
    out.stdout = String::from_utf8(sink)
        .map_err(|_| invariant(Span::default(), "output sink was not UTF-8").into_error())?;
    Ok(out)
}
