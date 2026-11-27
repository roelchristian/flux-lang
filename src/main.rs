use flux_lang::{
    format_source, lexer, parser,
    semantic::{prepare, prepare_timed, Mode as SemanticMode, Policy, Timings},
};
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::process::ExitCode;
use std::time::Instant;

const REPL_HELP: &str =
    "Submit one line at a time. Expressions print their value and type. Definitions and history last for this session.
:type EXPR    show the static type without evaluating
:schema EXPR  show a table's static schema without evaluating
:load PATH    load and check a file of declarations without executing it
:time EXPR    evaluate and report elapsed time
:history      list successful expressions from this session
:replay N     evaluate expression N again (including its explicit effects)
:help         show these commands
:quit         leave the REPL (:q also works alone)
Parenthesize a suspended action to inspect it; use ! to execute it.";

struct Session {
    declarations: String,
    expressions: Vec<String>,
}

impl Session {
    fn new() -> Self {
        Self {
            declarations: String::from("<< \"io\" : io;\n"),
            expressions: Vec::new(),
        }
    }

    fn expression(&mut self, expression: &str, mode: Mode) -> Result<(), String> {
        let expression = expression.trim();
        let tokens = lexer::lex(expression).map_err(|e| e.diagnostic("<repl>", expression))?;
        let Some(last) = tokens.last() else {
            return Err("expected an expression".into());
        };
        let cleaned;
        let expression = if last.text == ";" {
            cleaned = format!("{}{}", &expression[..last.start], &expression[last.end..]);
            cleaned.trim()
        } else {
            expression
        };
        if expression.is_empty() {
            return Err("expected an expression".into());
        }
        // Leave a newline before the closing delimiter; a line comment must not
        // consume the synthetic wrapper.
        let source = format!("{}!{{\n{expression}\n}};", self.declarations);
        let diag = |error: flux_lang::Error| error.diagnostic("<repl>", &source);
        let program = parser::parse(&source).map_err(diag)?;
        let prepared =
            prepare(&program, SemanticMode::Inspection, Policy::Compatible).map_err(diag)?;
        let inspection = prepared
            .inspection()
            .ok_or("missing expression to inspect")?;
        match mode {
            Mode::Type => println!("{}", inspection.type_name),
            Mode::Schema => {
                let schema = inspection.schema.as_ref().ok_or_else(|| {
                    format!(":schema expects a table; found {}", inspection.type_name)
                })?;
                println!("{schema}");
            }
            Mode::View | Mode::Time => {
                let start = Instant::now();
                let out = prepared.view().map_err(diag)?;
                let elapsed = start.elapsed();
                print!("{}", out.stdout);
                if out.value.contains('\n') {
                    println!("{}\n: {}", out.value, inspection.type_name);
                } else {
                    println!("{} : {}", out.value, inspection.type_name);
                }
                if matches!(mode, Mode::Time) {
                    println!("elapsed: {:.3} ms", elapsed.as_secs_f64() * 1_000.0);
                }
                self.expressions.push(expression.to_owned());
            }
        }
        Ok(())
    }

    fn load(&mut self, arg: &str) -> Result<(), String> {
        let path = arg
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(arg);
        if path.is_empty() {
            return Err("usage: :load PATH".into());
        }
        let file = fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        let candidate = format!("{}{file}\n", self.declarations);
        let diagnostic = |e: flux_lang::Error| {
            if let Some(start) = e.start.checked_sub(self.declarations.len()) {
                flux_lang::Error::new(start, e.message).diagnostic(path, &file)
            } else {
                e.diagnostic("<repl>", &candidate)
            }
        };
        let program = parser::parse(&candidate).map_err(diagnostic)?;
        if program.entry.is_some() {
            return Err(format!(
                "{path}: :load accepts declarations only; run script entries with flux FILE"
            ));
        }
        prepare(&program, SemanticMode::Script, Policy::Compatible).map_err(diagnostic)?;
        self.declarations = candidate;
        println!("loaded {path}");
        Ok(())
    }

    fn submission(&mut self, line: &str) -> Result<(), String> {
        let lexed = lexer::lex(line).map_err(|e| e.diagnostic("<repl>", line))?;
        let terminated = lexed.last().is_some_and(|t| t.text == ";");
        let source = format!(
            "{}{line}\n{}\n",
            self.declarations,
            if terminated { "" } else { ";" }
        );
        if let Ok(program) = parser::parse(&source) {
            if program.entry.is_none() {
                prepare(&program, SemanticMode::Script, Policy::Compatible)
                    .map_err(|e| e.diagnostic("<repl>", &source))?;
                self.declarations = source;
                return Ok(());
            }
            let prepared = prepare(&program, SemanticMode::Script, Policy::Compatible)
                .map_err(|e| e.diagnostic("<repl>", &source))?;
            let output = prepared
                .run()
                .map_err(|e| e.diagnostic("<repl>", &source))?;
            print!("{}", output.stdout);
            return Ok(());
        }
        self.expression(line, Mode::View)
    }

    fn dispatch(&mut self, line: &str) -> Result<bool, String> {
        let (head, arg) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let arg = arg.trim();
        match head {
            ":q" | ":quit" if arg.is_empty() => return Ok(false),
            ":help" => println!("{REPL_HELP}"),
            ":type" => self.expression(arg, Mode::Type)?,
            ":schema" => self.expression(arg, Mode::Schema)?,
            ":time" => self.expression(arg, Mode::Time)?,
            ":load" => self.load(arg)?,
            ":history" => {
                for (i, expression) in self.expressions.iter().enumerate() {
                    println!("{}  {expression}", i + 1);
                }
            }
            ":replay" => {
                let i = arg
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| i.checked_sub(1))
                    .and_then(|i| self.expressions.get(i).cloned())
                    .ok_or_else(|| "usage: :replay N (an index shown by :history)".to_string())?;
                self.expression(&i, Mode::View)?;
            }
            // In particular `:q 4` remains an ordinary numeric conversion.
            _ => self.submission(line)?,
        }
        Ok(true)
    }
}

#[derive(Clone, Copy)]
enum Mode {
    Type,
    Schema,
    View,
    Time,
}

fn repl() -> Result<u8, String> {
    let stdin = io::stdin();
    let interactive = stdin.is_terminal();
    let mut lines = stdin.lock().lines();
    let mut session = Session::new();
    loop {
        if interactive {
            print!("flux> ");
            io::stdout().flush().map_err(|e| e.to_string())?;
        }
        let Some(line) = lines.next() else { break };
        let line = line.map_err(|e| e.to_string())?;
        let line = line.trim();
        if line.is_empty() || line.starts_with("--") {
            continue;
        }
        match session.dispatch(line) {
            Ok(false) => break,
            Ok(true) => {}
            Err(error) => eprintln!("{error}"),
        }
    }
    Ok(0)
}

fn cli() -> Result<u8, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mode, path) = match args.as_slice() {
        [] => return repl(),
        [flag] if flag == "--help" || flag == "-h" => {
            println!("Flux reference prototype\n\nUsage: flux [FILE | check FILE | hir FILE | run-hir FILE | phases FILE | fmt FILE | --version]\n\nRun without arguments for the REPL. fmt writes to stdout.");
            return Ok(0);
        }
        [flag] if flag == "--version" || flag == "-V" => {
            println!("flux {}", env!("CARGO_PKG_VERSION"));
            return Ok(0);
        }
        [file]
            if !matches!(
                file.as_str(),
                "check" | "fmt" | "hir" | "phases" | "run-hir"
            ) && !file.starts_with('-') =>
        {
            ("run", file)
        }
        [command, file]
            if matches!(
                command.as_str(),
                "check" | "fmt" | "hir" | "phases" | "run-hir"
            ) =>
        {
            (command.as_str(), file)
        }
        _ => return Err("usage: flux [FILE | check FILE | fmt FILE]".into()),
    };
    let source = fs::read_to_string(path).map_err(|err| format!("{path}: {err}"))?;
    let diagnostic = |err: flux_lang::Error| err.diagnostic(path, &source);
    match mode {
        "check" | "hir" => {
            let program = parser::parse(&source).map_err(diagnostic)?;
            let strict = mode == "hir";
            let policy = if strict {
                Policy::TypedOnly
            } else {
                Policy::Compatible
            };
            let prepared = prepare(&program, SemanticMode::Script, policy).map_err(diagnostic)?;
            if strict {
                println!(
                    "{:#?}",
                    prepared.hir().expect("strict compilation yields HIR")
                );
            }
        }
        "phases" => {
            let start = Instant::now();
            let program = parser::parse(&source).map_err(diagnostic)?;
            let parse = start.elapsed();
            let mut timing = Timings::default();
            let plan = prepare_timed(
                &program,
                SemanticMode::Script,
                Policy::TypedOnly,
                &mut timing,
            )
            .map_err(diagnostic)?;
            let start = Instant::now();
            if program.entry.is_some() {
                let out = plan.run().map_err(diagnostic)?;
                std::io::stdout()
                    .write_all(out.stdout.as_bytes())
                    .map_err(|e| e.to_string())?;
            }
            let run = start.elapsed();
            eprintln!(
                "parse_ns	{}
resolve_ns	{}
infer_ns	{}
elaborate_ns	{}
eval_ns	{}",
                parse.as_nanos(),
                timing.resolution.as_nanos(),
                timing.inference.as_nanos(),
                timing.elaboration.as_nanos(),
                run.as_nanos()
            );
        }
        "fmt" => print!("{}", format_source(&source).map_err(diagnostic)?),
        _ => {
            let program = parser::parse(&source).map_err(diagnostic)?;
            if program.entry.is_none() {
                return Err(format!(
                    "{path}: no executable !{{ ... }}; block (try flux check)"
                ));
            }
            let policy = if mode == "run-hir" {
                Policy::TypedOnly
            } else {
                Policy::Compatible
            };
            let output = prepare(&program, SemanticMode::Script, policy)
                .map_err(diagnostic)?
                .run()
                .map_err(diagnostic)?;
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
