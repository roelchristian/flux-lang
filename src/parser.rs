use crate::lexer::{lex, Token};
pub use crate::Error;
use crate::{builtins, errors041};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    hash::{DefaultHasher, Hash, Hasher},
    ops::Range,
};

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub defs: Vec<Stmt>,
    pub imports: Vec<String>,
    pub entry: Option<Expr>,
    /// Zero-based byte offset of the entry’s opening `!`, if any.
    pub entry_start: Option<usize>,
    /// Explicit (class, type variable) qualifiers by declaration byte offset.
    pub constraints: BTreeMap<usize, Vec<(String, String)>>,
    pub exports: Vec<String>,
    origins: Vec<Origin>,
}
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Def {
        name: String,
        params: Vec<String>,
        param_types: Vec<Option<String>>,
        result_type: Option<String>,
        body: Expr,
        start: usize,
    },
    Sum {
        name: String,
        variants: Vec<Variant>,
        start: usize,
    },
    Schema {
        name: String,
        fields: Vec<(String, String)>,
        start: usize,
    },
    Effect {
        name: Option<String>,
        body: Expr,
        start: usize,
    },
}
#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub name: String,
    pub payload: Vec<String>,
    pub start: usize,
}
#[derive(Debug, Clone, PartialEq)]
pub struct RecoveryCase {
    pub pattern: RecoveryPattern,
    pub body: Expr,
    pub start: usize,
}
#[derive(Debug, Clone, PartialEq, Hash)]
pub enum RecoveryPattern {
    Wildcard,
    Bind(String),
    Constructor(String, Vec<String>),
}
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Int(i128),
    Str(String),
    Bool(bool),
    Unit,
    Name(String),
    Call(String, Vec<Expr>),
    Ref(String),
    Tuple(Vec<Expr>),
    Array(Vec<Expr>),
    Cases(Vec<(Expr, Expr)>, Box<Expr>),
    Lambda(Vec<String>, Box<Expr>),
    TypedLambda(Vec<String>, Vec<Option<String>>, Box<Expr>),
    Pipeline(Vec<Expr>),
    Record(Vec<(String, Expr)>),
    Block(Vec<Stmt>, Option<Box<Expr>>),
    Action(Vec<Stmt>, Option<Box<Expr>>),
    Fallible(Vec<Stmt>, Option<Box<Expr>>),
    Raise(Box<Expr>),
    Propagate(Box<Expr>),
    Recover(Box<Expr>, Vec<RecoveryCase>),
    Num(String),
    Char(char),
    NamedRecord(String, Vec<(String, Expr)>),
}

// Post-order traversal mirrors the parser's expression completion order.
// These entries are owned by Program, so cloning/moving it does not move or
// invalidate any pointer keys. A later resolver associates current AST nodes.
#[derive(Debug, Clone, PartialEq)]
struct Origin {
    signature: u64,
    span: Option<Range<usize>>,
}

impl Origin {
    fn new(expr: &Expr, span: Option<Range<usize>>) -> Self {
        Self {
            signature: expression_signature(expr),
            span,
        }
    }
}
fn expression_signature(expr: &Expr) -> u64 {
    let mut hash = DefaultHasher::new();
    std::mem::discriminant(expr).hash(&mut hash);
    match expr {
        Expr::Int(n) => n.hash(&mut hash),
        Expr::Char(c) => c.hash(&mut hash),
        Expr::Bool(b) => b.hash(&mut hash),
        Expr::Unit => {}
        Expr::Name(n) | Expr::Ref(n) | Expr::Str(n) | Expr::Num(n) => n.hash(&mut hash),
        Expr::Call(name, args) => {
            name.hash(&mut hash);
            args.len().hash(&mut hash);
        }
        Expr::Tuple(es) | Expr::Array(es) | Expr::Pipeline(es) => es.len().hash(&mut hash),
        Expr::Cases(arms, _) => arms.len().hash(&mut hash),
        Expr::Raise(_) | Expr::Propagate(_) => {}
        Expr::Recover(_, arms) => {
            for arm in arms {
                arm.pattern.hash(&mut hash);
                arm.start.hash(&mut hash);
            }
        }

        Expr::Lambda(ps, _) => ps.hash(&mut hash),
        Expr::TypedLambda(ps, ts, _) => {
            ps.hash(&mut hash);
            ts.hash(&mut hash);
        }
        Expr::NamedRecord(name, fs) => {
            name.hash(&mut hash);
            for (n, _) in fs {
                n.hash(&mut hash);
            }
        }
        Expr::Record(fs) => {
            for (n, _) in fs {
                n.hash(&mut hash);
            }
        }
        Expr::Block(stmts, last) | Expr::Action(stmts, last) | Expr::Fallible(stmts, last) => {
            stmts.len().hash(&mut hash);
            last.is_some().hash(&mut hash);
            for stmt in stmts {
                std::mem::discriminant(stmt).hash(&mut hash);
                match stmt {
                    Stmt::Def { name, start, .. }
                    | Stmt::Schema { name, start, .. }
                    | Stmt::Sum { name, start, .. } => {
                        name.hash(&mut hash);
                        start.hash(&mut hash);
                    }
                    Stmt::Effect { name, start, .. } => {
                        name.hash(&mut hash);
                        start.hash(&mut hash);
                    }
                }
            }
        }
    }
    hash.finish()
}
impl Program {
    pub(crate) fn expression_origins(&self) -> Vec<(&Expr, Range<usize>)> {
        fn stmt<'p>(s: &'p Stmt, nodes: &mut Vec<&'p Expr>) {
            match s {
                Stmt::Def { body, .. } | Stmt::Effect { body, .. } => expr(body, nodes),
                Stmt::Schema { .. } | Stmt::Sum { .. } => {}
            }
        }
        fn expr<'p>(e: &'p Expr, nodes: &mut Vec<&'p Expr>) {
            match e {
                Expr::Call(_, es) | Expr::Tuple(es) | Expr::Array(es) | Expr::Pipeline(es) => {
                    for x in es {
                        expr(x, nodes);
                    }
                }
                Expr::Cases(pairs, otherwise) => {
                    for (a, b) in pairs {
                        expr(a, nodes);
                        expr(b, nodes);
                    }
                    expr(otherwise, nodes);
                }
                Expr::Lambda(_, body) | Expr::TypedLambda(_, _, body) => expr(body, nodes),
                Expr::Record(fs) | Expr::NamedRecord(_, fs) => {
                    for (_, x) in fs {
                        expr(x, nodes);
                    }
                }
                Expr::Raise(body) | Expr::Propagate(body) => expr(body, nodes),
                Expr::Recover(body, arms) => {
                    expr(body, nodes);
                    for a in arms {
                        expr(&a.body, nodes);
                    }
                }
                Expr::Action(ss, last) | Expr::Block(ss, last) | Expr::Fallible(ss, last) => {
                    for s in ss {
                        stmt(s, nodes);
                    }
                    if let Some(x) = last {
                        expr(x, nodes);
                    }
                }
                Expr::Int(_)
                | Expr::Str(_)
                | Expr::Bool(_)
                | Expr::Char(_)
                | Expr::Unit
                | Expr::Num(_)
                | Expr::Name(_)
                | Expr::Ref(_) => {}
            }
            nodes.push(e);
        }
        let mut nodes = Vec::with_capacity(self.origins.len());
        for s in &self.defs {
            stmt(s, &mut nodes);
        }
        if let Some(e) = &self.entry {
            expr(e, &mut nodes);
        }
        if nodes.len() != self.origins.len()
            || nodes
                .iter()
                .zip(&self.origins)
                .any(|(node, origin)| expression_signature(node) != origin.signature)
        {
            // A programmatic AST edit invalidates the original position map.
            return Vec::new();
        }
        nodes
            .into_iter()
            .zip(&self.origins)
            .filter_map(|(node, origin)| origin.span.clone().map(|span| (node, span)))
            .collect()
    }
}

type PResult<T> = Result<T, Error>;
#[derive(Copy, Clone, Debug)]
enum Binding {
    Callable(usize),
    Constructor(usize),
    Value,
    Record,
}
pub fn parse(source: &str) -> PResult<Program> {
    Parser {
        toks: lex(source)?,
        pos: 0,
        eof: source.len(),
        scopes: Vec::new(),
        standard: HashMap::new(),
        aliases: HashMap::new(),
        depth: 0,
        constraints: BTreeMap::new(),
        origins: Vec::new(),
    }
    .program()
}
struct Parser {
    toks: Vec<Token>,
    pos: usize,
    eof: usize,
    scopes: Vec<HashMap<String, Binding>>,
    standard: HashMap<String, Binding>,
    aliases: HashMap<String, String>,
    depth: usize,
    constraints: BTreeMap<usize, Vec<(String, String)>>,
    origins: Vec<Origin>,
}
fn is_ident(s: &str) -> bool {
    let mut bytes = s.bytes();
    bytes.next().is_some_and(|b| b.is_ascii_lowercase())
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}
fn close(s: &str) -> Option<&'static str> {
    match s {
        "(" => Some(")"),
        "[" | "#[" => Some("]"),
        "{" | "!{" | "#{" | "{." | "?{" | ".{" | "|{" | "^{" => Some("}"),
        _ => None,
    }
}
impl Parser {
    fn at(&self, i: usize) -> &str {
        self.toks.get(i).map_or("", |t| t.text.as_str())
    }
    fn peek(&self) -> &str {
        self.at(self.pos)
    }
    fn offset(&self) -> usize {
        self.toks.get(self.pos).map_or(self.eof, |t| t.start)
    }
    fn err<T>(&self, m: impl Into<String>) -> PResult<T> {
        Err(Error::new(self.offset(), m.into()))
    }
    fn eat(&mut self, s: &str) -> bool {
        if self.peek() == s {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, s: &str) -> PResult<()> {
        if self.eat(s) {
            Ok(())
        } else {
            self.err(format!("expected `{s}`, found {}", self.found()))
        }
    }
    fn found(&self) -> String {
        if self.peek().is_empty() {
            "end of input".into()
        } else {
            format!("`{}`", self.peek())
        }
    }
    fn next(&mut self) -> PResult<Token> {
        let t = self
            .toks
            .get(self.pos)
            .cloned()
            .ok_or_else(|| Error::new(self.eof, "expected token, found end of input"))?;
        self.pos += 1;
        Ok(t)
    }
    fn ident(&mut self) -> PResult<String> {
        if is_ident(self.peek()) {
            Ok(self.next()?.text)
        } else {
            self.err(format!("expected identifier, found {}", self.found()))
        }
    }
    fn lookup(&self, n: &str) -> Option<Binding> {
        self.scopes
            .iter()
            .rev()
            .find_map(|s| s.get(n).copied())
            .or_else(|| self.standard.get(n).copied())
            .or_else(|| {
                if n.strip_prefix('.').is_some_and(|x| {
                    is_ident(x) || (!x.is_empty() && x.bytes().all(|b| b.is_ascii_digit()))
                }) {
                    Some(Binding::Callable(1))
                } else {
                    builtins::prelude_arity(n).map(Binding::Callable)
                }
            })
    }
    fn canonical(&self, n: &str) -> String {
        if let Some((alias, member)) = n.split_once('.') {
            if let Some(path) = self.aliases.get(alias) {
                let path = if path == "table" { "df" } else { path };
                return format!("{path}.{member}");
            }
        }
        n.to_owned()
    }
    fn scope(&mut self) -> &mut HashMap<String, Binding> {
        self.scopes.last_mut().expect("parser scope")
    }
    fn end_of_group(&self, from: usize) -> Option<usize> {
        let mut ends = vec![close(self.at(from))?];
        for i in from + 1..self.toks.len() {
            let s = self.at(i);
            if let Some(c) = close(s) {
                ends.push(c);
            } else if matches!(s, ")" | "]" | "}") {
                if ends.pop() != Some(s) {
                    return None;
                }
                if ends.is_empty() {
                    return Some(i);
                }
            }
        }
        None
    }
    // Find a delimiter without looking through nested parenthesis, sequence or brace groups.
    fn find(&self, mut i: usize, wanted: &[&str], stops: &[&str]) -> Option<usize> {
        while i < self.toks.len() {
            let s = self.at(i);
            if wanted.contains(&s) {
                return Some(i);
            }
            if stops.contains(&s) || matches!(s, ")" | "]" | "}") {
                return None;
            }
            if close(s).is_some() {
                i = self.end_of_group(i)? + 1;
            } else {
                i += 1;
            }
        }
        None
    }
    fn def_at(&self, i: usize) -> bool {
        if !is_ident(self.at(i)) {
            return false;
        }
        match self.at(i + 1) {
            ":=" | "::" => true,
            ":" | ":#" => self.find(i + 2, &[":="], &[";"]).is_some(),
            "(" => self.end_of_group(i + 1).is_some_and(|j| {
                self.at(j + 1) == ":="
                    || (matches!(self.at(j + 1), ":" | ":#")
                        && self.find(j + 2, &[":="], &[";"]).is_some())
            }),
            _ => false,
        }
    }
    fn prepass(&self, mut i: usize, root: bool) -> PResult<HashMap<String, Binding>> {
        let mut names = HashSet::new();
        let mut type_names = HashSet::new();
        let mut hoisted = HashMap::new();
        loop {
            let n = self.at(i).to_owned();
            if matches!(n.as_str(), "" | "}") {
                break;
            }
            if self.def_at(i) {
                let is_type = self.at(i + 1) == "::";
                let is_sum = is_type && self.at(i + 2) == "{" && self.at(i + 3) == "|";
                if is_type && !type_names.insert(n.clone()) {
                    return Err(Error::new(
                        self.toks[i].start,
                        format!("duplicate type `{n}`"),
                    ));
                }
                let b = match self.at(i + 1) {
                    "::" if is_sum => {
                        let mut j = i + 3;
                        while self.at(j) == "|" {
                            let (v, next) = self.variant_at(j)?;
                            if !names.insert(v.name.clone()) {
                                return Err(Error::new(
                                    v.start,
                                    format!("duplicate binding `{}`", v.name),
                                ));
                            }
                            hoisted.insert(v.name, Binding::Constructor(v.payload.len()));
                            j = next;
                        }
                        None
                    }
                    "::" => Some(Binding::Record),
                    "(" => {
                        let end = self.end_of_group(i + 1).ok_or_else(|| {
                            Error::new(self.toks[i].start, "unclosed parameter list")
                        })?;
                        let mut j = i + 2;
                        let mut count = usize::from(j < end);
                        while j < end {
                            if self.at(j) == "," {
                                count += 1;
                            }
                            if close(self.at(j)).is_some() {
                                j = self.end_of_group(j).unwrap_or(j);
                            }
                            j += 1;
                        }
                        Some(Binding::Callable(count))
                    }
                    _ if root => Some(Binding::Value),
                    _ => None,
                };
                if !is_sum && !names.insert(n.clone()) {
                    return Err(Error::new(
                        self.toks[i].start,
                        format!("duplicate binding `{n}`"),
                    ));
                }
                if let Some(b) = b {
                    hoisted.insert(n, b);
                }
            }
            let Some(end) = self.find(i, &[";"], &[]) else {
                break;
            };
            i = end + 1;
        }
        Ok(hoisted)
    }
    fn program(mut self) -> PResult<Program> {
        let hoisted = self.prepass(0, true)?;
        self.scopes.push(hoisted);
        let mut p = Program {
            defs: Vec::new(),
            imports: Vec::new(),
            entry: None,
            entry_start: None,
            constraints: BTreeMap::new(),
            exports: Vec::new(),
            origins: Vec::new(),
        };
        let mut exports = Vec::new();
        // Resolve top-level interfaces before any expression is parsed;
        // qualified callable arity is part of the interface, not the inferred type.
        let mut i = 0;
        while !self.at(i).is_empty() {
            let Some(end) = self.find(i, &[";"], &[]) else {
                break;
            };
            if self.at(i) == "<<" {
                self.pos = i + 1;
                let (path, alias, start) = self.import_decl()?;
                if self.aliases.insert(alias.clone(), path.clone()).is_some() {
                    return self.err(format!("duplicate module alias `{alias}`"));
                }
                for (name, arity) in builtins::module_items(&path)
                    .ok_or_else(|| Error::new(start, format!("unknown module `{path}`")))?
                {
                    self.scope()
                        .insert(format!("{alias}.{name}"), Binding::Callable(arity));
                }
                for owner in standard_owners(&path) {
                    for v in errors041::variants(owner) {
                        self.standard
                            .insert(v.name.to_owned(), Binding::Constructor(v.payload.len()));
                    }
                }
                p.imports.push(path);
            }
            i = end + 1;
        }
        self.pos = 0;
        while !self.peek().is_empty() {
            if self.eat("<<") {
                self.import_decl()?;
            } else if self.eat(">>") {
                self.expect("(")?;
                loop {
                    exports.push(self.ident()?);
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect(")")?;
                self.expect(";")?;
            } else if matches!(self.peek(), "!" | "!{") {
                let start = self.offset();
                let e = self.expr(false)?;
                if !matches!(e, Expr::Action(..)) {
                    return Err(Error::new(start, "expected an anonymous entry action"));
                }
                self.expect(";")?;
                if !self.peek().is_empty() {
                    return self.err("anonymous entry action must be last and unique");
                }
                p.entry = Some(e);
                p.entry_start = Some(start);
            } else if self.def_at(self.pos) {
                p.defs.push(self.definition(false)?);
            } else {
                return self.err(format!(
                    "expected declaration or anonymous entry action, found {}",
                    self.found()
                ));
            }
        }
        let mut seen = HashSet::new();
        for n in &exports {
            if !seen.insert(n.clone()) {
                return self.err(format!("duplicate export `{n}`"));
            }
            let d = p
                .defs
                .iter()
                .find(|d| matches!(d,Stmt::Def{name,..}|Stmt::Schema{name,..}|Stmt::Sum{name,..} if name==n));
            match d {
                Some(Stmt::Def {
                    result_type: Some(_),
                    param_types,
                    ..
                }) if param_types.iter().all(Option::is_some) => {}
                Some(Stmt::Schema { .. } | Stmt::Sum { .. }) => {}
                Some(_) => return self.err(format!("export `{n}` requires a complete signature")),
                None => return self.err(format!("unknown export `{n}`")),
            }
        }
        p.constraints = self.constraints;
        p.exports = exports;
        p.origins = self.origins;
        Ok(p)
    }
    fn import_decl(&mut self) -> PResult<(String, String, usize)> {
        let t = self.next()?;
        let raw = if t.text.starts_with('"') {
            decode(&t)?
        } else if is_ident(&t.text) {
            t.text
        } else {
            return Err(Error::new(
                t.start,
                "expected module name or quoted module path",
            ));
        };
        let stem = std::path::Path::new(&raw)
            .file_stem()
            .and_then(|x| x.to_str())
            .unwrap_or("");
        let alias = if self.eat(":") {
            self.ident()?
        } else {
            if !is_ident(stem) {
                return Err(Error::new(
                    t.start,
                    "module stem is not a valid default alias; supply an explicit alias",
                ));
            }
            stem.to_owned()
        };
        self.expect(";")?;
        // This prototype resolves only bundled interfaces, identified by their stem.
        Ok((stem.to_owned(), alias, t.start))
    }
    fn ty_until(&mut self, ends: &[&str]) -> PResult<String> {
        let start = self.pos;
        if let Some(j) = self.find(start, ends, &[";", "|", ":="]) {
            if start == j {
                return self.err("expected a type");
            }
            self.pos = j;
        } else {
            return self.err("expected a type followed by its delimiter");
        }
        Ok(self.toks[start..self.pos]
            .iter()
            .map(|t| match t.text.as_str() {
                "^?" => "^ ?",
                "^{" => "^ {",
                x => x,
            })
            .collect::<Vec<_>>()
            .join(" "))
    }
    // A declaration delimiter directly followed by a table marker shares spelling
    // with expression operator :#. Resolve it only in grammatical annotation position.
    fn annotation(&mut self, ends: &[&str]) -> PResult<Option<String>> {
        if self.eat(":") {
            return self.ty_until(ends).map(Some);
        }
        if self.eat(":#") {
            let positional =
                self.peek() == "[" && self.toks[self.pos - 1].end == self.toks[self.pos].start;
            let ty = self.ty_until(ends)?;
            return Ok(Some(if positional {
                format!("#{}", ty)
            } else {
                format!("# {ty}")
            }));
        }
        Ok(None)
    }
    fn parameters(&mut self) -> PResult<(Vec<String>, Vec<Option<String>>)> {
        self.expect("(")?;
        let mut names = Vec::new();
        let mut types = Vec::new();
        let mut seen = HashSet::new();
        loop {
            let n = self.ident()?;
            if !seen.insert(n.clone()) {
                return self.err(format!("duplicate parameter `{n}`"));
            }
            names.push(n);
            if let Some(ty) = self.annotation(&[",", ")"])? {
                types.resize(names.len(), Some(ty));
            }
            if !self.eat(",") {
                break;
            }
        }
        self.expect(")")?;
        if !types.is_empty() && types.len() != names.len() {
            return self.err("parameter annotations must cover every parameter");
        }
        types.resize(names.len(), None);
        Ok((names, types))
    }
    // One variant member's whitespace-separated type atoms carry the
    // constructor's prefix arity. Group composite types with parentheses.
    fn variant_at(&self, i: usize) -> PResult<(Variant, usize)> {
        let start = self.toks.get(i + 1).map_or(self.eof, |t| t.start);
        let name = self.at(i + 1);
        if !is_ident(name) {
            return Err(Error::new(start, "expected constructor name after `|`"));
        }
        let mut j = i + 2;
        let mut payload = Vec::new();
        while self.at(j) != ";" {
            let a = j;
            while matches!(self.at(j), "?" | "!" | "#") {
                j += 1;
            }
            if close(self.at(j)).is_some() {
                j = self
                    .end_of_group(j)
                    .map(|k| k + 1)
                    .ok_or_else(|| Error::new(start, "malformed constructor payload type"))?;
            } else if is_ident(self.at(j)) {
                j += 1;
            } else {
                return Err(Error::new(
                    self.toks.get(j).map_or(self.eof, |t| t.start),
                    "expected a constructor payload type or `;`",
                ));
            }
            payload.push(
                self.toks[a..j]
                    .iter()
                    .map(|t| match t.text.as_str() {
                        "^?" => "^ ?",
                        "^{" => "^ {",
                        x => x,
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
        Ok((
            Variant {
                name: name.to_owned(),
                payload,
                start,
            },
            j + 1,
        ))
    }

    fn definition(&mut self, effect: bool) -> PResult<Stmt> {
        let start = self.offset();
        let name = self.ident()?;
        if self.eat("::") {
            self.expect("{")?;
            if self.peek() == "|" {
                let mut variants = Vec::new();
                while self.peek() == "|" {
                    let (v, next) = self.variant_at(self.pos)?;
                    self.pos = next;
                    variants.push(v);
                }
                self.expect("}")?;
                self.expect(";")?;
                return Ok(Stmt::Sum {
                    name,
                    variants,
                    start,
                });
            }
            let mut fields = Vec::new();
            let mut seen = HashSet::new();
            while !self.eat("}") {
                let n = self.ident()?;
                if !seen.insert(n.clone()) {
                    return self.err(format!("duplicate schema field `{n}`"));
                }
                let ty = self
                    .annotation(&[";"])?
                    .ok_or_else(|| Error::new(self.offset(), "expected record field type"))?;
                fields.push((n, ty));
                self.expect(";")?;
            }
            self.expect(";")?;
            return Ok(Stmt::Schema {
                name,
                fields,
                start,
            });
        }
        let (params, param_types) = if self.peek() == "(" {
            self.parameters()?
        } else {
            (Vec::new(), Vec::new())
        };
        let result_type = self.annotation(&[":=", "|"])?;
        if self.eat("|") {
            if params.is_empty() || result_type.is_none() || param_types.iter().any(Option::is_none)
            {
                return self.err("type constraints require a complete named function signature");
            }
            let mut qualifiers = Vec::new();
            loop {
                let class = self.ident()?;
                if !matches!(class.as_str(), "num" | "eq" | "ord" | "hash") {
                    return self.err(format!("unknown type constraint `{class}`"));
                }
                let var = self.next()?;
                if !var.text.strip_prefix(char::from(39)).is_some_and(is_ident) {
                    return Err(Error::new(var.start, "expected a quantified type variable"));
                }
                let pair = (class, var.text);
                if qualifiers.contains(&pair) {
                    return Err(Error::new(var.start, "duplicate type constraint"));
                }
                qualifiers.push(pair);
                if !self.eat(",") {
                    break;
                }
            }
            self.constraints.insert(start, qualifiers);
        }
        if result_type.is_some() && param_types.iter().any(Option::is_none) {
            return self.err("annotated function result requires annotated parameters");
        }
        self.expect(":=")?;
        let body = if params.is_empty() {
            self.expr(effect)?
        } else {
            self.scopes
                .push(params.iter().map(|p| (p.clone(), Binding::Value)).collect());
            let r = self.expr(false);
            self.scopes.pop();
            r?
        };
        self.expect(";")?;
        if params.is_empty() {
            self.scope().insert(name.clone(), Binding::Value);
        }
        Ok(Stmt::Def {
            name,
            params,
            param_types,
            result_type,
            body,
            start,
        })
    }
    fn expr(&mut self, effect: bool) -> PResult<Expr> {
        if self.depth >= 256 {
            return self.err("expression depth limit exceeded");
        }
        let start = self.offset();
        self.depth += 1;
        let r = self.atom(effect);
        self.depth -= 1;
        if let Ok(expr) = &r {
            let end = self.toks[self.pos - 1].end;
            self.origins.push(Origin::new(expr, Some(start..end)));
        }
        r
    }
    fn atom(&mut self, effect: bool) -> PResult<Expr> {
        let t = self.next()?;
        match t.text.as_str() {
            "(" => {
                if self.eat(")") {
                    return Ok(Expr::Unit);
                }
                let first = self.expr(effect)?;
                if self.eat(",") {
                    let mut row = vec![first];
                    loop {
                        row.push(self.expr(effect)?);
                        if !self.eat(",") {
                            break;
                        }
                    }
                    self.expect(")")?;
                    Ok(Expr::Tuple(row))
                } else {
                    self.expect(")")?;
                    // Grouping is transparent in the AST; replace the inner
                    // node's source envelope rather than add an extra node.
                    self.origins.pop();
                    Ok(first)
                }
            }
            "[" | "#[" => {
                let mut xs = Vec::new();
                while !self.eat("]") {
                    xs.push(self.expr(effect)?);
                }
                if t.text == "#[" {
                    let rows = Expr::Array(xs);
                    // The table wrapper has a source token; the synthetic
                    // array operand has no independent opener or range.
                    self.origins.push(Origin::new(&rows, None));
                    Ok(Expr::Call(t.text, vec![rows]))
                } else {
                    Ok(Expr::Array(xs))
                }
            }
            "{" => self.block(false),
            ".{" => self.record_fields(effect).map(Expr::Record),
            "{." => Err(Error::new(
                t.start,
                "write `.{` to start an anonymous record",
            )),
            "|{" => self.pipeline(effect),
            "$" if matches!(self.lookup("$"), Some(Binding::Value)) => Ok(Expr::Name(t.text)),
            "$" => Err(Error::new(
                t.start,
                "`$` is only bound in pipeline stages after the source",
            )),
            "^{" => self.fallible(effect),
            "^:" => Ok(Expr::Raise(Box::new(self.expr(effect)?))),
            "^?" => Ok(Expr::Propagate(Box::new(self.expr(effect)?))),
            "^=" => self.recover(effect),
            "!{" => self.block(true),
            "!" if self.eat("{") => self.block(true),
            "!" if effect => self.call(t.text, 1, effect),
            "!" => Err(Error::new(
                t.start,
                "action execution is only legal inside an action block",
            )),
            "\\" => {
                let (params, types) = if self.peek() == "(" {
                    self.parameters()?
                } else {
                    (vec![self.ident()?], vec![None])
                };
                self.expect("->")?;
                self.scopes
                    .push(params.iter().map(|p| (p.clone(), Binding::Value)).collect());
                let out = self.expr(false);
                self.scopes.pop();
                let body = Box::new(out?);
                if types.iter().any(Option::is_some) {
                    Ok(Expr::TypedLambda(params, types, body))
                } else {
                    Ok(Expr::Lambda(params, body))
                }
            }
            "?" | "?{" => {
                let mut pairs = Vec::new();
                if t.text == "?{" || self.eat("{") {
                    while !self.eat("_") {
                        let c = self.expr(effect)?;
                        self.expect(":")?;
                        let v = self.expr(effect)?;
                        self.expect(";")?;
                        pairs.push((c, v));
                    }
                    self.expect(":")?;
                    let default = self.expr(effect)?;
                    self.expect(";")?;
                    self.expect("}")?;
                    Ok(Expr::Cases(pairs, Box::new(default)))
                } else {
                    let c = self.expr(effect)?;
                    let v = self.expr(effect)?;
                    let default = self.expr(effect)?;
                    Ok(Expr::Cases(vec![(c, v)], Box::new(default)))
                }
            }
            "<@" | "@>" | "!>" => Err(Error::new(
                t.start,
                "retired I/O operator; use a qualified module action",
            )),
            "<-" => Err(Error::new(
                t.start,
                "mutation with `<-` is reserved, not implemented",
            )),
            "?=" => Err(Error::new(t.start, "pattern matching is not implemented")),
            "#{" => Err(Error::new(
                t.start,
                "dictionary literals are not implemented",
            )),
            "&" => {
                let n = self.next()?;
                if matches!(self.lookup(&n.text),Some(Binding::Callable(k)|Binding::Constructor(k)) if k>0)
                {
                    Ok(Expr::Ref(self.canonical(&n.text)))
                } else {
                    Err(Error::new(
                        n.start,
                        format!("not a referenceable callable: `{}`", n.text),
                    ))
                }
            }
            ":" => {
                let s = self.next()?;
                if !matches!(s.text.as_str(), "n" | "z" | "q" | "d" | "r") {
                    return Err(Error::new(
                        s.start,
                        "expected numeric conversion target n, z, q, d, or r",
                    ));
                }
                self.call(format!(":{}", s.text), 1, effect)
            }
            "0b" => Ok(Expr::Bool(false)),
            "1b" => Ok(Expr::Bool(true)),
            s if s.starts_with('"') => Ok(Expr::Str(decode(&t)?)),
            s if s.starts_with('\'') && s.ends_with('\'') && s.len() >= 3 => {
                let d = decode(&t)?;
                let mut c = d.chars();
                let ch = c
                    .next()
                    .ok_or_else(|| Error::new(t.start, "empty character literal"))?;
                if c.next().is_some() {
                    return Err(Error::new(
                        t.start,
                        "character literal contains multiple scalars",
                    ));
                }
                Ok(Expr::Char(ch))
            }
            s if s
                .trim_start_matches('-')
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_digit) =>
            {
                if let Ok(n) = s.parse() {
                    Ok(Expr::Int(n))
                } else if s.bytes().all(|b| b.is_ascii_digit() || b == b'-') {
                    Err(Error::new(
                        t.start,
                        "integer literal exceeds this implementation's i128 range",
                    ))
                } else {
                    Ok(Expr::Num(t.text))
                }
            }
            _ => match self.lookup(&t.text) {
                Some(Binding::Value | Binding::Constructor(0)) => Ok(Expr::Name(t.text)),
                Some(Binding::Constructor(n)) => self.call(t.text, n, effect),
                Some(Binding::Callable(n)) => self.call(self.canonical(&t.text), n, effect),
                Some(Binding::Record) => self.record(t.text, effect),
                None => Err(Error::new(
                    t.start,
                    format!("unknown name or invalid expression token `{}`", t.text),
                )),
            },
        }
    }
    fn record(&mut self, name: String, effect: bool) -> PResult<Expr> {
        self.expect("{")?;
        self.record_fields(effect)
            .map(|f| Expr::NamedRecord(name, f))
    }
    fn record_fields(&mut self, effect: bool) -> PResult<Vec<(String, Expr)>> {
        let mut fields = Vec::new();
        let mut seen = HashSet::new();
        while !self.eat("}") {
            let n = self.ident()?;
            if !seen.insert(n.clone()) {
                return self.err(format!("duplicate record field `{n}`"));
            }
            self.expect(":=")?;
            fields.push((n, self.expr(effect)?));
            self.expect(";")?;
        }
        Ok(fields)
    }
    fn pipeline(&mut self, effect: bool) -> PResult<Expr> {
        let mut stages = vec![self.expr(effect)?];
        self.expect(";")?;
        self.scopes
            .push(HashMap::from([("$".to_owned(), Binding::Value)]));
        let tail = (|| -> PResult<()> {
            loop {
                stages.push(self.expr(effect)?);
                if self.eat("}") {
                    return Ok(());
                }
                self.expect(";")?;
            }
        })();
        self.scopes.pop();
        tail?;
        Ok(Expr::Pipeline(stages))
    }
    fn call(&mut self, n: String, arity: usize, effect: bool) -> PResult<Expr> {
        let mut args = Vec::with_capacity(arity);
        for i in 0..arity {
            if matches!(self.peek(), "" | ";" | "}" | ")" | "]" | ",") {
                return self.err(format!("missing operand {} of {arity} for `{n}`", i + 1));
            }
            args.push(self.expr(effect)?);
        }
        Ok(Expr::Call(n, args))
    }
    fn block(&mut self, effect: bool) -> PResult<Expr> {
        let h = self.prepass(self.pos, false)?;
        self.scopes.push(h);
        let r = self.block_contents(effect);
        self.scopes.pop();
        let (stmts, last) = r?;
        Ok(if effect {
            Expr::Action(stmts, last)
        } else {
            Expr::Block(stmts, last)
        })
    }
    fn fallible(&mut self, effect: bool) -> PResult<Expr> {
        let h = self.prepass(self.pos, false)?;
        self.scopes.push(h);
        let r = self.block_sequence(effect, true, false);
        self.scopes.pop();
        let (ss, last) = r?;
        Ok(Expr::Fallible(ss, last))
    }
    fn recover(&mut self, effect: bool) -> PResult<Expr> {
        let subject = Box::new(self.expr(effect)?);
        self.expect("{")?;
        let mut cases = Vec::new();
        while !self.eat("}") {
            let start = self.offset();
            let (pattern, names) = if self.eat("_") {
                (RecoveryPattern::Wildcard, Vec::new())
            } else {
                let name = self.ident()?;
                match self.lookup(&name) {
                    Some(Binding::Constructor(n)) => {
                        let mut slots = Vec::new();
                        let mut names = Vec::new();
                        for _ in 0..n {
                            if self.eat("_") {
                                slots.push("_".to_owned());
                            } else {
                                let binder = self.ident()?;
                                if names.contains(&binder) {
                                    return self
                                        .err(format!("duplicate recovery binder `{binder}`"));
                                }
                                names.push(binder.clone());
                                slots.push(binder);
                            }
                        }
                        (RecoveryPattern::Constructor(name, slots), names)
                    }
                    _ => (RecoveryPattern::Bind(name.clone()), vec![name]),
                }
            };
            self.expect(":")?;
            self.scopes
                .push(names.into_iter().map(|n| (n, Binding::Value)).collect());
            let body = self.expr(effect);
            self.scopes.pop();
            let body = body?;
            self.expect(";")?;
            cases.push(RecoveryCase {
                pattern,
                body,
                start,
            });
        }
        if cases.is_empty() {
            return self.err("recovery requires at least one case");
        }
        Ok(Expr::Recover(subject, cases))
    }
    fn block_contents(&mut self, effect: bool) -> PResult<(Vec<Stmt>, Option<Box<Expr>>)> {
        self.block_sequence(effect, effect, effect)
    }
    fn block_sequence(
        &mut self,
        effect: bool,
        sequence: bool,
        implicit_unit: bool,
    ) -> PResult<(Vec<Stmt>, Option<Box<Expr>>)> {
        let mut stmts = Vec::new();
        loop {
            if self.peek() == "}" {
                if !implicit_unit {
                    return self.err("block requires a final expression (write () for unit)");
                }
                self.pos += 1;
                return Ok((stmts, None));
            }
            if self.def_at(self.pos) {
                stmts.push(self.definition(effect)?);
                continue;
            }
            let start = self.offset();
            let body = self.expr(effect)?;
            if sequence && self.eat(";") {
                stmts.push(Stmt::Effect {
                    name: None,
                    body,
                    start,
                });
                continue;
            }
            self.expect("}")?;
            return Ok((stmts, Some(Box::new(body))));
        }
    }
}

pub(crate) fn standard_owners(module: &str) -> &'static [&'static str] {
    match module {
        "io" => &["ioerr", "csverr"],
        "csv" => &["ioerr", "csverr"],
        "math" => &["numerr"],
        _ => &[],
    }
}

fn decode(t: &Token) -> PResult<String> {
    if t.text.len() < 2 {
        return Err(Error::new(t.start, "invalid literal"));
    }
    let mut ch = t.text[1..t.text.len() - 1].chars();
    let mut out = String::new();
    while let Some(c) = ch.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let err = || Error::new(t.start, "invalid literal escape");
        out.push(match ch.next().ok_or_else(err)? {
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            '0' => '\0',
            '\\' => '\\',
            '"' => '"',
            '\'' => '\'',
            'u' => {
                if ch.next() != Some('{') {
                    return Err(err());
                }
                let mut hex = String::new();
                loop {
                    let d = ch.next().ok_or_else(err)?;
                    if d == '}' {
                        break;
                    }
                    hex.push(d);
                }
                u32::from_str_radix(&hex, 16)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or_else(err)?
            }
            _ => return Err(err()),
        })
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_start_identifies_opening_bang_after_declarations() {
        let source = "x := 2; -- explanatory comment\n    !{ x };";
        assert_eq!(parse(source).unwrap().entry_start, source.find("!{"));
        assert_eq!(parse("x := 2;").unwrap().entry_start, None);
        assert_eq!(parse("! { (); };").unwrap().entry_start, Some(0));
    }

    #[test]
    fn arity_missing_surplus_and_strict_prefix() {
        assert!(parse("p(x,y:n):n := + x y; n := p 2 3;").is_ok());
        for bad in [
            "n := + 2;",
            "n := + 2 3 4;",
            "n := 2 + 3;",
            "p(x,y:n):n := + x y; n := p(2,3);",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn callable_hoisting_and_lexical_shadowing() {
        let p = parse("a(x:n):n := b x; b(x:n):n := x; k(avg:n):n := avg;                        y := { a := 4; f(x:n):n := + a x; f 3 };").unwrap();
        assert!(matches!(&p.defs[2], Stmt::Def { body: Expr::Name(n), .. } if n == "avg"));
        assert!(parse("n := { f(x:n):n := + x k; k := 2; f 1 };").is_err());
        assert!(parse("n := { k := 1; k := 2; k };").is_err());
    }

    #[test]
    fn module_arity_and_terminal_anonymous_effect() {
        let p = parse("x := &say.print; << \"io\" : say; !{ ! @ x 1; };").unwrap();
        assert_eq!(p.imports, vec!["io"]);
        assert!(matches!(&p.defs[0], Stmt::Def { body: Expr::Ref(n), .. } if n == "io.print"));
        assert!(matches!(p.entry, Some(Expr::Action(_, None))));
        assert!(parse("!{ (); }; x := 1;").is_err());
        assert!(parse("!{ (); }; !{ (); };").is_err());
        assert!(parse("x := ! 1;").is_err());
        assert!(parse("!{ ! !> 1; };").is_err());
    }

    #[test]
    fn declaration_types_and_guards() {
        assert!(parse("pt :: { x:q; y:q; }; x := pt { x:=2; y:=3; };                        n:#[s n] := #[ (\"x\",1) ];                        f(x,y:(n,n),k:n):n := ? { > k 1 : .1 x; _ : .2 y; };").is_ok());
        assert!(parse(
            "pt :: { x:q; }; identity(t:#pt):#pt := t; \
                       !{ t:#pt := #[ pt { x:=2; } ]; # (:# identity t) };"
        )
        .is_ok());
        assert!(parse("f(x:q,y):q := + x y;").is_err());
        assert!(parse(">> (f); f(x:q) := * x x;").is_err());
        assert!(parse("x := ? { 1b : 1; };").is_err());
        assert!(parse("x := {. x:=3; };").is_err());
    }

    #[test]
    fn aesthetic_core_forms_parse_with_lexical_pipeline_scope() {
        let p = parse(
            r#"<< csv;
                          << "lib/df.flx" : frame;
                          a := ?{ 1b : .{ x := 7; }; _ : .{ x := 8; }; };
                          p := |{ 2; |{ $; + $ 1 }; * $ 3 };
                          q := \x -> + x 1;
                          r := \(x,y:q) -> + x y;
                          s := frame.extend &rev [1 2];
                          !{ ! csv.write "out.csv" []; };
                       "#,
        )
        .unwrap();
        assert_eq!(p.imports, vec!["csv", "df"]);
        assert!(matches!(&p.defs[1], Stmt::Def { body: Expr::Pipeline(es), .. } if es.len()==3));
        assert!(matches!(
            &p.defs[2],
            Stmt::Def {
                body: Expr::Lambda(..),
                ..
            }
        ));
        assert!(matches!(
            &p.defs[3],
            Stmt::Def {
                body: Expr::TypedLambda(..),
                ..
            }
        ));
        assert!(matches!(&p.defs[4], Stmt::Def { body: Expr::Call(n,..), .. } if n=="df.extend"));
        let Stmt::Def {
            body: Expr::Cases(rows, _),
            ..
        } = &p.defs[0]
        else {
            panic!()
        };
        assert!(matches!(&rows[0].1, Expr::Record(_)));
    }

    #[test]
    fn new_forms_reject_missing_operands_and_illegal_stage_binding() {
        for s in [
            "v := $;",
            "v := |{ $; 1 };",
            "v := |{ 1 };",
            "v := |{ 1; };",
            "v := |{ 1; $; };",
            "v := |{ 1; + $ };",
            "v := |{ 1; + $ 2 3 };",
            "v := take 2;",
            "v := drop 2 [1] [2];",
            "v := slice 1 [2];",
            "v := .{ a:=2; a:=3; };",
            "v := {. a:=2; };",
            r"v := \(x:q,y) -> x;",
            "<< io; << \"lib/io.flx\";",
            "<< \"lib/bad-name.flx\";",
        ] {
            assert!(parse(s).is_err(), "unexpected acceptance: {s}");
        }
        assert!(parse("take(n:n):n := n; y := take 2;").is_ok());
    }
    #[test]
    fn fallible_block_requires_tail_but_allows_statements() {
        assert!(parse("x := ^{ 1; () };").is_ok());
        assert!(parse("x := ^{ 1; };").is_err());
        assert!(parse("x := ^{};").is_err());
        assert!(parse("!{ (); };").is_ok());
    }
}
