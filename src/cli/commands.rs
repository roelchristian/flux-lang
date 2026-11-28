use super::{config::runtime_options, repl::repl};
use flux_lang::{
    format_path, interface_path, modules,
    parser::MemberKind,
    semantic::{prepare_modules, prepare_modules_timed, Mode as SemanticMode, Policy, Timings},
};
use std::{io, path::Path, time::Instant};

pub(super) fn cli() -> Result<u8, String> {
    const COMMANDS: &[&str] = &["check", "fmt", "hir", "interface", "phases", "run-hir"];
    const USAGE: &str =
        "flux [FILE [-- ARGS...] | check FILE | hir FILE | interface FILE | run-hir FILE [-- ARGS...] | phases FILE | fmt [--check] FILE | --version]";
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let separator = args.iter().position(|arg| arg == "--");
    let arguments = match separator {
        Some(at) => args[at + 1..]
            .iter()
            .map(|arg| {
                arg.to_str()
                    .map(str::to_owned)
                    .ok_or("script arguments must be UTF-8")
            })
            .collect::<Result<Vec<_>, _>>()?,
        None => Vec::new(),
    };
    let cli_args = &args[..separator.unwrap_or(args.len())];
    if separator.is_some()
        && !matches!(cli_args, [file] if !file.to_string_lossy().starts_with('-'))
        && !matches!(cli_args, [cmd, _] if cmd == "run-hir")
    {
        return Err(format!("usage: {USAGE}"));
    }
    let (mode, path) = match cli_args {
        [] => return repl(),
        [flag] if flag == "--help" || flag == "-h" => {
            println!("Flux reference prototype\n\nUsage: {USAGE}\n\nRun without arguments for the REPL.\nfmt FILE writes formatted text to stdout without editing FILE.\nfmt --check FILE writes nothing to stdout and exits 1 if FILE needs formatting.");
            return Ok(0);
        }
        [flag] if flag == "--version" || flag == "-V" => {
            println!("flux {}", env!("CARGO_PKG_VERSION"));
            return Ok(0);
        }
        [file]
            if file
                .to_str()
                .is_none_or(|text| !COMMANDS.contains(&text) && !text.starts_with('-')) =>
        {
            ("run", Path::new(file))
        }
        [command, flag, file] if command == "fmt" && flag == "--check" => {
            ("fmt-check", Path::new(file))
        }
        [command, flag] if command == "fmt" && flag == "--check" => {
            return Err(format!("usage: {USAGE}"));
        }
        [command, file] => (
            command
                .to_str()
                .filter(|text| COMMANDS.contains(text))
                .ok_or_else(|| format!("usage: {USAGE}"))?,
            Path::new(file),
        ),
        _ => return Err(format!("usage: {USAGE}")),
    };
    if matches!(mode, "fmt" | "fmt-check") {
        let formatted = format_path(path).map_err(|error| error.diagnostic())?;
        if mode == "fmt" {
            print!("{formatted}");
        } else {
            let (_, original) = modules::read_fragment(path).map_err(|error| error.diagnostic())?;
            if original != formatted {
                return Err(format!("{}: needs formatting", path.display()));
            }
        }
        return Ok(0);
    }

    if mode == "interface" {
        let interface = interface_path(path).map_err(|error| error.diagnostic())?;
        println!("{}", interface.path.display());
        for export in interface.exports {
            let kind = match export.member.kind {
                MemberKind::Callable(arity) => format!("callable/{arity}"),
                MemberKind::Constructor(arity) => format!("constructor/{arity}"),
                MemberKind::Value => "value".into(),
                MemberKind::Record => "record".into(),
                MemberKind::Sum => "sum".into(),
            };
            if let Some(row) = export.row {
                println!("  {kind} {}: {row}", export.member.name);
            } else {
                println!("  {kind} {}: {}", export.member.name, export.scheme);
            }
        }
        return Ok(0);
    }

    // The parse phase now includes discovery, local loads, parse of each
    // dependency, and assembly. Keep the stable timing key for tooling.
    let start = Instant::now();
    let loaded = modules::load(path).map_err(|error| error.diagnostic())?;
    let parse = start.elapsed();
    let diagnostic = |error: flux_lang::Error| loaded.diagnostic(&error);
    let program = &loaded.program;
    let inputs = &loaded.module_inputs;
    match mode {
        "check" | "hir" => {
            let prepared =
                prepare_modules(program, SemanticMode::Script, Policy::TypedOnly, inputs)
                    .map_err(diagnostic)?;
            if mode == "hir" {
                println!(
                    "{:#?}",
                    prepared.hir().expect("strict compilation yields HIR")
                );
            }
        }
        "phases" => {
            let mut timing = Timings::default();
            let plan = prepare_modules_timed(
                program,
                SemanticMode::Script,
                Policy::TypedOnly,
                inputs,
                &mut timing,
            )
            .map_err(diagnostic)?;
            let start = Instant::now();
            if program.entry.is_some() {
                plan.run_with_host_options(
                    &mut flux_lang::FsInput,
                    &mut io::stdin().lock(),
                    &[],
                    &mut io::stdout(),
                    runtime_options()?,
                )
                .map_err(diagnostic)?;
            }
            let run = start.elapsed();
            eprintln!(
                "parse_ns\t{}
resolve_ns\t{}
infer_ns\t{}
elaborate_ns\t{}
eval_ns\t{}",
                parse.as_nanos(),
                timing.resolution.as_nanos(),
                timing.inference.as_nanos(),
                timing.elaboration.as_nanos(),
                run.as_nanos()
            );
        }
        _ => {
            if program.entry.is_none() {
                return Err(format!(
                    "{}: no executable !{{ ... }}; block (try flux check)",
                    path.display()
                ));
            }
            let output = prepare_modules(program, SemanticMode::Script, Policy::TypedOnly, inputs)
                .map_err(diagnostic)?
                .run_with_host_options(
                    &mut flux_lang::FsInput,
                    &mut io::stdin().lock(),
                    &arguments,
                    &mut io::stdout(),
                    runtime_options()?,
                )
                .map_err(diagnostic)?;
            return Ok(output.status);
        }
    }
    Ok(0)
}
