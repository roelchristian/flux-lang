//! Canonical annotation AST, token normalisation and type grammar.

use super::{is_qualified_ident, PResult};
use crate::lexer::{is_ident, lex, Token};
use crate::Error;
use std::{collections::HashSet, ops::Range};

/// Canonical, syntax-only annotation with a half-open source byte range.
#[derive(Debug, Clone, PartialEq, Hash)]
pub struct TypeAnnotation {
    pub kind: TypeAnnotationKind,
    pub span: Range<usize>,
    source: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypePrimitive {
    N,
    Z,
    Q,
    D,
    R,
    S,
    B,
    C,
    Unit,
}
#[derive(Debug, Clone, PartialEq, Hash)]
pub enum TypeAnnotationKind {
    Primitive(TypePrimitive),
    Named(String),
    NamedApply(String, Vec<TypeAnnotation>),
    Variable(String),
    Hole,
    Optional(Box<TypeAnnotation>),
    Action(Box<TypeAnnotation>),
    Table(Box<TypeAnnotation>),
    PositionalTable(Vec<TypeAnnotation>),
    Sequence(Box<TypeAnnotation>),
    Tuple(Vec<TypeAnnotation>),
    Dictionary(Box<TypeAnnotation>, Box<TypeAnnotation>),
    Record(Vec<(String, TypeAnnotation)>),
    OpenRecord(Vec<(String, TypeAnnotation)>, String),
    Result(Box<TypeAnnotation>, Box<TypeAnnotation>),
    Function(Vec<TypeAnnotation>, Box<TypeAnnotation>),
}
impl TypeAnnotation {
    /// Normalized token spelling for the legacy oracle only.
    pub fn source(&self) -> &str {
        &self.source
    }
}
impl std::ops::Deref for TypeAnnotation {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        self.source()
    }
}
impl std::fmt::Display for TypeAnnotation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.source())
    }
}

/// Standalone type syntax for static library metadata. Program annotations
/// consume their original tokens directly and retain program-relative spans.
pub fn parse_type_annotation(source: &str) -> PResult<TypeAnnotation> {
    let tokens = lex(source)?;
    annotation_tokens(&tokens, source.len())
}

/// Shared recognition for annotations, row tails, sum headers and
/// qualifiers. The general lexer still emits the Boolean operator `~` as a
/// separate token and does not invent a notation in value expressions.
pub(super) fn adjacent_type_variable(tokens: &[Token], i: usize, eof: usize) -> PResult<Token> {
    let Some(sign) = tokens.get(i).filter(|t| t.text == "~") else {
        return Err(Error::new(
            tokens.get(i).map_or(eof, |t| t.start),
            "expected a type variable such as `~a`",
        ));
    };
    let Some(name) = tokens
        .get(i + 1)
        .filter(|t| sign.end == t.start && is_ident(&t.text))
    else {
        return Err(Error::new(
            sign.start,
            "write the type variable with adjacent tilde and name, such as `~a`",
        ));
    };
    Ok(Token {
        text: format!("~{}", name.text),
        start: sign.start,
        end: name.end,
    })
}

pub(super) fn annotation_tokens(input: &[Token], end: usize) -> PResult<TypeAnnotation> {
    let mut ts: Vec<Token> = Vec::with_capacity(input.len());
    let mut i = 0;
    while let Some(token) = input.get(i) {
        if token.text == "~" {
            ts.push(adjacent_type_variable(input, i, end)?);
            i += 2;
            continue;
        }
        i += 1;
        // Punctuation which is compound in expressions is separate in types.
        let words: &[&str] = match token.text.as_str() {
            "^?" => &["^", "?"],
            "^{" => &["^", "{"],
            "!{" => &["!", "{"],
            "?{" => &["?", "{"],
            ":#" => &[":", "#"],
            _ => &[],
        };
        if words.is_empty() {
            if token.text == "["
                && ts
                    .last()
                    .is_some_and(|t| t.text == "#" && t.end == token.start)
            {
                let t = ts.last_mut().unwrap();
                t.text.push('[');
                t.end = token.end;
            } else {
                ts.push(token.clone());
            }
        } else {
            let mut start = token.start;
            for &word in words {
                ts.push(Token {
                    text: word.to_owned(),
                    start,
                    end: start + word.len(),
                });
                start += word.len();
            }
        }
    }
    let mut p = AnnotationParser {
        t: &ts,
        i: 0,
        eof: end,
        depth: 0,
    };
    let ty = p.arrow(&[])?;
    if p.i != ts.len() {
        return p.err("trailing type annotation tokens");
    }
    Ok(ty)
}

struct AnnotationParser<'a> {
    t: &'a [Token],
    i: usize,
    eof: usize,
    depth: usize,
}
impl AnnotationParser<'_> {
    fn peek(&self) -> &str {
        self.t.get(self.i).map_or("", |t| t.text.as_str())
    }
    fn err<T>(&self, m: impl Into<String>) -> PResult<T> {
        Err(Error::new(
            self.t.get(self.i).map_or(self.eof, |t| t.start),
            m,
        ))
    }
    fn eat(&mut self, s: &str) -> bool {
        if self.peek() == s {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, s: &str) -> PResult<()> {
        if self.eat(s) {
            Ok(())
        } else {
            self.err(format!("expected `{s}` in type annotation"))
        }
    }
    fn node(&self, start: usize, kind: TypeAnnotationKind) -> TypeAnnotation {
        let ts = &self.t[start..self.i];
        TypeAnnotation {
            kind,
            span: ts[0].start..ts[ts.len() - 1].end,
            source: ts
                .iter()
                .map(|t| t.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        }
    }
    fn nested<T>(&mut self, f: impl FnOnce(&mut Self) -> PResult<T>) -> PResult<T> {
        if self.depth >= 128 {
            return self.err("type annotation depth limit exceeded");
        }
        self.depth += 1;
        let r = f(self);
        self.depth -= 1;
        r
    }
    fn arrow(&mut self, ends: &[&str]) -> PResult<TypeAnnotation> {
        self.nested(|p| {
            let start = p.i;
            let mut args = vec![p.result()?];
            while !p.peek().is_empty() && !ends.contains(&p.peek()) {
                if p.eat("->") {
                    let rhs = p.arrow(ends)?;
                    return Ok(p.node(start, TypeAnnotationKind::Function(args, Box::new(rhs))));
                }
                args.push(p.result()?);
            }
            if args.len() == 1 {
                Ok(args.remove(0))
            } else {
                p.err("multiple type components need an arrow")
            }
        })
    }
    fn result(&mut self) -> PResult<TypeAnnotation> {
        let start = self.i;
        let lhs = self.atom()?;
        if !self.eat("^") {
            return Ok(lhs);
        }
        let rhs = self.atom()?;
        if self.peek() == "^" {
            return self.err("result caret chains need explicit parentheses");
        }
        Ok(self.node(
            start,
            TypeAnnotationKind::Result(Box::new(lhs), Box::new(rhs)),
        ))
    }
    fn atom(&mut self) -> PResult<TypeAnnotation> {
        self.nested(Self::atom_impl)
    }
    fn atom_impl(&mut self) -> PResult<TypeAnnotation> {
        use TypeAnnotationKind::*;
        let start = self.i;
        let token = self.peek().to_owned();
        if token.is_empty() {
            return self.err("expected a type");
        }
        self.i += 1;
        let kind = match token.as_str() {
            "n" => Primitive(TypePrimitive::N),
            "z" => Primitive(TypePrimitive::Z),
            "q" => Primitive(TypePrimitive::Q),
            "d" => Primitive(TypePrimitive::D),
            "r" => Primitive(TypePrimitive::R),
            "s" => Primitive(TypePrimitive::S),
            "b" => Primitive(TypePrimitive::B),
            "c" => Primitive(TypePrimitive::C),
            "_" => Hole,
            "?" => Optional(Box::new(self.atom()?)),
            "!" => Action(Box::new(self.atom()?)),
            "#" => Table(Box::new(self.atom()?)),
            "[" => {
                let v = self.arrow(&["]"])?;
                self.expect("]")?;
                Sequence(Box::new(v))
            }
            "#[" => {
                let mut fields = Vec::new();
                while !self.eat("]") {
                    fields.push(self.result()?);
                    self.eat(",");
                }
                PositionalTable(fields)
            }
            "(" => {
                if self.eat(")") {
                    Primitive(TypePrimitive::Unit)
                } else {
                    let first = self.arrow(&[",", ")"])?;
                    if self.eat(")") {
                        return Ok(self.node(start, first.kind));
                    }
                    self.expect(",")?;
                    let mut fields = vec![first];
                    loop {
                        fields.push(self.arrow(&[",", ")"])?);
                        if !self.eat(",") {
                            break;
                        }
                    }
                    self.expect(")")?;
                    Tuple(fields)
                }
            }
            "{" => {
                let k = self.arrow(&[":"])?;
                self.expect(":")?;
                let v = self.arrow(&["}"])?;
                self.expect("}")?;
                Dictionary(Box::new(k), Box::new(v))
            }
            ".{" => {
                let mut fields = Vec::new();
                let mut seen = HashSet::new();
                loop {
                    if self.eat("}") {
                        break Record(fields);
                    }
                    if self.eat("|") {
                        let row = self.peek().to_owned();
                        if !row.strip_prefix('~').is_some_and(is_ident) {
                            return self.err("expected a row type variable such as `~r` after `|`");
                        }
                        self.i += 1;
                        self.expect("}")?;
                        break OpenRecord(fields, row);
                    }
                    let name = self.peek().to_owned();
                    if !is_ident(&name) {
                        return self.err("expected field name in structural record type");
                    }
                    if !seen.insert(name.clone()) {
                        return self.err(format!("duplicate field {name}"));
                    }
                    self.i += 1;
                    self.expect(":")?;
                    let value = self.arrow(&[";", "}", "|"])?;
                    fields.push((name, value));
                    if !self.eat(";") && self.peek() != "}" {
                        return self.err("expected `;` or `}` after a structural record field");
                    }
                }
            }
            name if name.strip_prefix('~').is_some_and(is_ident) => Variable(name.to_owned()),
            name if is_ident(name) || is_qualified_ident(name) => {
                if self.peek() == "[" && self.t[start].end == self.t[self.i].start {
                    self.i += 1;
                    let mut args = vec![self.arrow(&[",", "]"])?];
                    while self.eat(",") {
                        args.push(self.arrow(&[",", "]"])?);
                    }
                    self.expect("]")?;
                    NamedApply(name.to_owned(), args)
                } else {
                    Named(name.to_owned())
                }
            }
            _ => {
                self.i = start;
                return self.err(format!("unexpected token `{token}` in type annotation"));
            }
        };
        Ok(self.node(start, kind))
    }
}

// Post-order traversal mirrors the parser's expression completion order.
// These entries are owned by Program, so cloning/moving it does not move or
// invalidate any pointer keys. A later resolver associates current AST nodes.
pub(super) fn split_callable_annotation(
    signature: TypeAnnotation,
    arity: usize,
) -> PResult<(Vec<Option<TypeAnnotation>>, TypeAnnotation)> {
    fn complete(ty: &TypeAnnotation) -> bool {
        use TypeAnnotationKind::*;
        match &ty.kind {
            Hole => false,
            Optional(t) | Action(t) | Table(t) | Sequence(t) => complete(t),
            NamedApply(_, ts) | Tuple(ts) | PositionalTable(ts) => ts.iter().all(complete),
            Dictionary(a, b) | Result(a, b) => complete(a) && complete(b),
            Record(fields) | OpenRecord(fields, _) => fields.iter().all(|(_, t)| complete(t)),
            Function(args, ret) => args.iter().all(complete) && complete(ret),
            Named(_) | Variable(_) | Primitive(_) => true,
        }
    }
    if !complete(&signature) {
        return Err(Error::new(
            signature.span.start,
            "callable arity declaration requires a complete type; `_` is not allowed",
        ));
    }
    let start = signature.span.start;
    let mut params = Vec::with_capacity(arity);
    let mut tail = signature;
    while params.len() < arity {
        let TypeAnnotationKind::Function(mut args, out) = tail.kind else {
            return Err(Error::new(start, format!("callable arity /{arity} requires a type with at least {arity} argument domains")));
        };
        let take = (arity - params.len()).min(args.len());
        params.extend(args.drain(..take).map(Some));
        tail = if args.is_empty() {
            *out
        } else {
            let mut source = args
                .iter()
                .map(TypeAnnotation::source)
                .collect::<Vec<_>>()
                .join(" ");
            source.push_str(" -> ");
            source.push_str(out.source());
            TypeAnnotation {
                span: args[0].span.start..tail.span.end,
                kind: TypeAnnotationKind::Function(args, out),
                source,
            }
        };
    }
    Ok((params, tail))
}
