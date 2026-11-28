//! Parse and lower tacit function syntax without adding AST or runtime kinds.
use super::{
    originated, Binding, Expr, FormattingApplication, Origin, PResult, Parser, Stmt,
    DIRECT_ARITY_LIMIT,
};
use crate::{lexer::Token, Error};
use std::ops::Range;

const INPUT: &str = "\0tacit-input";
const CAPTURE_PREFIX: &str = "\0tacit-capture:";

const MAX_DEPTH: usize = 64;
const MAX_BRANCHES: usize = 64;

#[derive(Clone, Copy)]
struct Context {
    arity: usize,
    extended: bool,
}

impl Context {
    const LEGACY: Self = Self {
        arity: 1,
        extended: false,
    };

    // An implicit function operand, like the combiner of a fork, is not
    // itself an all-input branch. Its old unary rule remains in force.
    fn function(self) -> Self {
        Self { arity: 1, ..self }
    }
}

struct Tacit {
    kind: Kind,
    span: Range<usize>,
}

enum Kind {
    Input(usize),
    Reference(String),
    Value(String),
    // Explicit nested function block, with its detached postorder origins.
    Nested(Expr, Vec<Origin>),
    Capture(String),
    Compose(Box<Tacit>, Box<Tacit>),
    Fork(Box<Tacit>, Vec<Tacit>),
    Hook(Box<Tacit>, Box<Tacit>),
}

struct Capture {
    start: usize,
    name: String,
    body: Expr,
    origins: Vec<Origin>,
}

pub(crate) fn is_capture(stmt: &Stmt) -> bool {
    matches!(stmt, Stmt::Def { name, params, .. }
        if params.is_empty() && name.starts_with(CAPTURE_PREFIX))
}

fn input_name(index: usize) -> String {
    if index == 0 {
        INPUT.into()
    } else {
        format!("{INPUT}:{index}")
    }
}

fn input(index: usize, origins: &mut Vec<Origin>, span: Option<Range<usize>>) -> Expr {
    originated(Expr::Name(input_name(index)), span, origins)
}

fn apply(f: Expr, arguments: Vec<Expr>, origins: &mut Vec<Origin>) -> Expr {
    if arguments.len() == 1 {
        Expr::Call("@".into(), vec![f, arguments.into_iter().next().unwrap()])
    } else {
        let args = originated(Expr::Tuple(arguments), None, origins);
        Expr::Call("@@".into(), vec![f, args])
    }
}

fn branch(tacit: Tacit, cx: Context, origins: &mut Vec<Origin>) -> Expr {
    let Tacit { kind, span } = tacit;
    let body = match kind {
        Kind::Input(index) => return input(index, origins, Some(span)),
        Kind::Capture(name) => return originated(Expr::Name(name), Some(span), origins),
        Kind::Compose(g, h) => {
            let g = function(*g, cx.function(), origins, true);
            let h = branch(*h, cx, origins);
            apply(g, vec![h], origins)
        }
        Kind::Fork(f, branches) => {
            let f = function(*f, cx.function(), origins, true);
            let args = branches
                .into_iter()
                .map(|g| branch(g, cx, origins))
                .collect();
            apply(f, args, origins)
        }
        Kind::Hook(f, h) => {
            let f = function(*f, cx.function(), origins, true);
            let x = input(0, origins, None);
            let h = branch(*h, cx, origins);
            apply(f, vec![x, h], origins)
        }
        Kind::Reference(_) | Kind::Value(_) | Kind::Nested(..) => {
            let f = match kind {
                Kind::Reference(name) => Expr::Ref(name),
                Kind::Value(name) => Expr::Name(name),
                Kind::Nested(expr, detached) => {
                    origins.extend(detached);
                    expr
                }
                _ => unreachable!(),
            };
            let f = originated(f, Some(span.clone()), origins);
            let args = (0..cx.arity)
                .map(|index| input(index, origins, None))
                .collect();
            apply(f, args, origins)
        }
    };
    originated(body, Some(span), origins)
}

// A function-position atom denotes the function rather than consuming the
// outer arguments. The legacy value-atom eta expansion is intentionally
// retained for old unary forms.
fn function(tacit: Tacit, cx: Context, origins: &mut Vec<Origin>, record_root: bool) -> Expr {
    let Tacit { kind, span } = tacit;
    let expr = match kind {
        Kind::Reference(name) => Expr::Ref(name),
        Kind::Value(name) if cx.extended => Expr::Name(name),
        Kind::Capture(name) => Expr::Name(name),
        Kind::Nested(expr, detached) => {
            origins.extend(detached);
            expr
        }
        _ => closure(
            Tacit {
                kind,
                span: span.clone(),
            },
            cx,
            origins,
        ),
    };
    if record_root {
        originated(expr, Some(span), origins)
    } else {
        expr
    }
}

fn closure(tacit: Tacit, cx: Context, origins: &mut Vec<Origin>) -> Expr {
    Expr::Lambda(
        (0..cx.arity).map(input_name).collect(),
        Box::new(branch(tacit, cx, origins)),
    )
}

pub(super) fn identity(span: Range<usize>, origins: &mut Vec<Origin>) -> Expr {
    function(
        Tacit {
            kind: Kind::Input(0),
            span,
        },
        Context::LEGACY,
        origins,
        false,
    )
}

fn checked_number(token: &Token) -> Option<usize> {
    token
        .text
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| token.text.parse::<usize>().ok())
        .flatten()
}

impl Parser {
    pub(super) fn tacit_block(&mut self, level: usize) -> PResult<Expr> {
        let cx = if self.at(self.pos + 1) == ":" {
            let arity = self.next()?;
            self.expect(":")?;
            Context {
                arity: checked_number(&arity)
                    .filter(|n| (1..=DIRECT_ARITY_LIMIT).contains(n))
                    .ok_or_else(|| {
                        Error::new(
                            arity.start,
                            format!("tacit arity must be an unsuffixed integer from 1 to {DIRECT_ARITY_LIMIT}"),
                        )
                    })?,
                extended: true,
            }
        } else {
            Context::LEGACY
        };
        let mut captures = Vec::new();
        let tacit = self.tacit(level, cx, &mut captures)?;
        if !matches!(self.peek(), "" | "}" | ";") {
            return self.err("unexpected additional tacit operand; expected `}`");
        }
        self.expect("}")?;
        if !cx.extended {
            return Ok(function(tacit, cx, &mut self.origins, false));
        }

        // Capture sites were visited in source order. Initialize them in
        // that order outside the curried closure, even for unused branches.
        let defs: Vec<_> = captures
            .into_iter()
            .map(|capture| {
                self.origins.extend(capture.origins);
                Stmt::Def {
                    name: capture.name,
                    params: Vec::new(),
                    param_types: Vec::new(),
                    result_type: None,
                    body: capture.body,
                    start: capture.start,
                }
            })
            .collect();
        let span = tacit.span.clone();
        let expr = closure(tacit, cx, &mut self.origins);
        if defs.is_empty() {
            Ok(expr)
        } else {
            let expr = originated(expr, Some(span), &mut self.origins);
            Ok(Expr::Block(defs, Some(Box::new(expr))))
        }
    }

    fn tacit(&mut self, level: usize, cx: Context, captures: &mut Vec<Capture>) -> PResult<Tacit> {
        if level >= MAX_DEPTH || self.depth + level >= 256 {
            return self.err("tacit expression depth limit exceeded");
        }
        let token = self.next()?;
        let start = token.start;
        let adjacent = self.offset() == token.end;
        let kind = match token.text.as_str() {
            "." if cx.arity == 1 => Kind::Input(0),
            "." => {
                return Err(Error::new(
                    start,
                    "bare `.` needs a unary tacit input; use `$1` etc.",
                ));
            }
            "(" => {
                let inner = self.tacit(level + 1, cx, captures)?;
                self.expect(")")?;
                inner.kind
            }
            "&{" if cx.extended => {
                let mark = self.origins.len();
                let nested = self.tacit_block(level + 1)?;
                Kind::Nested(nested, self.origins.split_off(mark))
            }
            "@." | "@*" | "@?" => self.tacit_operator(&token, level + 1, cx, captures)?,
            "@" if cx.extended && adjacent && self.eat("{") => {
                self.tacit_fork(level + 1, cx, captures)?
            }
            "@" if cx.extended && adjacent && self.eat("=") => {
                let mark = self.origins.len();
                let body = self.expr(false)?;
                let name = format!("{CAPTURE_PREFIX}{start}");
                captures.push(Capture {
                    start,
                    name: name.clone(),
                    body,
                    origins: self.origins.split_off(mark),
                });
                Kind::Capture(name)
            }
            "$" if cx.extended => {
                let index = self
                    .toks
                    .get(self.pos)
                    .filter(|tok| tok.start == token.end)
                    .and_then(checked_number)
                    .ok_or_else(|| {
                        Error::new(start, "tacit argument index must be adjacent, e.g. `$1`")
                    })?;
                if !(1..=cx.arity).contains(&index) {
                    return Err(Error::new(
                        start,
                        format!("tacit argument `${index}` is outside arity {}", cx.arity),
                    ));
                }
                self.pos += 1;
                Kind::Input(index - 1)
            }
            "!" => {
                return Err(Error::new(
                    start,
                    "effect execution is not a tacit function",
                ))
            }
            "$" => {
                return Err(Error::new(
                    start,
                    "pipeline placeholder is not tacit identity",
                ))
            }
            "&" => {
                return Err(Error::new(
                    start,
                    "use a bare callable or `.` inside a tacit block",
                ))
            }
            atom => match self.lookup(atom) {
                Some(Binding::Callable(n) | Binding::Constructor(n)) if n > 0 => {
                    Kind::Reference(self.canonical(atom))
                }
                Some(Binding::Value) => Kind::Value(self.canonical(atom)),
                _ => {
                    return Err(Error::new(
                        start,
                        format!("expected tacit callable or `.`, found `{atom}`"),
                    ))
                }
            },
        };
        Ok(Tacit {
            kind,
            span: start..self.toks[self.pos - 1].end,
        })
    }

    fn tacit_operator(
        &mut self,
        token: &Token,
        level: usize,
        cx: Context,
        captures: &mut Vec<Capture>,
    ) -> PResult<Kind> {
        let operator = token.text.as_str();
        let description = match operator {
            "@." => "composition requires two function operands",
            "@*" => "fork requires three function operands",
            _ if cx.arity != 1 => {
                return Err(Error::new(
                    token.start,
                    "tacit hook `@?` requires unary input",
                ));
            }
            _ => "hook requires two function operands",
        };
        let mut operand = |parser: &mut Self, cx| {
            if matches!(parser.peek(), "" | "}" | ")" | ";") {
                return Err(Error::new(token.start, description));
            }
            parser.tacit(level, cx, captures)
        };
        let first = operand(self, cx.function())?;
        let second = operand(self, cx)?;
        let third = if operator == "@*" {
            Some(operand(self, cx)?)
        } else {
            None
        };
        if let Some(layout) = &mut self.formatting_applications {
            let operands: Vec<_> = [&first, &second]
                .into_iter()
                .chain(third.iter())
                .map(|t| t.span.clone())
                .collect();
            layout.push(FormattingApplication {
                span: token.start..operands.last().unwrap().end,
                operator: token.start..token.end,
                operands,
                tacit: true,
            });
        }
        let first = Box::new(first);
        Ok(if let Some(third) = third {
            Kind::Fork(first, vec![second, third])
        } else if operator == "@." {
            Kind::Compose(first, Box::new(second))
        } else {
            Kind::Hook(first, Box::new(second))
        })
    }

    fn tacit_fork(
        &mut self,
        level: usize,
        cx: Context,
        captures: &mut Vec<Capture>,
    ) -> PResult<Kind> {
        let combiner = self.tacit(level, cx.function(), captures)?;
        self.expect(";")?;
        let mut branches = Vec::new();
        loop {
            if matches!(self.peek(), "" | "}" | ";") {
                return self.err("tacit fork requires a branch after `;`");
            }
            if branches.len() >= MAX_BRANCHES {
                return self.err(format!(
                    "tacit fork supports at most {MAX_BRANCHES} branches"
                ));
            }
            branches.push(self.tacit(level, cx, captures)?);
            if self.eat("}") {
                break;
            }
            self.expect(";")?;
        }
        Ok(Kind::Fork(Box::new(combiner), branches))
    }
}
