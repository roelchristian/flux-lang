use flux_lang::{check_source, format_source, run_source};
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::process::ExitCode;

fn repl() -> Result<u8, String> {
    let stdin = io::stdin();
    let interactive = stdin.is_terminal();
    let mut lines = stdin.lock().lines();
    let mut declarations = String::from("<< \"io\" : io;\n");
    loop {
        if interactive {
            print!("flux> ");
            io::stdout().flush().map_err(|e| e.to_string())?;
        }
        let Some(line) = lines.next() else { break };
        let line = line.map_err(|e| e.to_string())?;
        let line = line.trim();
        if line == ":q" || line == ":quit" {
            break;
        }
        if line.is_empty() || line.starts_with("--") {
            continue;
        }
        let statement = if line.ends_with(';') {
            line.to_string()
        } else {
            format!("{line};")
        };
        let candidate = format!("{declarations}{statement}\n");
        if let Ok(program) = check_source(&candidate) {
            if program.entry.is_none() {
                declarations = candidate;
                continue;
            }
            match run_source(&candidate) {
                Ok(output) => print!("{}", output.stdout),
                Err(error) => eprintln!("{}", error.diagnostic("<repl>", &candidate)),
            }
            continue;
        }
        let expression = line.strip_suffix(';').unwrap_or(line);
        let command = format!("{declarations}!{{ ! io.print ({expression}); }};");
        match run_source(&command) {
            Ok(output) => print!("{}", output.stdout),
            Err(err) => eprintln!("{}", err.diagnostic("<repl>", &command)),
        }
    }
    Ok(0)
}

fn cli() -> Result<u8, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mode, path) = match args.as_slice() {
        [] => return repl(),
        [flag] if flag == "--help" || flag == "-h" => {
            println!("Flux reference prototype\n\nUsage: flux [FILE | check FILE | fmt FILE | --version]\n\nRun without arguments for the REPL. fmt writes to stdout.");
            return Ok(0);
        }
        [flag] if flag == "--version" || flag == "-V" => {
            println!("flux {}", env!("CARGO_PKG_VERSION"));
            return Ok(0);
        }
        [file] if file != "check" && file != "fmt" && !file.starts_with('-') => ("run", file),
        [command, file] if command == "check" || command == "fmt" => (command.as_str(), file),
        _ => return Err("usage: flux [FILE | check FILE | fmt FILE]".into()),
    };
    let source = fs::read_to_string(path).map_err(|err| format!("{path}: {err}"))?;
    let diagnostic = |err: flux_lang::Error| err.diagnostic(path, &source);
    match mode {
        "check" => {
            check_source(&source).map_err(diagnostic)?;
        }
        "fmt" => print!("{}", format_source(&source).map_err(diagnostic)?),
        _ => {
            let program = check_source(&source).map_err(diagnostic)?;
            if program.entry.is_none() {
                return Err(format!(
                    "{path}: no executable !{{ ... }}; block (try flux check)"
                ));
            }
            let output = flux_lang::eval::run(&program).map_err(diagnostic)?;
            print!("{}", output.stdout);
            return Ok(output.status);
        }
    }
    Ok(0)
}

fn main() -> ExitCode {
    match cli() {
        Ok(status) => ExitCode::from(status),
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(1)
        }
    }
}
