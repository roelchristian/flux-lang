//! Prefix expression parsing and source application layout.

use super::{tacit, Binding, Expr, FormattingApplication, Origin, PResult, Parser};
use crate::{lexer::decode_literal, Error};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};

impl Parser {
    pub(super) fn scoped_expression(
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

    pub(super) fn expr(&mut self, effect: bool) -> PResult<Expr> {
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
            "!^{" => self.combined_action(t.start),
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
}
