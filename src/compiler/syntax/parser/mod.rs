//! Strict prefix parser and its stable public syntax API.
use crate::builtins;
use crate::lexer::{is_ident, lex, Delimiter, Token};
pub use crate::Error;
use std::collections::{BTreeMap, HashMap, HashSet};

mod annotation;
mod ast;
mod blocks;
mod declarations;
mod expressions;
mod lookahead;
mod origins;
mod patterns;
mod program;
mod tacit;
#[cfg(test)]
mod tests;

pub use annotation::{parse_type_annotation, TypeAnnotation, TypeAnnotationKind, TypePrimitive};
pub use ast::{
    Expr, FormattingApplication, ImportDecl, ImportedModule, MatchCase, MatchPattern,
    MatchPatternKind, MemberKind, ModuleMember, Program, RecoveryCase, RecoveryPattern, Stmt,
    SyntaxId, Variant,
};
use origins::{originated, Origin};
pub use program::{discover_imports, parse, parse_module, parse_with_layout};
pub(crate) use program::{parse_module_with_layout, standard_owners};
pub(crate) use tacit::is_capture;

type PResult<T> = Result<T, Error>;
#[derive(Copy, Clone, Debug)]
enum Binding {
    Callable(usize),
    Constructor(usize),
    Value,
    Record,
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

    fn scope(&mut self) -> &mut HashMap<String, Binding> {
        self.scopes.last_mut().expect("parser scope")
    }
}
