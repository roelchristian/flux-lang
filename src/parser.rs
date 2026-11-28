use crate::lexer::{decode_literal, is_ident, lex, Delimiter, Token};
pub use crate::Error;
use crate::{builtins, errors041};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    hash::{DefaultHasher, Hash, Hasher},
    ops::Range,
};

mod annotation;
mod tacit;
use annotation::{adjacent_type_variable, annotation_tokens, split_callable_annotation};
pub use annotation::{parse_type_annotation, TypeAnnotation, TypeAnnotationKind, TypePrimitive};
pub(crate) use tacit::is_capture;

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
    members: Vec<ModuleMember>,
    formatting_applications: Vec<FormattingApplication>,
}
/// Original source ranges for one ordinary prefix or tacit application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormattingApplication {
    /// From the operator through its final operand, excluding outer grouping.
    pub span: Range<usize>,
    pub operator: Range<usize>,
    /// Operand ranges include any explicit grouping and exclude surrounding trivia.
    pub operands: Vec<Range<usize>>,
    pub tacit: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Def {
        name: String,
        params: Vec<String>,
        param_types: Vec<Option<TypeAnnotation>>,
        result_type: Option<TypeAnnotation>,
        body: Expr,
        start: usize,
    },
    Sum {
        name: String,
        type_params: Vec<String>,
        variants: Vec<Variant>,
        start: usize,
    },
    Schema {
        name: String,
        fields: Vec<(String, TypeAnnotation)>,
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
    pub payload: Vec<TypeAnnotation>,
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
pub struct MatchCase {
    pub pattern: MatchPattern,
    pub body: Expr,
    pub start: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MatchPattern {
    pub span: Range<usize>,
    pub kind: MatchPatternKind,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MatchPatternKind {
    Wildcard,
    Bind(String),
    Constructor(String, Vec<MatchPattern>),
    Tuple(Vec<MatchPattern>),
    Record(Vec<(String, MatchPattern)>),
    Optional(Option<Box<MatchPattern>>),
    Bool(bool),
    Unit,
}

/// Postorder syntax identity, scoped to one complete Program. Independent
/// parses and clones of the same syntax have identical IDs. Structural
/// edits or changes to the order of linked modules may renumber them.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct SyntaxId(pub usize);

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
    TypedLambda(Vec<String>, Vec<Option<TypeAnnotation>>, Box<Expr>),
    Pipeline(Vec<Expr>),
    Record(Vec<(String, Expr)>),
    Block(Vec<Stmt>, Option<Box<Expr>>),
    Action(Vec<Stmt>, Option<Box<Expr>>),
    Fallible(Vec<Stmt>, Option<Box<Expr>>),
    Raise(Box<Expr>),
    Propagate(Box<Expr>),
    Recover(Box<Expr>, Vec<RecoveryCase>),
    Match(Box<Expr>, Vec<MatchCase>),
    Num(String),
    Char(char),
    NamedRecord(String, Vec<(String, Expr)>),
}

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

        Expr::Match(_, arms) => {
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
impl Stmt {
    pub(crate) fn start(&self) -> usize {
        match self {
            Self::Def { start, .. }
            | Self::Effect { start, .. }
            | Self::Schema { start, .. }
            | Self::Sum { start, .. } => *start,
        }
    }

    pub(crate) fn body_and_arity(&self) -> Option<(&Expr, usize)> {
        match self {
            Self::Def { params, body, .. } => Some((body, params.len())),
            Self::Effect { body, .. } => Some((body, 0)),
            Self::Schema { .. } | Self::Sum { .. } => None,
        }
    }
}

impl Expr {
    // Structural children in parser origin order. Not an evaluation walk:
    // all branches and latent bodies are included, even when never executed.
    pub(crate) fn try_for_each_child<'a, E>(
        &'a self,
        mut visit: impl FnMut(&'a Expr) -> Result<(), E>,
    ) -> Result<(), E> {
        match self {
            Self::Call(_, es) | Self::Tuple(es) | Self::Array(es) | Self::Pipeline(es) => {
                es.iter().try_for_each(visit)
            }
            Self::Cases(pairs, fallback) => {
                for (cond, branch) in pairs {
                    visit(cond)?;
                    visit(branch)?;
                }
                visit(fallback)
            }
            Self::Lambda(_, body)
            | Self::TypedLambda(_, _, body)
            | Self::Raise(body)
            | Self::Propagate(body) => visit(body),
            Self::Record(fields) | Self::NamedRecord(_, fields) => {
                fields.iter().try_for_each(|(_, e)| visit(e))
            }
            Self::Recover(body, arms) => {
                visit(body)?;
                arms.iter().try_for_each(|a| visit(&a.body))
            }
            Self::Match(body, arms) => {
                visit(body)?;
                arms.iter().try_for_each(|a| visit(&a.body))
            }
            Self::Block(stmts, tail) | Self::Action(stmts, tail) | Self::Fallible(stmts, tail) => {
                for (body, _) in stmts.iter().filter_map(Stmt::body_and_arity) {
                    visit(body)?;
                }
                tail.iter().try_for_each(|e| visit(e))
            }
            Self::Int(_)
            | Self::Str(_)
            | Self::Bool(_)
            | Self::Char(_)
            | Self::Unit
            | Self::Num(_)
            | Self::Name(_)
            | Self::Ref(_) => Ok(()),
        }
    }

    pub(crate) fn for_each_child<'a>(&'a self, mut visit: impl FnMut(&'a Expr)) {
        self.try_for_each_child(|e| {
            visit(e);
            Ok::<(), std::convert::Infallible>(())
        })
        .unwrap();
    }
}

impl Program {
    fn expressions_postorder(&self) -> Vec<&Expr> {
        fn collect<'p>(e: &'p Expr, nodes: &mut Vec<&'p Expr>) {
            e.for_each_child(|child| collect(child, nodes));
            nodes.push(e);
        }

        let roots = self
            .defs
            .iter()
            .filter_map(|stmt| stmt.body_and_arity().map(|(body, _)| body))
            .chain(self.entry.iter());
        let mut nodes = Vec::with_capacity(self.origins.len());
        for e in roots {
            collect(e, &mut nodes);
        }
        nodes
    }

    /// The full deterministic syntax index, including generated expressions
    /// (such as tacit expansion nodes), independent of source span availability.
    pub fn identified_expressions(&self) -> impl Iterator<Item = (SyntaxId, &Expr)> {
        self.expressions_postorder()
            .into_iter()
            .enumerate()
            .map(|(i, e)| (SyntaxId(i), e))
    }

    #[cfg(test)]
    pub(crate) fn expression_origins(&self) -> Vec<(&Expr, Range<usize>)> {
        self.identified_origins()
            .into_iter()
            .map(|(_, expr, span)| (expr, span))
            .collect()
    }

    pub(crate) fn identified_origins(&self) -> Vec<(SyntaxId, &Expr, Range<usize>)> {
        let nodes = self.expressions_postorder();
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
            .enumerate()
            .filter_map(|(i, (node, origin))| {
                origin.span.clone().map(|span| (SyntaxId(i), node, span))
            })
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemberKind {
    Callable(usize),
    Value,
    Record,
    Sum,
    Constructor(usize),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleMember {
    pub name: String,
    pub linked: String,
    pub kind: MemberKind,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedModule {
    pub path: String,
    pub members: Vec<ModuleMember>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportDecl {
    pub path: String,
    pub alias: String,
    pub start: usize,
}

impl Program {
    /// Available only for parses that explicitly requested layout metadata.
    pub fn formatting_applications(&self) -> &[FormattingApplication] {
        &self.formatting_applications
    }

    pub fn module_members(&self) -> Vec<ModuleMember> {
        self.members.clone()
    }
    /// Prepend declaration-only, independently parsed dependencies in dependency
    /// order. Their globally rebased expression origins are retained.
    pub fn merge_libraries(&mut self, libraries: impl IntoIterator<Item = Program>) {
        let mut defs = Vec::new();
        let mut origins = Vec::new();
        let mut imports = Vec::new();
        let mut seen = HashSet::new();
        for mut lib in libraries {
            assert!(
                lib.entry.is_none(),
                "cannot link a script entry as a library"
            );
            defs.append(&mut lib.defs);
            origins.append(&mut lib.origins);
            self.constraints.append(&mut lib.constraints);
            for x in lib.imports {
                if seen.insert(x.clone()) {
                    imports.push(x);
                }
            }
        }
        defs.append(&mut self.defs);
        self.defs = defs;
        origins.append(&mut self.origins);
        self.origins = origins;
        for x in self.imports.drain(..) {
            if seen.insert(x.clone()) {
                imports.push(x);
            }
        }
        self.imports = imports;
    }
}

fn parser_for(
    source: &str,
    base: usize,
    namespace: Option<&str>,
    imports: &[ImportedModule],
) -> PResult<Parser> {
    let eof = source
        .len()
        .checked_add(base)
        .ok_or_else(|| Error::new(base, "source location exceeds addressable size"))?;
    let mut toks = lex(source).map_err(|e| Error::new(base.saturating_add(e.start), e.message))?;
    for t in &mut toks {
        t.start += base;
        t.end += base;
    }
    Ok(Parser {
        toks,
        pos: 0,
        eof,
        scopes: Vec::new(),
        type_scopes: Vec::new(),
        standard: errors041::variants("ioerr")
            .iter()
            .map(|v| (v.name.to_owned(), Binding::Constructor(v.payload.len())))
            .collect(),
        native_type_names: HashSet::from(["ioerr".to_owned(), "array".to_owned()]),
        aliases: HashMap::new(),
        source_interfaces: imports.to_vec(),
        links: HashMap::new(),
        namespace: namespace.map(str::to_owned),
        depth: 0,
        constraints: BTreeMap::new(),
        origins: Vec::new(),
        formatting_applications: None,
    })
}

/// Parse an independently scoped module, with globally rebased locations.
pub fn parse_module(
    source: &str,
    base: usize,
    namespace: Option<&str>,
    imports: &[ImportedModule],
) -> PResult<Program> {
    parser_for(source, base, namespace, imports)?.program()
}

/// Like `parse`, collecting original call ranges for source formatting.
pub fn parse_with_layout(source: &str) -> PResult<Program> {
    parse_module_with_layout(source, 0, None, &[])
}

/// Like `parse_module`, collecting layout for this source file only.
pub(crate) fn parse_module_with_layout(
    source: &str,
    base: usize,
    namespace: Option<&str>,
    imports: &[ImportedModule],
) -> PResult<Program> {
    let mut p = parser_for(source, base, namespace, imports)?;
    p.formatting_applications = Some(Vec::new());
    p.program()
}

/// Discover only top-level imports before binding interfaces. No imported
/// members or callable arities are needed for this lexical pass.
pub fn discover_imports(source: &str, base: usize) -> PResult<Vec<ImportDecl>> {
    let mut p = parser_for(source, base, None, &[])?;
    let mut imports = Vec::new();
    let mut i = 0;
    while !p.at(i).is_empty() {
        if p.at(i) == "<<" {
            let start = p.toks[i].start;
            p.pos = i + 1;
            let (path, alias, _) = p.import_decl()?;
            i = p.pos;
            imports.push(ImportDecl { path, alias, start });
        } else if let Some(end) = p.find(i, &[";"], &[]) {
            i = end + 1;
        } else {
            break;
        }
    }
    Ok(imports)
}

pub fn parse(source: &str) -> PResult<Program> {
    parse_module(source, 0, None, &[])
}
type Prelude = (HashMap<String, Binding>, HashSet<String>);
struct Parser {
    toks: Vec<Token>,
    pos: usize,
    eof: usize,
    scopes: Vec<HashMap<String, Binding>>,
    type_scopes: Vec<HashMap<String, String>>,
    native_type_names: HashSet<String>,
    source_interfaces: Vec<ImportedModule>,
    links: HashMap<String, String>,
    namespace: Option<String>,
    standard: HashMap<String, Binding>,
    aliases: HashMap<String, String>,
    depth: usize,
    constraints: BTreeMap<usize, Vec<(String, String)>>,
    origins: Vec<Origin>,
    formatting_applications: Option<Vec<FormattingApplication>>,
}
fn is_qualified_ident(s: &str) -> bool {
    s.split_once('.')
        .is_some_and(|(a, b)| is_ident(a) && is_ident(b))
}
/// A REPL submission needs more input if it ends inside matched delimiter
/// prefixes. Bad tokens and mismatched closing signs go to normal diagnostics.
pub fn needs_continuation(source: &str) -> bool {
    let Ok(tokens) = lex(source) else {
        return false;
    };
    let mut stack = Vec::new();
    for token in tokens {
        if let Some(expected) = Delimiter::opening(&token.text) {
            stack.push(expected);
        } else if let Some(end) = Delimiter::closing(&token.text) {
            if stack.pop() != Some(end) {
                return false;
            }
        }
    }
    !stack.is_empty()
}

// Lowered binders cannot collide with a bare identifier, module link or
// pipeline binding. Every synthetic lambda owns its own resolver scope.
const DIRECT_ARITY_LIMIT: usize = 64;

fn originated(expr: Expr, span: Option<Range<usize>>, origins: &mut Vec<Origin>) -> Expr {
    origins.push(Origin::new(&expr, span));
    expr
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
    fn own_name(&self, n: &str) -> String {
        self.namespace
            .as_ref()
            .map_or_else(|| n.to_owned(), |m| format!("\0{m}:{n}"))
    }
    fn decl_link(&self, n: &str) -> String {
        if self.scopes.len() == 1 {
            self.own_name(n)
        } else {
            n.to_owned()
        }
    }
    fn canonical(&self, n: &str) -> String {
        if self.scopes.iter().skip(1).rev().any(|s| s.contains_key(n)) {
            return n.to_owned();
        }
        if let Some(link) = self.links.get(n) {
            return link.clone();
        }
        if let Some((alias, member)) = n.split_once('.') {
            if let Some(path) = self.aliases.get(alias) {
                let path = if path == "table" { "df" } else { path };
                return format!("{path}.{member}");
            }
        }
        n.to_owned()
    }
    fn annotation_links(&self, mut node: TypeAnnotation) -> PResult<TypeAnnotation> {
        fn rewrite(p: &Parser, t: &mut TypeAnnotation) -> PResult<()> {
            use TypeAnnotationKind::*;
            match &mut t.kind {
                Named(n) | NamedApply(n, _) => {
                    if let Some(name) = p.type_scopes.iter().rev().find_map(|s| s.get(n)) {
                        *n = name.clone();
                    } else if is_qualified_ident(n) {
                        return Err(Error::new(
                            t.span.start,
                            format!("unknown or unexported module type `{n}`"),
                        ));
                    } else if p.namespace.is_some() && !p.native_type_names.contains(n) {
                        return Err(Error::new(t.span.start, format!("unknown type {n}")));
                    }
                }
                Optional(t) | Action(t) | Table(t) | Sequence(t) => rewrite(p, t)?,
                PositionalTable(ts) | Tuple(ts) => {
                    for t in ts {
                        rewrite(p, t)?;
                    }
                }
                Dictionary(k, v) | Result(k, v) => {
                    rewrite(p, k)?;
                    rewrite(p, v)?;
                }
                Record(fields) | OpenRecord(fields, _) => {
                    for (_, t) in fields {
                        rewrite(p, t)?;
                    }
                }
                Function(args, ret) => {
                    for t in args {
                        rewrite(p, t)?;
                    }
                    rewrite(p, ret)?;
                }
                Primitive(_) | Variable(_) | Hole => {}
            }
            if let NamedApply(_, args) = &mut t.kind {
                for arg in args {
                    rewrite(p, arg)?;
                }
            }
            Ok(())
        }
        rewrite(self, &mut node)?;
        Ok(node)
    }
    fn scoped_expression(
        &mut self,
        names: impl IntoIterator<Item = String>,
        effect: bool,
    ) -> PResult<Expr> {
        self.scopes.push(
            names
                .into_iter()
                .map(|name| (name, Binding::Value))
                .collect(),
        );
        let result = self.expr(effect);
        self.scopes.pop();
        result
    }

    fn case_body(
        &mut self,
        names: impl IntoIterator<Item = String>,
        effect: bool,
    ) -> PResult<Expr> {
        self.expect(":")?;
        let body = self.scoped_expression(names, effect)?;
        self.expect(";")?;
        Ok(body)
    }

    fn scope(&mut self) -> &mut HashMap<String, Binding> {
        self.scopes.last_mut().expect("parser scope")
    }
    fn end_of_group(&self, from: usize) -> Option<usize> {
        let mut ends = vec![Delimiter::opening(self.at(from))?];
        for i in from + 1..self.toks.len() {
            let s = self.at(i);
            if let Some(c) = Delimiter::opening(s) {
                ends.push(c);
            } else if let Some(end) = Delimiter::closing(s) {
                if ends.pop() != Some(end) {
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
            if stops.contains(&s) || Delimiter::closing(s).is_some() {
                return None;
            }
            if Delimiter::opening(s).is_some() {
                i = self.end_of_group(i)? + 1;
            } else {
                i += 1;
            }
        }
        None
    }
    fn type_head(&self, i: usize) -> Option<usize> {
        if self.at(i + 1) == "::" {
            return Some(i + 1);
        }
        if self.at(i + 1) != "[" || self.toks[i].end != self.toks[i + 1].start {
            return None;
        }
        let end = self.end_of_group(i + 1)?;
        (self.at(end + 1) == "::").then_some(end + 1)
    }
    fn arity_header_at(&self, i: usize) -> bool {
        self.at(i + 1) == "/" && matches!(self.at(i + 3), ":=" | ":" | ":#" | "|")
    }
    fn declared_arity(&self, i: usize) -> PResult<(usize, Range<usize>)> {
        let [name, slash, count] = &self.toks[i..i + 3] else {
            unreachable!("arity candidates have at least three tokens");
        };
        if name.end != slash.start || slash.end != count.start {
            return Err(Error::new(
                slash.start,
                "write callable arity adjacent as `name/arity`",
            ));
        }
        let arity = count.text.bytes().all(|byte| byte.is_ascii_digit())
            .then(|| count.text.parse::<usize>().ok())
            .flatten()
            .filter(|n| (1..=DIRECT_ARITY_LIMIT).contains(n))
            .ok_or_else(|| Error::new(count.start, format!(
                "callable arity must be an unsuffixed positive integer from 1 to {DIRECT_ARITY_LIMIT}"
            )))?;
        Ok((arity, count.start..count.end))
    }
    fn def_at(&self, i: usize) -> bool {
        if !is_ident(self.at(i)) {
            return false;
        }
        match self.at(i + 1) {
            ":=" | "::" => true,
            "[" => self.type_head(i).is_some(),
            "/" => self.arity_header_at(i),
            ":" | ":#" => self.find(i + 2, &[":="], &[";"]).is_some(),
            "(" => self.end_of_group(i + 1).is_some_and(|j| {
                self.at(j + 1) == ":="
                    || (matches!(self.at(j + 1), ":" | ":#")
                        && self.find(j + 2, &[":="], &[";"]).is_some())
            }),
            _ => false,
        }
    }
    fn prepass(&self, mut i: usize, root: bool) -> PResult<Prelude> {
        let mut names = HashSet::new();
        let mut type_names = HashSet::new();
        let mut hoisted = HashMap::new();
        loop {
            let n = self.at(i).to_owned();
            if matches!(n.as_str(), "" | "}") {
                break;
            }
            if self.def_at(i) {
                let type_head = self.type_head(i);
                let is_type = type_head.is_some();
                if is_type && n == "array" {
                    return Err(Error::new(
                        self.toks[i].start,
                        "`array` is a built-in type name and cannot be redeclared",
                    ));
                }
                let is_sum =
                    type_head.is_some_and(|h| self.at(h + 1) == "{" && self.at(h + 2) == "|");
                let head = type_head.unwrap_or(i + 1);
                if is_type && !type_names.insert(n.clone()) {
                    return Err(Error::new(
                        self.toks[i].start,
                        format!("duplicate type `{n}`"),
                    ));
                }
                let b = match self.at(head) {
                    "::" if is_sum => {
                        let mut j = head + 2;
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
                    "/" => Some(Binding::Callable(self.declared_arity(i)?.0)),
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
                            if Delimiter::opening(self.at(j)).is_some() {
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
        Ok((hoisted, type_names))
    }
    fn program(mut self) -> PResult<Program> {
        let (hoisted, type_names) = self.prepass(0, true)?;
        self.links = hoisted
            .keys()
            .map(|n| (n.clone(), self.own_name(n)))
            .collect();
        self.scopes.push(hoisted);
        self.type_scopes.push(
            type_names
                .iter()
                .map(|n| (n.clone(), self.own_name(n)))
                .collect(),
        );
        let mut used_interfaces = vec![false; self.source_interfaces.len()];
        let mut used_aliases = HashSet::new();
        let mut p = Program {
            defs: Vec::new(),
            imports: Vec::new(),
            entry: None,
            entry_start: None,
            constraints: BTreeMap::new(),
            exports: Vec::new(),
            origins: Vec::new(),
            members: Vec::new(),
            formatting_applications: Vec::new(),
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
                let (raw, alias, start) = self.import_decl()?;
                if !used_aliases.insert(alias.clone()) {
                    return Err(Error::new(
                        start,
                        format!("duplicate module alias `{alias}`"),
                    ));
                }
                if let Some((at, module)) = self
                    .source_interfaces
                    .iter()
                    .enumerate()
                    .find(|(i, m)| !used_interfaces[*i] && m.path == raw)
                {
                    used_interfaces[at] = true;
                    let members = module.members.clone();
                    for member in members {
                        let surface = format!("{alias}.{}", member.name);
                        let value = match member.kind {
                            MemberKind::Callable(n) => Some(Binding::Callable(n)),
                            MemberKind::Value => Some(Binding::Value),
                            MemberKind::Record => Some(Binding::Record),
                            MemberKind::Constructor(n) => Some(Binding::Constructor(n)),
                            MemberKind::Sum => None,
                        };
                        if let Some(b) = value {
                            self.scope().insert(surface.clone(), b);
                            self.links.insert(surface.clone(), member.linked.clone());
                        }
                        if matches!(member.kind, MemberKind::Record | MemberKind::Sum) {
                            self.type_scopes[0].insert(surface, member.linked);
                        }
                    }
                } else {
                    let native = std::path::Path::new(&raw)
                        .file_stem()
                        .and_then(|x| x.to_str())
                        .unwrap_or("");
                    if self.namespace.is_some() && (raw.starts_with("./") || raw.starts_with("../"))
                    {
                        return Err(Error::new(
                            start,
                            format!("source module `{raw}` requires a loaded interface"),
                        ));
                    }
                    let items = builtins::module_items(native)
                        .ok_or_else(|| Error::new(start, format!("unknown module `{raw}`")))?;
                    self.aliases.insert(alias.clone(), native.to_owned());
                    for (name, arity) in items {
                        self.scope()
                            .insert(format!("{alias}.{name}"), Binding::Callable(arity));
                    }
                    for &owner in standard_owners(native) {
                        self.native_type_names.insert(owner.to_owned());
                        for v in errors041::variants(owner) {
                            self.standard
                                .insert(v.name.to_owned(), Binding::Constructor(v.payload.len()));
                        }
                    }
                    p.imports.push(native.to_owned());
                }
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
            let linked = self.own_name(n);
            let mut found = false;
            for d in &p.defs {
                let kind = match d {
                    Stmt::Def {
                        name,
                        params,
                        result_type,
                        param_types,
                        ..
                    } if name == &linked => {
                        if result_type.is_none() || !param_types.iter().all(Option::is_some) {
                            return self.err(format!("export `{n}` requires a complete signature"));
                        }
                        if params.is_empty() {
                            MemberKind::Value
                        } else {
                            MemberKind::Callable(params.len())
                        }
                    }
                    Stmt::Schema { name, .. } if name == &linked => MemberKind::Record,
                    Stmt::Sum { name, variants, .. } if name == &linked => {
                        for v in variants {
                            let surface = if self.namespace.is_some() {
                                v.name.rsplit_once(':').unwrap().1
                            } else {
                                &v.name
                            };
                            p.members.push(ModuleMember {
                                name: surface.to_owned(),
                                linked: v.name.clone(),
                                kind: MemberKind::Constructor(v.payload.len()),
                            });
                        }
                        MemberKind::Sum
                    }
                    _ => continue,
                };
                found = true;
                p.members.push(ModuleMember {
                    name: n.clone(),
                    linked: linked.clone(),
                    kind,
                });
            }
            if !found {
                return self.err(format!("unknown export `{n}`"));
            }
        }
        p.constraints = self.constraints;
        p.exports = exports;
        p.origins = self.origins;
        p.formatting_applications = self.formatting_applications.unwrap_or_default();
        Ok(p)
    }
    fn import_decl(&mut self) -> PResult<(String, String, usize)> {
        let t = self.next()?;
        let raw = if t.text.starts_with('"') {
            decode_literal(&t)?
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
        Ok((raw, alias, t.start))
    }
    fn type_tokens_until(&mut self, ends: &[&str]) -> PResult<Vec<Token>> {
        let start = self.pos;
        if let Some(j) = self.find(start, ends, &[";", "|", ":="]) {
            if start == j {
                return self.err("expected a type");
            }
            self.pos = j;
        } else {
            return self.err("expected a type followed by its delimiter");
        }
        Ok(self.toks[start..self.pos].to_vec())
    }
    fn annotation(&mut self, ends: &[&str]) -> PResult<Option<TypeAnnotation>> {
        let mut tokens = if self.eat(":") {
            Vec::new()
        } else if self.eat(":#") {
            let end = self.toks[self.pos - 1].end;
            vec![Token {
                text: "#".into(),
                start: end - 1,
                end,
            }]
        } else {
            return Ok(None);
        };
        tokens.extend(self.type_tokens_until(ends)?);
        self.annotation_links(annotation_tokens(&tokens, self.offset())?)
            .map(Some)
    }
    fn parameters(&mut self) -> PResult<(Vec<String>, Vec<Option<TypeAnnotation>>)> {
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
            if Delimiter::opening(self.at(j)).is_some() {
                j = self
                    .end_of_group(j)
                    .map(|k| k + 1)
                    .ok_or_else(|| Error::new(start, "malformed constructor payload type"))?;
            } else if self.at(j) == "~" {
                adjacent_type_variable(&self.toks, j, self.eof)?;
                j += 2;
            } else if is_ident(self.at(j)) || is_qualified_ident(self.at(j)) {
                j += 1;
                if self.at(j) == "[" && self.toks[j - 1].end == self.toks[j].start {
                    j = self
                        .end_of_group(j)
                        .map(|k| k + 1)
                        .ok_or_else(|| Error::new(start, "unclosed applied constructor type"))?;
                }
            } else {
                return Err(Error::new(
                    self.toks.get(j).map_or(self.eof, |t| t.start),
                    "expected a constructor payload type or `;`",
                ));
            }
            let end = self.toks.get(j).map_or(self.eof, |t| t.start);
            payload.push(annotation_tokens(&self.toks[a..j], end)?);
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
        let head = self.pos;
        let surface = self.ident()?;
        let name = self.decl_link(&surface);
        let arity_header = if self.arity_header_at(head) {
            let header = self.declared_arity(head)?;
            self.pos += 2;
            Some(header)
        } else {
            None
        };
        let mut type_params = Vec::new();
        if self.eat("[") {
            loop {
                let param = adjacent_type_variable(&self.toks, self.pos, self.eof)?;
                self.pos += 2;
                if type_params.contains(&param.text) {
                    return Err(Error::new(param.start, "duplicate sum type parameter"));
                }
                type_params.push(param.text);
                if !self.eat(",") {
                    break;
                }
            }
            self.expect("]")?;
            self.expect("::")?;
        }
        if !type_params.is_empty() || self.eat("::") {
            self.expect("{")?;
            if self.peek() == "|" {
                let mut variants = Vec::new();
                while self.peek() == "|" {
                    let (mut v, next) = self.variant_at(self.pos)?;
                    v.name = self.decl_link(&v.name);
                    v.payload = v
                        .payload
                        .into_iter()
                        .map(|t| self.annotation_links(t))
                        .collect::<PResult<_>>()?;
                    self.pos = next;
                    variants.push(v);
                }
                self.expect("}")?;
                self.expect(";")?;
                return Ok(Stmt::Sum {
                    name,
                    type_params,
                    variants,
                    start,
                });
            }
            if !type_params.is_empty() {
                return self.err("type parameters are supported only on nominal sums");
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
        let (params, param_types, result_type) = if let Some((arity, _)) = &arity_header {
            let params: Vec<_> = (0..*arity)
                .map(|i| format!("\0callable-arity-{i}"))
                .collect();
            if let Some(ty) = self.annotation(&[":=", "|"])? {
                let (param_types, result_type) = split_callable_annotation(ty, *arity)?;
                (params, param_types, Some(result_type))
            } else {
                (params, vec![None; *arity], None)
            }
        } else {
            let (params, param_types) = if self.peek() == "(" {
                self.parameters()?
            } else {
                (Vec::new(), Vec::new())
            };
            let result_type = self.annotation(&[":=", "|"])?;
            (params, param_types, result_type)
        };
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
                let var = adjacent_type_variable(&self.toks, self.pos, self.eof)?;
                self.pos += 2;
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
        let body = if let Some((_, count_span)) = arity_header {
            let rhs = self.pos;
            let mut body = self.expr(false)?;
            let end = self.pos - 1;
            let mut atom = rhs;
            let mut last = end;
            while self.at(atom) == "(" && self.end_of_group(atom) == Some(last) {
                atom += 1;
                last -= 1;
            }
            let supported = match self.at(atom) {
                "&{" => true, // A tacit expression may contain an eager capture block.
                "&" | "\\" => {
                    matches!(
                        body,
                        Expr::Lambda(..) | Expr::TypedLambda(..) | Expr::Ref(..)
                    )
                }
                _ => false,
            };
            if !supported {
                return Err(Error::new(
                    self.toks[rhs].start,
                    "callable arity right-hand side must be a tacit form, function reference, or lambda",
                ));
            }
            let source = self.toks[rhs].start..self.toks[end].end;
            for param in &params {
                let input = originated(
                    Expr::Name(param.clone()),
                    Some(count_span.clone()),
                    &mut self.origins,
                );
                body = originated(
                    Expr::Call("@".into(), vec![body, input]),
                    Some(source.clone()),
                    &mut self.origins,
                );
            }
            body
        } else if params.is_empty() {
            self.expr(effect)?
        } else {
            self.scoped_expression(params.iter().cloned(), false)?
        };
        self.expect(";")?;
        if params.is_empty() {
            self.scope().insert(surface, Binding::Value);
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
            "!" if effect => self.call(t.text, 1, effect, t.start..t.end),
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
                let body = Box::new(self.scoped_expression(params.iter().cloned(), false)?);
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
            "?=" => self.match_expression(effect),
            "#{" => Err(Error::new(
                t.start,
                "dictionary literals are not implemented",
            )),
            "&{" => self.tacit_block(0),
            "@." | "@*" | "@?" | "." => Err(Error::new(
                t.start,
                format!("`{}` is only valid inside `&{{ ... }}`", t.text),
            )),
            "&" => {
                let n = self.next()?;
                if n.text == "." && t.end == n.start {
                    return Ok(tacit::identity(n.start..n.end, &mut self.origins));
                }
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
                self.call(format!(":{}", s.text), 1, effect, t.start..s.end)
            }
            "0b" => Ok(Expr::Bool(false)),
            "1b" => Ok(Expr::Bool(true)),
            s if s.starts_with('"') => Ok(Expr::Str(decode_literal(&t)?)),
            s if s.starts_with('\'') && s.ends_with('\'') && s.len() >= 3 => {
                let d = decode_literal(&t)?;
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
                // The lexer has already validated the whole numeric token.
                // Preserve non-inline values and explicitly typed spellings
                // verbatim for HIR, including a decimal's written scale.
                if let Ok(n) = s.parse::<i128>() {
                    Ok(Expr::Int(n))
                } else {
                    Ok(Expr::Num(t.text))
                }
            }
            _ => match self.lookup(&t.text) {
                Some(Binding::Value | Binding::Constructor(0)) => {
                    Ok(Expr::Name(self.canonical(&t.text)))
                }
                Some(Binding::Constructor(n) | Binding::Callable(n)) => {
                    self.call(self.canonical(&t.text), n, effect, t.start..t.end)
                }
                Some(Binding::Record) => self.record(self.canonical(&t.text), effect),
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
    fn call(
        &mut self,
        n: String,
        arity: usize,
        effect: bool,
        operator: Range<usize>,
    ) -> PResult<Expr> {
        let mut args = Vec::with_capacity(arity);
        let mut spans = self
            .formatting_applications
            .is_some()
            .then(|| Vec::with_capacity(arity));
        for i in 0..arity {
            if matches!(self.peek(), "" | ";" | "}" | ")" | "]" | ",") {
                return self.err(format!("missing operand {} of {arity} for `{n}`", i + 1));
            }
            let start = self.offset();
            args.push(self.expr(effect)?);
            if let Some(spans) = &mut spans {
                spans.push(start..self.toks[self.pos - 1].end);
            }
        }
        if let (Some(layout), Some(operands)) = (&mut self.formatting_applications, spans) {
            if let Some(last) = operands.last() {
                layout.push(FormattingApplication {
                    span: operator.start..last.end,
                    operator,
                    operands,
                    tacit: false,
                });
            }
        }
        Ok(Expr::Call(n, args))
    }
    fn scoped_block(
        &mut self,
        effect: bool,
        sequence: bool,
        implicit_unit: bool,
    ) -> PResult<(Vec<Stmt>, Option<Box<Expr>>)> {
        let (bindings, types) = self.prepass(self.pos, false)?;
        self.scopes.push(bindings);
        self.type_scopes
            .push(types.into_iter().map(|n| (n.clone(), n)).collect());
        let result = self.block_sequence(effect, sequence, implicit_unit);
        self.scopes.pop();
        self.type_scopes.pop();
        result
    }

    fn block(&mut self, effect: bool) -> PResult<Expr> {
        let (stmts, last) = self.scoped_block(effect, effect, effect)?;
        Ok(if effect {
            Expr::Action(stmts, last)
        } else {
            Expr::Block(stmts, last)
        })
    }

    fn fallible(&mut self, effect: bool) -> PResult<Expr> {
        let (stmts, last) = self.scoped_block(effect, true, false)?;
        Ok(Expr::Fallible(stmts, last))
    }

    fn match_expression(&mut self, effect: bool) -> PResult<Expr> {
        let subject = Box::new(self.expr(effect)?);
        self.expect("{")?;
        let mut cases = Vec::new();
        while !self.eat("}") {
            let start = self.offset();
            let mut binders = HashSet::new();
            let pattern = self.match_pattern(0, &mut binders)?;
            let body = self.case_body(binders, effect)?;
            cases.push(MatchCase {
                pattern,
                body,
                start,
            });
        }
        if cases.is_empty() {
            return self.err("match requires at least one case");
        }
        Ok(Expr::Match(subject, cases))
    }

    fn match_pattern(
        &mut self,
        depth: usize,
        binders: &mut HashSet<String>,
    ) -> PResult<MatchPattern> {
        use MatchPatternKind::*;
        if depth >= 128 {
            return self.err("match pattern nesting limit exceeded");
        }
        let token = self.next()?;
        let kind = match token.text.as_str() {
            "_" => Wildcard,
            "0b" => Bool(false),
            "1b" => Bool(true),
            "?0" => Optional(None),
            "?1" => Optional(Some(Box::new(self.match_pattern(depth + 1, binders)?))),
            "(" if self.eat(")") => Unit,
            "(" => {
                let first = self.match_pattern(depth + 1, binders)?;
                if self.eat(")") {
                    first.kind
                } else {
                    self.expect(",")?;
                    let mut fields = vec![first];
                    loop {
                        fields.push(self.match_pattern(depth + 1, binders)?);
                        if !self.eat(",") {
                            break;
                        }
                    }
                    self.expect(")")?;
                    Tuple(fields)
                }
            }
            ".{" => {
                let mut fields = Vec::new();
                let mut seen = HashSet::new();
                while !self.eat("}") {
                    let name = self.ident()?;
                    if !seen.insert(name.clone()) {
                        return self.err(format!("duplicate match field `{name}`"));
                    }
                    self.expect(":=")?;
                    fields.push((name, self.match_pattern(depth + 1, binders)?));
                    self.expect(";")?;
                }
                Record(fields)
            }
            name if is_ident(name) || is_qualified_ident(name) => {
                if let Some(Binding::Constructor(arity)) = self.lookup(name) {
                    let linked = self.canonical(name);
                    let args = (0..arity)
                        .map(|_| self.match_pattern(depth + 1, binders))
                        .collect::<PResult<_>>()?;
                    Constructor(linked, args)
                } else if is_qualified_ident(name) {
                    return Err(Error::new(
                        token.start,
                        format!("unknown or unexported match constructor `{name}`"),
                    ));
                } else {
                    if !binders.insert(name.to_owned()) {
                        return Err(Error::new(
                            token.start,
                            format!("duplicate match binder `{name}`"),
                        ));
                    }
                    Bind(name.to_owned())
                }
            }
            name => {
                return Err(Error::new(
                    token.start,
                    format!("expected match pattern, found `{name}`"),
                ))
            }
        };
        Ok(MatchPattern {
            span: token.start..self.toks[self.pos - 1].end,
            kind,
        })
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
                let name = if is_qualified_ident(self.peek()) {
                    self.next()?.text
                } else {
                    self.ident()?
                };
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
                        (
                            RecoveryPattern::Constructor(self.canonical(&name), slots),
                            names,
                        )
                    }
                    _ if is_qualified_ident(&name) => {
                        return Err(Error::new(
                            start,
                            format!("unknown or unexported recovery constructor `{name}`"),
                        ))
                    }
                    _ => (RecoveryPattern::Bind(name.clone()), vec![name]),
                }
            };
            let body = self.case_body(names, effect)?;
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
        "fs" | "sys" => &["ioerr"],
        "rx" => &["rxerr"],
        "dt" => &["dterr"],
        "json" => &["jsonerr"],
        _ => &[],
    }
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
    #[test]
    fn annotation_ast_preserves_caret_arrow_precedence() {
        use TypeAnnotationKind::*;
        let source = "?q^?err n -> !(q^err) -> [q^err]";
        let t = parse_type_annotation(source).unwrap();
        assert_eq!(t.span, 0..source.len());
        let Function(args, tail) = t.kind else {
            panic!()
        };
        assert_eq!(args.len(), 2);
        let Result(ok, err) = &args[0].kind else {
            panic!()
        };
        assert_eq!(&source[ok.span.clone()], "?q");
        assert_eq!(&source[err.span.clone()], "?err");
        assert!(matches!(ok.kind, Optional(_)));
        assert!(matches!(err.kind, Optional(_)));
        let Function(args, ret) = tail.kind else {
            panic!()
        };
        let Action(inner) = &args[0].kind else {
            panic!()
        };
        assert!(matches!(inner.kind, Result(..)));
        assert_eq!(&source[inner.span.clone()], "(q^err)");
        let Sequence(inner) = &ret.kind else { panic!() };
        assert!(matches!(inner.kind, Result(..)));
        let bad = parse_type_annotation("q^err^e").unwrap_err();
        assert_eq!(bad.start, 5);
        assert!(bad.message.contains("parentheses"));
    }

    #[test]
    fn declarations_store_canonical_types_with_source_offsets() {
        use TypeAnnotationKind::*;
        let source = "row :: { x:q; }; err :: { | bad (q^err); }; f(x,y:~a,g:.[z]):z := 0;";
        let source = source.replace(".[z]", ".{data:#[z q]; tag:?{s:z};}");
        let p = parse(&source).unwrap();
        let Stmt::Schema { fields, .. } = &p.defs[0] else {
            panic!()
        };
        assert_eq!(&source[fields[0].1.span.clone()], "q");
        let Stmt::Sum { variants, .. } = &p.defs[1] else {
            panic!()
        };
        assert!(matches!(variants[0].payload[0].kind, Result(..)));
        assert_eq!(&source[variants[0].payload[0].span.clone()], "(q^err)");
        let Stmt::Def {
            param_types: ts,
            result_type: Some(out),
            ..
        } = &p.defs[2]
        else {
            panic!()
        };
        assert_eq!(&source[ts[0].as_ref().unwrap().span.clone()], "~a");
        assert_eq!(ts[0], ts[1]);
        let Record(fields) = &ts[2].as_ref().unwrap().kind else {
            panic!()
        };
        assert!(matches!(fields[0].1.kind, PositionalTable(_)));
        assert_eq!(&source[fields[0].1.span.clone()], "#[z q]");
        let Optional(inner) = &fields[1].1.kind else {
            panic!()
        };
        assert!(matches!(inner.kind, Dictionary(..)));
        assert_eq!(&source[out.span.clone()], "z");
        assert!(matches!(parse_type_annotation("_").unwrap().kind, Hole));
    }

    #[test]
    fn malformed_annotation_reports_original_token() {
        let source = "-- comment
f(x:q^err^e):q := 0;";
        let problem = parse(source).unwrap_err();
        assert_eq!(problem.start, source.rfind('^').unwrap());
        assert!(problem.message.contains("parentheses"));
    }
    #[test]
    fn source_module_links_only_exports_and_rebases_origins() {
        let library_src = ">> (f,row,err); f(x:q):q := + x private; private := 2; row :: { x:q; }; err :: { | bad n; | zero; };";
        let lib = parse_module(library_src, 200, Some("library"), &[]).unwrap();
        let members = lib.module_members();
        assert_eq!(members.len(), 5);
        assert!(members.iter().all(|m| m.linked.as_bytes()[0] == 0));
        assert!(members.iter().all(|m| m.name != "private"));
        let imports = [ImportedModule {
            path: "./dep.flx".into(),
            members,
        }];
        let source = r#"<< "./dep.flx" : d; private := 9; x:d.row := d.row { x := d.f 2q; }; rr:e^d.err := ^{ ^: d.bad 4n }; e :: { val:n; }; y := ^= rr { d.bad n : e { val := n; }; d.zero : e { val := 0n; }; };"#;
        let mut root = parse_module(source, 600, Some("root"), &imports).unwrap();
        assert!(parse_module(
            r#"<< "./dep.flx" : d; x := d.private;"#,
            0,
            Some("other"),
            &imports
        )
        .is_err());
        assert!(parse_module(
            r#"<< "./dep.flx" : d; x := d.f 1 2;"#,
            0,
            Some("other"),
            &imports
        )
        .is_err());
        let Stmt::Def {
            result_type: Some(t),
            ..
        } = &root.defs[1]
        else {
            panic!()
        };
        let TypeAnnotationKind::Named(n) = &t.kind else {
            panic!()
        };
        assert_eq!(
            n,
            &imports[0]
                .members
                .iter()
                .find(|m| m.name == "row")
                .unwrap()
                .linked
        );
        assert_eq!(&source[t.span.start - 600..t.span.end - 600], "d.row");
        root.merge_libraries([lib]);
        let origins = root.expression_origins();
        assert!(origins
            .iter()
            .any(|(_, s)| s.start >= 200 && s.end <= 200 + library_src.len()));
        assert!(origins
            .iter()
            .any(|(_, s)| s.start >= 600 && s.end <= 600 + source.len()));
        let res = crate::resolve::resolve(&root).unwrap();
        assert!(res.groups.len() >= 8);
    }

    #[test]
    fn linked_nominal_scope_blocks_native_and_foreign_leakage() {
        assert!(parse_module("f(x:ioerr):ioerr := x;", 0, Some("m"), &[]).is_ok());
        assert!(parse_module("f(x:csverr):csverr := x;", 0, Some("m"), &[]).is_err());
        assert!(parse_module(r#"<< "csv"; f(x:csverr):csverr := x;"#, 0, Some("m"), &[]).is_ok());
        let p = parse_module(
            r#"row :: { x:q; }; a := { row :: { y:s; }; r:row := row {y:="ok";}; r };"#,
            100,
            Some("m"),
            &[],
        )
        .unwrap();
        let Stmt::Def {
            body: Expr::Block(ss, _),
            ..
        } = &p.defs[1]
        else {
            panic!()
        };
        let Stmt::Def {
            result_type: Some(t),
            ..
        } = &ss[1]
        else {
            panic!()
        };
        assert!(matches!(&t.kind,TypeAnnotationKind::Named(n) if n=="row"));
        let smuggled = format!("x := 0; y := {}m:x;", char::from(0));
        assert!(parse_module(&smuggled, 0, Some("m"), &[]).is_err());
    }

    #[test]
    fn import_discovery_preserves_original_paths_aliases_and_offsets() {
        let src = r#"-- comment
           x := "! << fake;";
           << "./folder/lib.flx" : dep;
           << csv;"#;
        let imports = discover_imports(src, 500).unwrap();
        assert_eq!(imports[0].path, "./folder/lib.flx");
        assert_eq!(imports[0].alias, "dep");
        assert_eq!(imports[0].start, 500 + src.find(r#"<< "./folder"#).unwrap());
        assert_eq!(imports[1].path, "csv");
        assert_eq!(imports[1].alias, "csv");
    }
    #[test]
    fn exact_numerals_keep_inline_threshold_and_raw_wide_spelling() {
        let source = concat!(
            "max := 170141183460469231731687303715884105727; ",
            "min := -170141183460469231731687303715884105728; ",
            "wide := 170141183460469231731687303715884105728; ",
            "below := -170141183460469231731687303715884105729; ",
            "deci := -001.23000d; ",
            "typed := + 340282366920938463463374607431768211456n 0n;"
        );
        let p = parse(source).unwrap();
        let body = |i: usize| match &p.defs[i] {
            Stmt::Def { body, .. } => body,
            _ => panic!(),
        };
        assert_eq!(body(0), &Expr::Int(i128::MAX));
        assert_eq!(body(1), &Expr::Int(i128::MIN));
        assert_eq!(
            body(2),
            &Expr::Num("170141183460469231731687303715884105728".into())
        );
        assert_eq!(
            body(3),
            &Expr::Num("-170141183460469231731687303715884105729".into())
        );
        assert_eq!(body(4), &Expr::Num("-001.23000d".into()));
        let Expr::Call(name, args) = body(5) else {
            panic!()
        };
        assert_eq!(name, "+");
        assert_eq!(args.len(), 2);
        assert!(matches!(&args[0],Expr::Num(raw) if raw.ends_with('n')));
        let spans = p.expression_origins();
        assert!(spans
            .iter()
            .any(|(e, s)| *e == body(4) && &source[s.clone()] == "-001.23000d"));
    }

    #[test]
    fn wide_literals_do_not_change_prefix_arity_or_lexical_constraints() {
        let wide = "170141183460469231731687303715884105728";
        for bad in [
            format!("x := + {wide};"),
            format!("x := + {wide} 1 2;"),
            format!("x := :d {wide} 2;"),
            format!("x := -{wide}n;"),
            format!("x := {wide}e4d;"),
            format!("x := {wide}.0z;"),
        ] {
            assert!(parse(&bad).is_err(), "{bad}");
        }
        assert!(parse(&format!("x := :d (+ {wide} 2);")).is_ok());
        assert!(parse(&format!("x := - {wide} 2;")).is_ok());
    }

    #[test]
    fn tacit_lowering_hygiene_and_origin_alignment() {
        let source = "identity := &.;\nvalue(x:z):z := x;\nmain := &{ @* + value (@? - .) };\n";
        let p = parse(source).unwrap();
        let Stmt::Def {
            body: Expr::Lambda(params, _),
            ..
        } = &p.defs[2]
        else {
            panic!("expected an ordinary synthetic lambda");
        };
        assert!(params[0].starts_with('\0'));
        let positions = p.expression_origins();
        assert!(positions
            .iter()
            .any(|(_, span)| &source[span.clone()] == "&."));
        assert!(positions.iter().any(
            |(node, span)| matches!(node, Expr::Ref(name) if name == "value")
                && &source[span.clone()] == "value"
        ));
        assert!(positions
            .iter()
            .any(|(_, span)| &source[span.clone()] == "(@? - .)"));
        let p = parse("fnn(x,y:z):z := - x y; alias := &{ (fnn) };").unwrap();
        assert!(matches!(&p.defs[1], Stmt::Def { body: Expr::Ref(name), .. } if name == "fnn"));
    }

    #[test]
    fn tacit_rejects_missing_extra_and_outside_forms_with_bounded_depth() {
        for source in [
            "x := .;",
            "x := @. . .;",
            "x := @* + . .;",
            "x := @? - .;",
            "x := &{};",
            "x := &{ . . };",
            "x := &{ @. . };",
            "x := &{ @* + . };",
            "x := &{ @? - };",
            "x := &{ $ };",
            "x := &{ ! };",
            "x := &{ 0 };",
            "x := & .;",
        ] {
            assert!(parse(source).is_err(), "{source}");
        }
        let source = format!("f := &{{ {} . }};", "@. . ".repeat(300));
        assert!(parse(&source).unwrap_err().message.contains("depth limit"));
    }

    #[test]
    fn imports_after_tacit_delimiters_are_discovered() {
        let text = "identity := &{ @. . . };\n<< \"./external.flx\" : ext;\n";
        let imports = discover_imports(text, 71).unwrap();
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].start, text.find("<<").unwrap() + 71);
        assert_eq!(imports[0].path, "./external.flx");
    }

    #[test]
    fn formatting_layout_preserves_original_calls_grouping_and_trivia() {
        let source = concat!(
            "n := + (* 2 3) -- between operands\n  :q 4;\n",
            "f := &{ @* + (@. .x .y) @? - . };\n"
        );
        let p = parse_with_layout(source).unwrap();
        let ordinary = parse(source).unwrap();
        assert!(ordinary.formatting_applications().is_empty());
        assert_eq!(ordinary.defs, p.defs);
        let forms = p
            .formatting_applications()
            .iter()
            .map(|a| {
                (
                    &source[a.operator.clone()],
                    a.operands
                        .iter()
                        .map(|r| &source[r.clone()])
                        .collect::<Vec<_>>(),
                    a.tacit,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            forms,
            [
                ("*", vec!["2", "3"], false),
                (":q", vec!["4"], false),
                ("+", vec!["(* 2 3)", ":q 4"], false),
                ("@.", vec![".x", ".y"], true),
                ("@?", vec!["-", "."], true),
                ("@*", vec!["+", "(@. .x .y)", "@? - ."], true),
            ]
        );
        let app = &p.formatting_applications()[2];
        assert_eq!(
            &source[app.span.clone()],
            "+ (* 2 3) -- between operands\n  :q 4"
        );
    }

    #[test]
    fn generic_sum_headers_payloads_and_match_origins() {
        let source = "tree[~a] :: { | leaf ~a; | node tree[~a] tree[~a]; }; f(t:tree[q]):q := ?= t { leaf x : x; node _ _ : 0q; };";
        let p = parse(source).unwrap();
        let Stmt::Sum {
            type_params,
            variants,
            ..
        } = &p.defs[0]
        else {
            panic!("expected generic sum");
        };
        assert_eq!(type_params, &["~a"]);
        assert_eq!(variants[1].payload.len(), 2);
        assert!(matches!(&variants[1].payload[0].kind,
            TypeAnnotationKind::NamedApply(name, args) if name == "tree" && args.len() == 1));
        let Stmt::Def {
            body: Expr::Match(_, arms),
            ..
        } = &p.defs[1]
        else {
            panic!("expected match");
        };
        assert_eq!(&source[arms[0].pattern.span.clone()], "leaf x");
        assert!(
            p.expression_origins()
                .iter()
                .any(|(body, span)| std::ptr::eq(*body, &arms[0].body)
                    && &source[span.clone()] == "x")
        );
        let formatted = crate::format_source(source).unwrap();
        assert!(formatted.contains("tree[~a]"));
        assert!(formatted.contains("tree[q]"));
        parse(&formatted).unwrap();
        assert_eq!(crate::format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn rejects_duplicate_pattern_binders_and_sum_parameters() {
        for (source, fragment) in [
            ("f(x) := ?= x { (a,a) : a; };", "duplicate match binder"),
            (
                "f(x) := ?= x { .{ a := _; a := _; } : (); };",
                "duplicate match field",
            ),
            (
                "f(x) := ?= x { lib.unknown : (); };",
                "unknown or unexported match constructor",
            ),
            (
                "tree[~a,~a] :: { | leaf ~a; };",
                "duplicate sum type parameter",
            ),
            ("tree[~a] :: { field:q; };", "only on nominal sums"),
        ] {
            assert!(
                parse(source).unwrap_err().message.contains(fragment),
                "{source}"
            );
        }
        assert!(parse_type_annotation("tree [q]").is_err());
    }

    #[test]
    fn explicit_open_row_annotation_parses_without_changing_closed_records() {
        use TypeAnnotationKind::*;
        let p = parse("keep(t:#.{ field:q; | ~r }):#.{ | ~r } := t;").unwrap();
        let Stmt::Def {
            param_types,
            result_type,
            ..
        } = &p.defs[0]
        else {
            panic!("expected function declaration");
        };
        let Table(t) = &param_types[0].as_ref().unwrap().kind else {
            panic!("expected table")
        };
        let OpenRecord(fields, row) = &t.kind else {
            panic!("expected open record")
        };
        assert_eq!(fields[0].0, "field");
        assert!(matches!(fields[0].1.kind, Primitive(TypePrimitive::Q)));
        assert_eq!(row, "~r");
        let Table(t) = &result_type.as_ref().unwrap().kind else {
            panic!("expected table")
        };
        assert!(matches!(&t.kind, OpenRecord(fields, row) if fields.is_empty() && row == "~r"));
        assert!(
            matches!(parse_type_annotation(".{ value:q }").unwrap().kind, Record(fields) if fields.len() == 1)
        );

        let input = "keep(t:#.{ field:q; x:n; | ~r }):#.{ | ~r } := t;";
        let formatted = crate::format_source(input).unwrap();
        let reparsed = parse(&formatted).unwrap();
        assert_eq!(p.defs.len(), reparsed.defs.len());
        assert!(formatted.contains("| ~r"));
        assert!(formatted.contains("field : q;"));
        assert!(formatted.contains("x     : n;"));
        assert_eq!(crate::format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn rejects_malformed_open_row_syntax() {
        for input in [
            ".{ | }",
            ".{ | r }",
            ".{ x:q | ~r }",
            ".{ x:q; | ~r; }",
            ".{ | ~r | ~t }",
            ".{ | ~r x:q; }",
            ".{ x:q; x:s; | ~r }",
            ".{ x:q; .. ~r }",
        ] {
            assert!(parse_type_annotation(input).is_err(), "{input}");
        }
    }

    #[test]
    fn type_links_rewrite_open_row_fields_without_rewriting_the_tail() {
        use TypeAnnotationKind::*;
        let lib = parse_module(">> (item); item :: { v:q; };", 0, Some("dep"), &[]).unwrap();
        let members = lib.module_members();
        let linked = members[0].linked.clone();
        let imports = [ImportedModule {
            path: "./dep.flx".into(),
            members,
        }];
        let src =
            r#"<< "./dep.flx" : d; keep(x:.{ value:d.item; | ~r }):.{ value:d.item; | ~r } := x;"#;
        let program = parse_module(src, 200, Some("main"), &imports).unwrap();
        let Stmt::Def {
            result_type: Some(ty),
            ..
        } = &program.defs[0]
        else {
            panic!("expected annotated definition");
        };
        let OpenRecord(fields, row) = &ty.kind else {
            panic!("expected open record")
        };
        assert!(matches!(&fields[0].1.kind, Named(name) if name == &linked));
        assert_eq!(row, "~r");
        assert!(ty.span.start >= 200);
    }

    #[test]
    fn built_in_array_type_needs_no_import_even_in_modules() {
        let p = parse_module(
            ">> (keep); keep(a:array[~a]):array[~a] := a; array := 1z;",
            0,
            Some("test"),
            &[],
        )
        .unwrap();
        let Stmt::Def {
            result_type: Some(ty),
            ..
        } = &p.defs[0]
        else {
            panic!("expected function")
        };
        assert!(
            matches!(&ty.kind, TypeAnnotationKind::NamedApply(name, args)
            if name == "array" && args.len() == 1)
        );
        assert!(parse("array := 1z;").is_ok());
        for bad in ["array :: { x:q; };", "array[~a] :: { | leaf ~a; };"] {
            assert!(parse(bad).unwrap_err().message.contains("built-in type"));
        }
    }

    #[test]
    fn callable_arity_hoists_and_lowers_with_hygiene_and_origins() {
        let source = "early := pick 2z; pick/1: z z -> z := (&+);";
        let p = parse(source).unwrap();
        let Stmt::Def {
            body: Expr::Call(_, args),
            ..
        } = &p.defs[0]
        else {
            panic!("forward call not hoisted");
        };
        assert_eq!(args.len(), 1);
        let Stmt::Def {
            params,
            param_types,
            result_type,
            body,
            ..
        } = &p.defs[1]
        else {
            panic!("expected lowered callable");
        };
        assert_eq!(params.len(), 1);
        assert!(params[0].starts_with('\0'));
        assert!(matches!(
            param_types[0].as_ref().unwrap().kind,
            TypeAnnotationKind::Primitive(TypePrimitive::Z)
        ));
        assert!(
            matches!(&result_type.as_ref().unwrap().kind, TypeAnnotationKind::Function(args, _) if args.len() == 1)
        );
        let Expr::Call(op, parts) = body else {
            panic!("expected generated application")
        };
        assert_eq!(op, "@");
        assert!(matches!(&parts[0], Expr::Ref(name) if name == "+"));
        assert!(matches!(&parts[1], Expr::Name(name) if name == &params[0]));
        let origins = p.expression_origins();
        assert!(origins
            .iter()
            .any(|(e, range)| std::ptr::eq(*e, body) && &source[range.clone()] == "(&+)"));
        assert!(origins
            .iter()
            .any(|(e, range)| std::ptr::eq(*e, &parts[1]) && &source[range.clone()] == "1"));
    }

    #[test]
    fn callable_arity_is_honored_in_nested_groups_and_module_interfaces() {
        let p = parse("main(x) := { answer := foo 1z 2z; foo/2:=&+; answer };").unwrap();
        let Stmt::Def {
            body: Expr::Block(decls, _),
            ..
        } = &p.defs[0]
        else {
            panic!("local block")
        };
        assert!(
            matches!(&decls[0], Stmt::Def { body: Expr::Call(name, args), .. }
            if name == "foo" && args.len() == 2)
        );
        let lib =
            parse_module(">> (pick); pick/2:z -> z -> z:=&+;", 100, Some("lib"), &[]).unwrap();
        assert_eq!(lib.module_members()[0].kind, MemberKind::Callable(2));
        let interface = [ImportedModule {
            path: "./lib.flx".into(),
            members: lib.module_members(),
        }];
        let root = parse_module(
            r#"<< "./lib.flx" : m; x := m.pick 3z 7z;"#,
            200,
            Some("main"),
            &interface,
        )
        .unwrap();
        assert!(
            matches!(&root.defs[0], Stmt::Def { body: Expr::Call(_, args), .. } if args.len() == 2)
        );
    }

    #[test]
    fn callable_arity_rejects_non_addressable_headers_and_right_hand_sides() {
        for (source, fragment) in [
            ("f /2 := &+;", "adjacent"),
            ("f/ 2 := &+;", "adjacent"),
            ("f/0 := &+;", "positive"),
            ("f/2n := &+;", "unsuffixed"),
            ("f/65 := &+;", "64"),
            ("f/2:q -> q := &+;", "at least 2"),
            ("f/1:_ -> q := &.;", "complete type"),
            ("f/1 | num ~a := &.;", "complete named function"),
            ("f/1 := 1z;", "right-hand side"),
            ("f/1 := (&.,1z);", "right-hand side"),
        ] {
            assert!(
                parse(source).unwrap_err().message.contains(fragment),
                "{source}"
            );
        }
        assert!(parse("x := / 4z 2z; f/1 := (&.);").is_ok());
    }

    #[test]
    fn tilde_variables_are_adjacent_only_in_all_type_positions() {
        let p = parse(
            "tree[~a] :: { | leaf ~a; | node tree[~a] tree[~a]; }; keep/1: ~a -> ~a | eq ~a := &.;",
        )
        .unwrap();
        let Stmt::Sum {
            type_params,
            variants,
            ..
        } = &p.defs[0]
        else {
            panic!("sum")
        };
        assert_eq!(type_params, &["~a"]);
        assert_eq!(variants[0].payload[0].source(), "~a");
        let Stmt::Def { start, .. } = &p.defs[1] else {
            panic!("definition")
        };
        assert_eq!(p.constraints[start], vec![("eq".into(), "~a".into())]);
        for source in [
            "id(x:~ a):~a := x;",
            "id(x:~-- split
a):~a := x;",
            "f(x:.{ | ~ r }):q := 0q;",
            "tree[~ a] :: { | leaf ~a; };",
            "tree[~a] :: { | leaf ~ a; };",
            "f(x:~a):~a | eq ~ a := x;",
        ] {
            assert!(
                parse(source)
                    .unwrap_err()
                    .message
                    .contains("adjacent tilde"),
                "{source}"
            );
        }
        assert!(parse("no(b:b):b := ~b; e := ~= 0.1r 1r 1r; c := 'a';").is_ok());
    }

    #[test]
    fn sequence_symbols_share_registry_arity_and_remain_referenceable() {
        let p = parse(
            "a := [: 2n [1 2 3]; b := :] 1n a; c := sect 1n 2n a; d := ,. [a b]; e := |: d; f := @/ &+ e; g := &|:; h := &{ ,. };"
        ).unwrap();
        for (index, (name, count)) in [
            ("[:", 2),
            (":]", 2),
            ("sect", 3),
            (",.", 1),
            ("|:", 1),
            ("@/", 2),
        ]
        .into_iter()
        .enumerate()
        {
            assert!(
                matches!(&p.defs[index], Stmt::Def { body: Expr::Call(op, args), .. }
                if op == name && args.len() == count),
                "{name}"
            );
        }
        assert!(matches!(&p.defs[6], Stmt::Def { body: Expr::Ref(op), .. } if op == "|:"));
        assert!(matches!(&p.defs[7], Stmt::Def { body: Expr::Ref(op), .. } if op == ",."));
        assert!(
            parse("row := .{ field := 2; }; x := (1,.field row); y := (1,.{ a := 2; });").is_ok()
        );
        assert!(parse("bad := @/ &+;")
            .unwrap_err()
            .message
            .contains("missing operand 2"));
    }
}
