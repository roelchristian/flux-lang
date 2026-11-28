use super::config::runtime_options;
use flux_lang::{
    lexer, modules,
    parser::needs_continuation,
    semantic::{prepare_modules, Mode as SemanticMode, Policy},
    RuntimeOptions,
};
use std::{
    io::{self, IsTerminal, Write},
    path::PathBuf,
    time::Instant,
};

const REPL_HELP: &str =
    "Use one line or continue an open delimiter on another line. Expressions print their value and type.
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
    declarations: Vec<(PathBuf, String)>,
    expressions: Vec<String>,
    options: RuntimeOptions,
}

impl Session {
    fn new(options: RuntimeOptions) -> Self {
        Self {
            declarations: vec![(PathBuf::from("<repl>"), String::from("<< \"io\" : io;\n"))],
            expressions: Vec::new(),
            options,
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
        let source = format!("!{{\n{expression}\n}};");
        let loaded = self.candidate(PathBuf::from("<repl>"), source)?;
        let diag = |error: flux_lang::Error| loaded.diagnostic(&error);
        let prepared = prepare_modules(
            &loaded.program,
            SemanticMode::Inspection,
            Policy::TypedOnly,
            &loaded.module_inputs,
        )
        .map_err(diag)?;
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
                let out = prepared
                    .view_with_host_options(
                        &mut flux_lang::FsInput,
                        &mut io::stdin().lock(),
                        &[],
                        &mut io::stdout(),
                        self.options,
                    )
                    .map_err(diag)?;
                let elapsed = start.elapsed();
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
        let (origin, file) = modules::read_fragment(path).map_err(|error| error.diagnostic())?;
        let loaded = self.candidate(origin.clone(), file.clone())?;
        if loaded.program.entry.is_some() {
            return Err(format!(
                "{path}: :load accepts declarations only; run script entries with flux FILE"
            ));
        }
        prepare_modules(
            &loaded.program,
            SemanticMode::Script,
            Policy::TypedOnly,
            &loaded.module_inputs,
        )
        .map_err(|error| loaded.diagnostic(&error))?;
        self.declarations.push((origin, file));
        println!("loaded {path}");
        Ok(())
    }

    fn candidate(&self, origin: PathBuf, source: String) -> Result<modules::LoadedProgram, String> {
        let mut fragments = self.declarations.clone();
        fragments.push((origin, source));
        modules::load_virtual_parts("<repl>", &fragments).map_err(|error| error.diagnostic())
    }

    fn submission(&mut self, line: &str) -> Result<(), String> {
        let lexed = lexer::lex(line).map_err(|e| e.diagnostic("<repl>", line))?;
        let terminated = lexed.last().is_some_and(|t| t.text == ";");
        let source = format!("{line}\n{}\n", if terminated { "" } else { ";" });
        let origin = PathBuf::from("<repl>");
        match self.candidate(origin.clone(), source.clone()) {
            Ok(loaded) => {
                let prepared = prepare_modules(
                    &loaded.program,
                    SemanticMode::Script,
                    Policy::TypedOnly,
                    &loaded.module_inputs,
                )
                .map_err(|error| loaded.diagnostic(&error))?;
                if loaded.program.entry.is_none() {
                    self.declarations.push((origin, source));
                } else {
                    prepared
                        .run_with_host_options(
                            &mut flux_lang::FsInput,
                            &mut io::stdin().lock(),
                            &[],
                            &mut io::stdout(),
                            self.options,
                        )
                        .map_err(|error| loaded.diagnostic(&error))?;
                }
                Ok(())
            }
            Err(error) if lexed.first().is_some_and(|token| token.text == "<<") => Err(error),
            Err(_) => self.expression(line, Mode::View),
        }
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

pub(super) fn repl() -> Result<u8, String> {
    let stdin = io::stdin();
    let interactive = stdin.is_terminal();
    const MAX_PENDING_BYTES: usize = 64 * 1024;
    let mut line = String::new();
    let mut pending = String::new();
    let mut session = Session::new(runtime_options()?);
    loop {
        if interactive {
            print!(
                "{}",
                if pending.is_empty() {
                    "flux> "
                } else {
                    "...> "
                }
            );
            io::stdout().flush().map_err(|e| e.to_string())?;
        }
        line.clear();
        if stdin.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            if !pending.is_empty() {
                eprintln!("incomplete REPL submission at end of input");
            }
            break;
        }
        if pending.is_empty() && (line.trim().is_empty() || line.trim_start().starts_with("--")) {
            continue;
        }
        if pending.is_empty() && line.trim_start().starts_with(":load ") {
            // Filesystem paths are not Flux expression tokens.
            if let Err(error) = session.dispatch(line.trim()) {
                eprintln!("{error}");
            }
            continue;
        }
        if pending.len() + line.len() > MAX_PENDING_BYTES {
            pending.clear();
            eprintln!("REPL submission exceeds 64 KiB");
            continue;
        }
        pending.push_str(&line);
        if needs_continuation(&pending) {
            continue;
        }
        let proceed = session.dispatch(pending.trim());
        pending.clear();
        match proceed {
            Ok(false) => break,
            Ok(true) => {}
            Err(error) => eprintln!("{error}"),
        }
    }
    Ok(0)
}
