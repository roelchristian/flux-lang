//! Match and recovery patterns and lexical arm binders.

use super::{
    is_qualified_ident, Binding, Expr, MatchCase, MatchPattern, MatchPatternKind, PResult, Parser,
    RecoveryCase, RecoveryPattern,
};
use crate::{lexer::is_ident, Error};
use std::collections::HashSet;

impl Parser {
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

    pub(super) fn match_expression(&mut self, effect: bool) -> PResult<Expr> {
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

    pub(super) fn recover(&mut self, effect: bool) -> PResult<Expr> {
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
}
