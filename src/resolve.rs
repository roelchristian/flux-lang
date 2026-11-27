//! Lexical identities and dependency ordering for a parsed Flux program.
use crate::parser::{Expr, Program, Stmt};
use crate::{builtins, Error};
use std::{
    collections::{BTreeSet, HashMap},
    ops::Range,
};

/// Deterministic within the same parsed syntax tree; unrelated edits may renumber IDs.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct SymbolId(pub usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SymbolKind {
    Value,
    Callable(usize),
    Schema,
    Parameter,
    Stage,
    External,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Symbol {
    pub id: SymbolId,
    pub name: String,
    pub kind: SymbolKind,
    pub start: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Resolution {
    /// Program.defs indices in dependency order. A recursive group consists only
    /// of named function declarations. Its members are in source order.
    pub groups: Vec<Vec<usize>>,
    pub top: Vec<Option<SymbolId>>,
    pub symbols: Vec<Symbol>,
    refs: HashMap<usize, SymbolId>,
    defs: HashMap<usize, SymbolId>,
    params: HashMap<usize, Vec<SymbolId>>,
    lambdas: HashMap<usize, Vec<SymbolId>>,
    placeholders: HashMap<usize, SymbolId>,
    offsets: HashMap<usize, usize>,
    spans: HashMap<usize, Range<usize>>,
}

impl Resolution {
    /// Resolve against the same AST object that was passed to `resolve`, not a clone.
    pub fn reference(&self, expr: &Expr) -> Option<SymbolId> {
        self.refs.get(&key(expr)).copied()
    }
    pub fn declaration(&self, stmt: &Stmt) -> Option<SymbolId> {
        self.defs.get(&key(stmt)).copied()
    }
    pub fn parameters(&self, stmt: &Stmt) -> &[SymbolId] {
        self.params.get(&key(stmt)).map_or(&[], Vec::as_slice)
    }
    pub fn lambda_parameters(&self, lambda: &Expr) -> &[SymbolId] {
        self.lambdas.get(&key(lambda)).map_or(&[], Vec::as_slice)
    }
    /// Symbol for the preceding stage's value, visible within this stage.
    pub fn placeholder(&self, stage: &Expr) -> Option<SymbolId> {
        self.placeholders.get(&key(stage)).copied()
    }
    pub fn symbol(&self, id: SymbolId) -> &Symbol {
        &self.symbols[id.0]
    }
    /// Exact source expression envelope, if present. Synthetic expressions
    /// and manually changed AST trees have no exact source envelope.
    pub fn span(&self, expr: &Expr) -> Option<Range<usize>> {
        self.spans.get(&key(expr)).cloned()
    }
    /// Exact expression start or nearest declaration/entry fallback.
    pub fn offset(&self, expr: &Expr) -> Option<usize> {
        self.spans
            .get(&key(expr))
            .map(|s| s.start)
            .or_else(|| self.offsets.get(&key(expr)).copied())
    }
}

fn key<T>(node: &T) -> usize {
    node as *const T as usize
}

pub fn resolve(p: &Program) -> Result<Resolution, Error> {
    let mut r = Resolver {
        out: Resolution::default(),
        scopes: Vec::new(),
        external: HashMap::new(),
        roots: HashMap::new(),
        deps: vec![BTreeSet::new(); p.defs.len()],
        root: None,
        offset: 0,
    };
    r.out.spans = p
        .expression_origins()
        .into_iter()
        .map(|(expr, span)| (key(expr), span))
        .collect();
    r.scopes.push(HashMap::new());
    for (i, decl) in p.defs.iter().enumerate() {
        let id = r.allocate_decl(decl);
        if let Some(id) = id {
            r.bind(id)?;
            r.roots.insert(id, i);
        }
        r.out.top.push(id);
    }
    for (i, decl) in p.defs.iter().enumerate() {
        r.root = Some(i);
        r.statement(decl)?;
    }
    r.root = None;
    if let Some(expr) = &p.entry {
        r.offset = p.entry_start.unwrap_or(0);
        r.expression(expr)?;
    }
    r.out.groups = groups(&r.deps, p)?;
    Ok(r.out)
}

struct Resolver {
    out: Resolution,
    scopes: Vec<HashMap<String, SymbolId>>,
    external: HashMap<String, SymbolId>,
    roots: HashMap<SymbolId, usize>,
    deps: Vec<BTreeSet<usize>>,
    root: Option<usize>,
    offset: usize,
}

fn decl_name(s: &Stmt) -> Option<(&str, SymbolKind, usize)> {
    match s {
        Stmt::Def {
            name,
            params,
            start,
            ..
        } => Some((
            name,
            if params.is_empty() {
                SymbolKind::Value
            } else {
                SymbolKind::Callable(params.len())
            },
            *start,
        )),
        Stmt::Schema { name, start, .. } => Some((name, SymbolKind::Schema, *start)),
        Stmt::Effect {
            name: Some(name),
            start,
            ..
        } => Some((name, SymbolKind::Value, *start)),
        Stmt::Effect { name: None, .. } => None,
    }
}
fn stmt_offset(s: &Stmt) -> usize {
    match s {
        Stmt::Def { start, .. } | Stmt::Schema { start, .. } | Stmt::Effect { start, .. } => *start,
    }
}

impl Resolver {
    fn fresh(&mut self, name: &str, kind: SymbolKind, start: usize) -> SymbolId {
        let id = SymbolId(self.out.symbols.len());
        self.out.symbols.push(Symbol {
            id,
            name: name.to_owned(),
            kind,
            start,
        });
        id
    }
    fn allocate_decl(&mut self, stmt: &Stmt) -> Option<SymbolId> {
        let (name, kind, start) = decl_name(stmt)?;
        let id = self.fresh(name, kind, start);
        self.out.defs.insert(key(stmt), id);
        Some(id)
    }
    fn bind(&mut self, id: SymbolId) -> Result<(), Error> {
        let sym = self.out.symbol(id);
        let top = self.scopes.last_mut().expect("resolver scope");
        if top.contains_key(&sym.name) {
            return Err(Error::new(
                sym.start,
                format!("duplicate binding `{}`", sym.name),
            ));
        }
        top.insert(sym.name.clone(), id);
        Ok(())
    }
    fn lookup(&self, n: &str) -> Option<SymbolId> {
        self.scopes.iter().rev().find_map(|s| s.get(n).copied())
    }
    fn edge(&mut self, id: SymbolId) {
        if let (Some(owner), Some(&dependency)) = (self.root, self.roots.get(&id)) {
            self.deps[owner].insert(dependency);
        }
    }
    fn use_name(&mut self, e: &Expr, n: &str, usage: Usage) -> Result<(), Error> {
        let start = self.out.offset(e).unwrap_or(self.offset);
        let id = if let Some(id) = self.lookup(n) {
            let k = self.out.symbol(id).kind;
            let valid = match (usage, k) {
                (Usage::Plain, SymbolKind::Value | SymbolKind::Parameter | SymbolKind::Stage) => {
                    true
                }
                (Usage::Ref, SymbolKind::Callable(_)) => true,
                (Usage::Record, SymbolKind::Schema) => true,
                (Usage::Call(actual), SymbolKind::Callable(expected)) => actual == expected,
                _ => false,
            };
            if !valid {
                return Err(Error::new(
                    start,
                    format!("invalid use of `{n}` as {}", usage.description()),
                ));
            }
            id
        } else {
            match usage {
                Usage::Plain | Usage::Record => {
                    return Err(Error::new(start, format!("unknown binding `{n}`")));
                }
                Usage::Call(_) | Usage::Ref => {
                    // The parser already checked qualification. Keep registry
                    // arity validation here for consumers of a manually built AST.
                    let arity = builtins::lookup(n)
                        .map(|b| b.arity)
                        .or_else(|| builtins::runtime_arity(n))
                        .ok_or_else(|| Error::new(start, format!("unknown callable `{n}`")))?;
                    match usage {
                        Usage::Call(k) if k != arity => {
                            return Err(Error::new(
                                start,
                                format!("`{n}` expects {arity} operands, got {k}"),
                            ))
                        }
                        Usage::Ref if arity == 0 => {
                            return Err(Error::new(
                                start,
                                format!("`{n}` has no referenceable operand"),
                            ))
                        }
                        _ => {}
                    }
                    let canonical = builtins::canonical(n);
                    if let Some(&id) = self.external.get(canonical) {
                        id
                    } else {
                        let id = self.fresh(canonical, SymbolKind::External, start);
                        self.external.insert(canonical.to_owned(), id);
                        id
                    }
                }
            }
        };
        self.edge(id);
        self.out.refs.insert(key(e), id);
        Ok(())
    }
    fn parameter_scope(&mut self, names: &[String]) -> Result<Vec<SymbolId>, Error> {
        self.scopes.push(HashMap::new());
        let mut ids = Vec::with_capacity(names.len());
        for n in names {
            let id = self.fresh(n, SymbolKind::Parameter, self.offset);
            self.bind(id)?;
            ids.push(id);
        }
        Ok(ids)
    }
    fn statement(&mut self, s: &Stmt) -> Result<(), Error> {
        let outer = self.offset;
        self.offset = stmt_offset(s);
        match s {
            Stmt::Schema { .. } => {}
            Stmt::Def { params, body, .. } if !params.is_empty() => {
                let ids = self.parameter_scope(params)?;
                self.out.params.insert(key(s), ids);
                self.expression(body)?;
                self.scopes.pop();
            }
            Stmt::Def { body, .. } | Stmt::Effect { body, .. } => self.expression(body)?,
        }
        self.offset = outer;
        Ok(())
    }
    fn block(&mut self, stmts: &[Stmt], tail: Option<&Expr>) -> Result<(), Error> {
        self.scopes.push(HashMap::new());
        // Allocate all declarations in deterministic source order. Only callables
        // and schemas enter scope before the declaration site.
        for s in stmts {
            if let Some(id) = self.allocate_decl(s) {
                if matches!(
                    self.out.symbol(id).kind,
                    SymbolKind::Callable(_) | SymbolKind::Schema
                ) {
                    self.bind(id)?;
                }
            }
        }
        for s in stmts {
            self.statement(s)?;
            if let Some(id) = self.out.declaration(s) {
                if self.out.symbol(id).kind == SymbolKind::Value {
                    self.bind(id)?;
                }
            }
        }
        if let Some(e) = tail {
            self.expression(e)?;
        }
        self.scopes.pop();
        Ok(())
    }
    fn expression(&mut self, e: &Expr) -> Result<(), Error> {
        self.out.offsets.insert(key(e), self.offset);
        match e {
            Expr::Name(n) => self.use_name(e, n, Usage::Plain)?,
            Expr::Ref(n) => self.use_name(e, n, Usage::Ref)?,
            Expr::Call(n, args) => {
                self.use_name(e, n, Usage::Call(args.len()))?;
                for a in args {
                    self.expression(a)?;
                }
            }
            Expr::Tuple(es) | Expr::Array(es) => {
                for a in es {
                    self.expression(a)?;
                }
            }
            Expr::Cases(arms, fallback) => {
                for (cond, branch) in arms {
                    self.expression(cond)?;
                    self.expression(branch)?;
                }
                self.expression(fallback)?;
            }
            Expr::Lambda(params, body) | Expr::TypedLambda(params, _, body) => {
                let ids = self.parameter_scope(params)?;
                self.out.lambdas.insert(key(e), ids);
                self.expression(body)?;
                self.scopes.pop();
            }
            Expr::Pipeline(stages) => {
                if let Some((first, rest)) = stages.split_first() {
                    self.expression(first)?;
                    for stage in rest {
                        let id = self.fresh("$", SymbolKind::Stage, self.offset);
                        self.out.placeholders.insert(key(stage), id);
                        self.scopes.push(HashMap::new());
                        self.bind(id)?;
                        self.expression(stage)?;
                        self.scopes.pop();
                    }
                }
            }
            Expr::NamedRecord(n, fields) => {
                self.use_name(e, n, Usage::Record)?;
                for (_, v) in fields {
                    self.expression(v)?;
                }
            }
            Expr::Record(fields) => {
                for (_, v) in fields {
                    self.expression(v)?;
                }
            }
            Expr::Block(stmts, tail) | Expr::Action(stmts, tail) => {
                self.block(stmts, tail.as_deref())?
            }
            Expr::Int(_)
            | Expr::Num(_)
            | Expr::Str(_)
            | Expr::Bool(_)
            | Expr::Char(_)
            | Expr::Unit => {}
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Usage {
    Plain,
    Ref,
    Call(usize),
    Record,
}
impl Usage {
    fn description(self) -> &'static str {
        match self {
            Self::Plain => "a value",
            Self::Ref => "a callable reference",
            Self::Call(_) => "a direct callable",
            Self::Record => "a record constructor",
        }
    }
}

/// Iterative Kosaraju so a long dependency chain does not grow the Rust stack.
fn groups(edges: &[BTreeSet<usize>], p: &Program) -> Result<Vec<Vec<usize>>, Error> {
    let n = edges.len();
    let mut reverse = vec![Vec::new(); n];
    for (i, ds) in edges.iter().enumerate() {
        for &j in ds {
            reverse[j].push(i);
        }
    }
    let mut seen = vec![false; n];
    let mut postorder = Vec::with_capacity(n);
    for i in 0..n {
        if seen[i] {
            continue;
        }
        seen[i] = true;
        let mut stack = vec![(i, edges[i].iter())];
        while let Some((j, children)) = stack.last_mut() {
            if let Some(&k) = children.next() {
                if !seen[k] {
                    seen[k] = true;
                    stack.push((k, edges[k].iter()));
                }
            } else {
                postorder.push(*j);
                stack.pop();
            }
        }
    }
    seen.fill(false);
    let mut components = Vec::new();
    for i in postorder.into_iter().rev() {
        if seen[i] {
            continue;
        }
        seen[i] = true;
        let mut stack = vec![i];
        let mut component = Vec::new();
        while let Some(j) = stack.pop() {
            component.push(j);
            for &k in &reverse[j] {
                if !seen[k] {
                    seen[k] = true;
                    stack.push(k);
                }
            }
        }
        component.sort_unstable();
        if component.len() > 1 || edges[i].contains(&i) {
            if let Some(&v) = component
                .iter()
                .find(|&&j| !matches!(decl_name(&p.defs[j]), Some((_, SymbolKind::Callable(_), _))))
            {
                let names = component
                    .iter()
                    .filter_map(|&j| decl_name(&p.defs[j]).map(|(n, _, _)| n))
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(Error::new(
                    stmt_offset(&p.defs[v]),
                    format!("cyclic value dependency involving {names}"),
                ));
            }
        }
        components.push(component);
    }
    components.reverse();
    Ok(components)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    #[test]
    fn orders_deps_and_allows_function_sccs() {
        let p = parse("a := f 0; f(x:n):n := ? (= x 0) b (g x); g(x:n):n := f x; b := 3;").unwrap();
        assert_eq!(
            resolve(&p).unwrap().groups,
            vec![vec![3], vec![1, 2], vec![0]]
        );
    }

    #[test]
    fn rejects_value_cycles_at_the_value_declaration() {
        let src = "f(x:n):n := a;\na := f 0;";
        let p = parse(src).unwrap();
        let e = resolve(&p).unwrap_err();
        assert_eq!(e.start, src.find("a :=").unwrap());
        assert!(e.message.contains("cyclic value dependency"));
        assert!(resolve(&parse("a := a;").unwrap()).is_err());
    }

    #[test]
    fn resolves_declaration_site_shadowing_and_stage_captures() {
        let p = parse("avg := 1; p := { f(x:n):z := + avg x; avg := 2; f 0 }; q := |{ 1; |{ $; + $ 1 }; + $ 1 };").unwrap();
        let r = resolve(&p).unwrap();
        let Stmt::Def {
            body: Expr::Block(stmts, _),
            ..
        } = &p.defs[1]
        else {
            panic!()
        };
        let Stmt::Def {
            body: Expr::Call(_, args),
            ..
        } = &stmts[0]
        else {
            panic!()
        };
        assert_eq!(r.reference(&args[0]), r.top[0]);
        assert_ne!(r.reference(&args[0]), r.declaration(&stmts[1]));
        let Stmt::Def {
            body: Expr::Pipeline(outer),
            ..
        } = &p.defs[2]
        else {
            panic!()
        };
        let Expr::Pipeline(inner) = &outer[1] else {
            panic!()
        };
        assert_eq!(r.reference(&inner[0]), r.placeholder(&outer[1]));
        let Expr::Call(_, ops) = &inner[1] else {
            panic!()
        };
        assert_eq!(r.reference(&ops[0]), r.placeholder(&inner[1]));
        assert_ne!(r.placeholder(&inner[1]), r.placeholder(&outer[1]));
    }

    #[test]
    fn stable_ids_and_alias_identity_on_fresh_parse() {
        let source = r#"<< "statistics" : st;
            f(x:[q]):?q := st.mean x;
            g(y:[q]):?q := avg y;
            fit := \(x:q) -> |{ x; + $ x };
        "#;
        let p = parse(source).unwrap();
        let q = parse(source).unwrap();
        let r = resolve(&p).unwrap();
        let s = resolve(&q).unwrap();
        assert_eq!(r.symbols, s.symbols);
        assert_eq!(r.groups, s.groups);
        let (Stmt::Def { body: a, .. }, Stmt::Def { body: b, .. }) = (&p.defs[0], &p.defs[1])
        else {
            panic!()
        };
        assert_eq!(r.reference(a), r.reference(b));
        assert_eq!(r.symbol(r.reference(a).unwrap()).name, "avg");
    }

    #[test]
    fn rejects_invalid_external_arity_on_synthetic_ast() {
        let mut p = parse("x := + 1 2;").unwrap();
        let Stmt::Def {
            body: Expr::Call(_, operands),
            ..
        } = &mut p.defs[0]
        else {
            panic!()
        };
        operands.pop();
        assert!(resolve(&p)
            .unwrap_err()
            .message
            .contains("expects 2 operands"));
    }

    #[test]
    fn expression_origins_follow_owned_ast_and_preserve_subranges() {
        let source = r#"
           t := #[ (1,2) (3,4) ];
           !{
               n := ?{ 1b : (+ 5 (- 8 7)); _ : 2; };
               |{ n; + $ 1 }
           };
        "#;
        let moved = Box::new(parse(source).unwrap().clone());
        let r = resolve(&moved).unwrap();
        let show = |e: &Expr| r.span(e).map(|range| &source[range]);
        let Stmt::Def {
            body: table @ Expr::Call(_, args),
            ..
        } = &moved.defs[0]
        else {
            panic!()
        };
        assert_eq!(show(table), Some("#[ (1,2) (3,4) ]"));
        assert_eq!(show(&args[0]), None);
        let Expr::Array(rows) = &args[0] else {
            panic!()
        };
        assert_eq!(show(&rows[0]), Some("(1,2)"));
        let entry @ Expr::Action(stmts, Some(tail)) = moved.entry.as_ref().unwrap() else {
            panic!()
        };
        assert!(show(entry).unwrap().starts_with("!{"));
        let Stmt::Def {
            body: Expr::Cases(pairs, _),
            ..
        } = &stmts[0]
        else {
            panic!()
        };
        let (guard, value) = &pairs[0];
        assert_eq!(show(guard), Some("1b"));
        assert_eq!(show(value), Some("(+ 5 (- 8 7))"));
        let Expr::Call(_, ops) = value else { panic!() };
        assert_eq!(show(&ops[0]), Some("5"));
        assert_eq!(show(&ops[1]), Some("(- 8 7)"));
        assert_eq!(show(tail), Some("|{ n; + $ 1 }"));
    }

    #[test]
    fn changed_ast_does_not_inherit_unverified_expression_ranges() {
        let mut p = parse("q := + 1 2;").unwrap();
        let Stmt::Def {
            body: Expr::Call(_, operands),
            ..
        } = &mut p.defs[0]
        else {
            panic!()
        };
        operands[1] = Expr::Int(9);
        let r = resolve(&p).unwrap();
        let Stmt::Def {
            body: call @ Expr::Call(_, operands),
            ..
        } = &p.defs[0]
        else {
            panic!()
        };
        assert_eq!(r.span(call), None);
        assert_eq!(r.span(&operands[1]), None);
        assert_eq!(r.offset(&operands[1]), Some(0));
    }
}
