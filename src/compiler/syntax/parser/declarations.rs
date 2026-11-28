//! Declaration headers, type annotation linkage and bodies.

use super::{
    annotation::{adjacent_type_variable, annotation_tokens, split_callable_annotation},
    is_qualified_ident, originated, Binding, Expr, PResult, Parser, Stmt, TypeAnnotation,
    TypeAnnotationKind, Variant,
};
use crate::{
    lexer::{is_ident, Delimiter, Token},
    Error,
};
use std::collections::HashSet;

impl Parser {
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
    pub(super) fn parameters(&mut self) -> PResult<(Vec<String>, Vec<Option<TypeAnnotation>>)> {
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
    pub(super) fn variant_at(&self, i: usize) -> PResult<(Variant, usize)> {
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

    pub(super) fn definition(&mut self, effect: bool) -> PResult<Stmt> {
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
}
