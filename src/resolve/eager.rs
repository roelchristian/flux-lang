//! Conservative analysis of reads before local initialization.

use super::{Resolution, SymbolId, SymbolKind};
use crate::parser::{Expr, MatchPattern, MatchPatternKind, Program, Stmt, SyntaxId};
use crate::Error;
use std::collections::HashMap;

// A local callable is in scope before its declaration, but local quantities
// are installed one at a time. Check that eagerly reachable code does not use
// an uninitialized slot. Suspended code is only followed at a known call or
// execute site. Opaque higher-order calls still have the evaluator's guard.
pub(super) fn verify_eager_locals(p: &Program, r: &Resolution) -> Result<(), Error> {
    let mut index = EagerIndex {
        resolution: r,
        defs: HashMap::new(),
        blocks: Vec::new(),
    };
    index.gather_stmts(&p.defs);
    if let Some(e) = &p.entry {
        index.gather_expr(e);
    }
    for (stmts, tail) in &index.blocks {
        let mut pending = stmts
            .iter()
            .filter_map(|s| r.declaration(s))
            .filter(|id| r.symbol(*id).kind == SymbolKind::Value)
            .collect::<std::collections::HashSet<_>>();
        if pending.is_empty() {
            continue;
        }
        for stmt in *stmts {
            if let Some((body, 0)) = stmt.body_and_arity() {
                index.check(body, &pending)?;
            }
            if let Some(id) = r.declaration(stmt) {
                pending.remove(&id);
            }
        }
        if let Some(tail) = tail {
            index.check(tail, &pending)?;
        }
    }
    Ok(())
}

struct EagerIndex<'a> {
    resolution: &'a Resolution,
    defs: HashMap<SymbolId, (&'a Expr, usize)>,
    blocks: Vec<(&'a [Stmt], Option<&'a Expr>)>,
}
impl<'a> EagerIndex<'a> {
    fn gather_stmts(&mut self, stmts: &'a [Stmt]) {
        for stmt in stmts {
            if let Some(body) = stmt.body_and_arity() {
                if let Some(id) = self.resolution.declaration(stmt) {
                    self.defs.insert(id, body);
                }
                self.gather_expr(body.0);
            }
        }
    }

    fn gather_expr(&mut self, e: &'a Expr) {
        if let Expr::Block(stmts, tail) | Expr::Action(stmts, tail) | Expr::Fallible(stmts, tail) =
            e
        {
            self.blocks.push((stmts, tail.as_deref()));
            self.gather_stmts(stmts);
            if let Some(tail) = tail {
                self.gather_expr(tail);
            }
        } else {
            e.for_each_child(|child| self.gather_expr(child));
        }
    }

    fn check(
        &self,
        e: &'a Expr,
        pending: &std::collections::HashSet<SymbolId>,
    ) -> Result<(), Error> {
        if !pending.is_empty() {
            let mut walk = EagerWalk {
                index: self,
                pending,
                active: std::collections::HashSet::new(),
                depth: 0,
                fuel: 20_000,
                masked: Vec::new(),
                pattern_values: HashMap::new(),
            };
            walk.expression(e)?;
        }
        Ok(())
    }
    fn known(&self, expr: &'a Expr, fuel: usize) -> Known<'a> {
        if fuel == 0 {
            return Known::Unknown;
        }
        match expr {
            Expr::Name(_) => {
                let id = self.resolution.reference(expr);
                if let Some(id) =
                    id.filter(|&id| self.resolution.symbol(id).kind == SymbolKind::Constructor(0))
                {
                    Known::Variant(id, &[])
                } else {
                    id.and_then(|id| self.defs.get(&id))
                        .and_then(|&(body, n)| (n == 0).then_some(body))
                        .map_or(Known::Unknown, |body| self.known(body, fuel - 1))
                }
            }
            Expr::Call(_, args)
                if self.resolution.reference(expr).is_some_and(|id| {
                    matches!(self.resolution.symbol(id).kind, SymbolKind::Constructor(_))
                }) =>
            {
                Known::Variant(self.resolution.reference(expr).unwrap(), args)
            }
            Expr::Ref(_) => self
                .resolution
                .reference(expr)
                .and_then(|id| self.defs.get(&id))
                .map_or(Known::Unknown, |&(body, n)| Known::Function(body, n)),
            Expr::Lambda(ps, body) | Expr::TypedLambda(ps, _, body) => {
                Known::Function(body, ps.len())
            }
            Expr::Action(ss, tail) => Known::Action(expr, ss, tail.as_deref()),
            Expr::Tuple(es) => Known::Tuple(es),
            Expr::Record(fs) | Expr::NamedRecord(_, fs) => Known::Record(fs),
            Expr::Call(n, args)
                if args.len() == 1
                    && n.starts_with('.')
                    && !matches!(n.as_str(), ".." | ".:")
                    && self.resolution.reference(expr).is_some_and(|id| {
                        self.resolution.symbol(id).kind == SymbolKind::External
                    }) =>
            {
                self.known(&args[0], fuel - 1)
                    .project(&n[1..])
                    .map_or(Known::Unknown, |e| self.known(e, fuel - 1))
            }
            Expr::Call(op, args)
                if matches!(op.as_str(), "?0" | "?1")
                    && self.resolution.reference(expr).is_some_and(|id| {
                        self.resolution.symbol(id).kind == SymbolKind::External
                    }) =>
            {
                Known::Optional(args.first())
            }
            Expr::Bool(b) => Known::Bool(*b),
            Expr::Unit => Known::Unit,
            _ => Known::Unknown,
        }
    }
}

#[derive(Clone, Copy)]
enum Known<'a> {
    Unknown,
    Bool(bool),
    Unit,
    Variant(SymbolId, &'a [Expr]),
    Tuple(&'a [Expr]),
    Record(&'a [(String, Expr)]),
    Optional(Option<&'a Expr>),
    Function(&'a Expr, usize),
    Action(&'a Expr, &'a [Stmt], Option<&'a Expr>),
}
impl<'a> Known<'a> {
    fn project(self, selector: &str) -> Option<&'a Expr> {
        match self {
            Self::Tuple(es) => es.get(selector.parse::<usize>().ok()?.checked_sub(1)?),
            Self::Record(fs) => fs.iter().find(|(n, _)| n == selector).map(|(_, e)| e),
            _ => None,
        }
    }
}
struct EagerWalk<'a, 's> {
    index: &'s EagerIndex<'a>,
    pending: &'s std::collections::HashSet<SymbolId>,
    masked: Vec<SymbolId>,
    pattern_values: HashMap<SymbolId, Known<'a>>,
    active: std::collections::HashSet<SyntaxId>,
    depth: usize,
    fuel: usize,
}
impl<'a> EagerWalk<'a, '_> {
    fn guarded(
        &mut self,
        owner: &'a Expr,
        f: impl FnOnce(&mut Self) -> Result<Known<'a>, Error>,
    ) -> Result<Known<'a>, Error> {
        let id = self
            .index
            .resolution
            .syntax_id(owner)
            .expect("indexed owner");
        if self.depth >= 128 || !self.active.insert(id) {
            return Ok(Known::Unknown);
        }
        self.depth += 1;
        let result = f(self);
        self.depth -= 1;
        self.active.remove(&id);
        result
    }
    fn apply(&mut self, f: Known<'a>, args: usize) -> Result<Known<'a>, Error> {
        if let Known::Function(body, n) = f {
            if args < n {
                return Ok(Known::Function(body, n - args));
            }
            if n != 0 {
                let result = self.guarded(body, |w| w.expression(body))?;
                if args > n {
                    return self.apply(result, args - n);
                }
                return Ok(result);
            }
        }
        Ok(Known::Unknown)
    }
    fn block(&mut self, ss: &'a [Stmt], tail: Option<&'a Expr>) -> Result<Known<'a>, Error> {
        // Recursion can instantiate the same lexical block again. Those
        // slots belong to a fresh frame; its own steps are checked separately.
        let mark = self.masked.len();
        self.masked.extend(
            ss.iter()
                .filter_map(|s| self.index.resolution.declaration(s)),
        );
        let result = (|| {
            for s in ss {
                match s {
                    Stmt::Def { params, body, .. } if params.is_empty() => {
                        self.expression(body)?;
                    }
                    Stmt::Effect { body, .. } => {
                        self.expression(body)?;
                    }
                    _ => {}
                }
            }
            tail.map_or(Ok(Known::Unknown), |e| self.expression(e))
        })();
        self.masked.truncate(mark);
        result
    }
    fn known_value(&self, e: &'a Expr, fuel: usize) -> Known<'a> {
        self.index
            .resolution
            .reference(e)
            .and_then(|id| self.pattern_values.get(&id).copied())
            .unwrap_or_else(|| self.index.known(e, fuel))
    }
    fn known_match(
        &self,
        pattern: &MatchPattern,
        value: Known<'a>,
        bindings: &mut Vec<(SymbolId, Known<'a>)>,
    ) -> Option<bool> {
        use MatchPatternKind::*;
        fn merge(values: impl Iterator<Item = Option<bool>>) -> Option<bool> {
            let mut unknown = false;
            for v in values {
                match v {
                    Some(false) => return Some(false),
                    None => unknown = true,
                    Some(true) => {}
                }
            }
            (!unknown).then_some(true)
        }
        match (&pattern.kind, value) {
            (Bind(_), _) => {
                if let Some(id) = self.index.resolution.match_binding(pattern) {
                    bindings.push((id, value));
                }
                Some(true)
            }
            (Wildcard, _) | (Unit, Known::Unit) => Some(true),
            (Bool(a), Known::Bool(b)) => Some(*a == b),
            (Optional(None), Known::Optional(payload)) => Some(payload.is_none()),
            (Optional(Some(_)), Known::Optional(None)) => Some(false),
            (Optional(Some(inner)), Known::Optional(Some(item))) => {
                self.known_match(inner, self.known_value(item, 32), bindings)
            }
            (Constructor(_, patterns), Known::Variant(id, items)) => {
                if self.index.resolution.match_constructor(pattern)? != id {
                    return Some(false);
                }
                if patterns.len() != items.len() {
                    return None;
                }
                merge(
                    patterns
                        .iter()
                        .zip(items)
                        .map(|(p, e)| self.known_match(p, self.known_value(e, 32), bindings)),
                )
            }
            (Tuple(patterns), Known::Tuple(items)) if patterns.len() == items.len() => merge(
                patterns
                    .iter()
                    .zip(items)
                    .map(|(p, e)| self.known_match(p, self.known_value(e, 32), bindings)),
            ),
            (Record(patterns), Known::Record(fields)) => {
                merge(patterns.iter().map(|(name, pattern)| {
                    let (_, item) = fields.iter().find(|(field, _)| field == name)?;
                    self.known_match(pattern, self.known_value(item, 32), bindings)
                }))
            }
            _ => None,
        }
    }
    fn match_body(
        &mut self,
        body: &'a Expr,
        bindings: Vec<(SymbolId, Known<'a>)>,
    ) -> Result<Known<'a>, Error> {
        // Nested re-entry can encounter the same syntactic arm. Restore any
        // outer abstract values on exit, rather than clearing them blindly.
        let previous: Vec<_> = bindings
            .into_iter()
            .map(|(id, value)| (id, self.pattern_values.insert(id, value)))
            .collect();
        let result = self.expression(body);
        for (id, prior) in previous.into_iter().rev() {
            if let Some(prior) = prior {
                self.pattern_values.insert(id, prior);
            } else {
                self.pattern_values.remove(&id);
            }
        }
        result
    }
    fn expression(&mut self, e: &'a Expr) -> Result<Known<'a>, Error> {
        // Recursion/highly shared call graphs cannot turn the checker into an
        // unbounded evaluator; unresolved paths retain the runtime guard.
        if self.fuel == 0 {
            return Ok(Known::Unknown);
        }
        self.fuel -= 1;
        let r = self.index.resolution;
        Ok(match e {
            Expr::Name(_) => {
                if let Some(id) = r
                    .reference(e)
                    .filter(|s| self.pending.contains(s) && !self.masked.contains(s))
                {
                    return Err(Error::new(
                        r.offset(e).unwrap_or(0),
                        format!(
                            "local `{}` is read before its initializer has finished",
                            r.symbol(id).name
                        ),
                    ));
                }
                self.known_value(e, 128)
            }
            Expr::Call(n, args) => {
                let id = r.reference(e);
                let external = id.is_some_and(|id| r.symbol(id).kind == SymbolKind::External);
                if external && n == "??" {
                    if let [fallback, optional] = args.as_slice() {
                        return match self.expression(optional)? {
                            Known::Optional(Some(_)) => Ok(Known::Unknown),
                            Known::Optional(None) => self.expression(fallback),
                            _ => {
                                self.expression(fallback)?;
                                Ok(Known::Unknown)
                            }
                        };
                    }
                }
                let values = args
                    .iter()
                    .map(|x| self.expression(x))
                    .collect::<Result<Vec<_>, _>>()?;
                if let Some(id) =
                    id.filter(|&id| matches!(r.symbol(id).kind, SymbolKind::Constructor(_)))
                {
                    return Ok(Known::Variant(id, args));
                }
                match (external, n.as_str(), values.as_slice()) {
                    (true, selector, [value])
                        if selector.starts_with('.') && !matches!(selector, ".." | ".:") =>
                    {
                        value
                            .project(&selector[1..])
                            .map_or(Known::Unknown, |e| self.index.known(e, 128))
                    }
                    (true, "?0", []) => Known::Optional(None),
                    (true, "?1", [_]) => Known::Optional(args.first()),
                    (true, "@", [f, _]) => self.apply(*f, 1)?,
                    (true, "@@", [f, Known::Tuple(es)]) => self.apply(*f, es.len())?,
                    (true, "!", [Known::Action(owner, ss, tail)]) => {
                        self.guarded(owner, |w| w.block(ss, *tail))?
                    }
                    _ => {
                        if let Some(&(body, count)) = id.and_then(|id| self.index.defs.get(&id)) {
                            self.apply(Known::Function(body, count), args.len())?
                        } else {
                            Known::Unknown
                        }
                    }
                }
            }
            Expr::Cases(pairs, tail) => {
                let mut uncertain = false;
                for (cond, value) in pairs {
                    match self.expression(cond)? {
                        Known::Bool(false) => continue,
                        Known::Bool(true) => {
                            let selected = self.expression(value)?;
                            return Ok(if uncertain { Known::Unknown } else { selected });
                        }
                        _ => {
                            uncertain = true;
                            self.expression(value)?;
                        }
                    }
                }
                let fallback = self.expression(tail)?;
                if uncertain {
                    Known::Unknown
                } else {
                    fallback
                }
            }
            Expr::Raise(body) | Expr::Propagate(body) => {
                self.expression(body)?;
                Known::Unknown
            }
            Expr::Recover(body, arms) => {
                self.expression(body)?;
                for arm in arms {
                    self.expression(&arm.body)?;
                }
                Known::Unknown
            }
            Expr::Match(body, arms) => {
                let value = self.expression(body)?;
                let mut uncertain = false;
                for arm in arms {
                    let mut bindings = Vec::new();
                    match self.known_match(&arm.pattern, value, &mut bindings) {
                        Some(false) => continue,
                        Some(true) => {
                            let selected = self.match_body(&arm.body, bindings)?;
                            return Ok(if uncertain { Known::Unknown } else { selected });
                        }
                        None => {
                            uncertain = true;
                            self.match_body(&arm.body, bindings)?;
                        }
                    }
                }
                Known::Unknown
            }
            Expr::Block(ss, tail) | Expr::Fallible(ss, tail) => self.block(ss, tail.as_deref())?,
            Expr::Array(es) | Expr::Tuple(es) | Expr::Pipeline(es) => {
                for x in es {
                    self.expression(x)?;
                }
                self.index.known(e, 128)
            }
            Expr::Record(fs) | Expr::NamedRecord(_, fs) => {
                for (_, x) in fs {
                    self.expression(x)?;
                }
                self.index.known(e, 128)
            }
            _ => self.index.known(e, 128),
        })
    }
}
