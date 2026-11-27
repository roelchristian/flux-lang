//! Rank-one inference for the Draft 0.4 checked subset.
use crate::{
    parser::{
        Expr, Program, RecoveryCase, RecoveryPattern, Stmt, TypeAnnotation, TypeAnnotationKind,
        TypePrimitive,
    },
    resolve::{Resolution, SymbolId},
    types::{Class, Constraint, InferCtx, NominalId, Prim, Row, Scheme, Type},
};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InferErrorKind {
    TypeError,
    IncompatibleError,
    ResultNeedsHandling,
    Unsupported,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InferError {
    pub kind: InferErrorKind,
    pub start: usize,
    pub end: usize,
    pub message: String,
    pub expected: Option<String>,
    pub actual: Option<String>,
}
impl std::fmt::Display for InferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for InferError {}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    Builtin(String),
    Numeric { operator: String, result: Type },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Constructor {
    pub owner: NominalId,
    pub id: SymbolId,
    pub payload: Vec<Type>,
}
#[derive(Debug, Default)]
pub struct InferResult {
    pub expressions: HashMap<usize, Type>,
    pub bindings: HashMap<SymbolId, Scheme>,
    pub operations: HashMap<usize, Operation>,
    pub callees: HashMap<usize, Type>,
    pub schemas: HashMap<NominalId, Row>,
    pub constructors: HashMap<SymbolId, Constructor>,
    pub boundaries: HashMap<usize, usize>,
}
fn address(e: &Expr) -> usize {
    (e as *const Expr) as usize
}
impl InferResult {
    pub fn constructor(&self, id: SymbolId) -> Option<&Constructor> {
        self.constructors.get(&id)
    }
    pub fn boundary_of(&self, e: &Expr) -> Option<usize> {
        self.boundaries.get(&address(e)).copied()
    }
    pub fn nominal_schema(&self, id: &NominalId) -> Option<&Row> {
        self.schemas.get(id)
    }
    pub fn type_of(&self, e: &Expr) -> Option<&Type> {
        self.expressions.get(&address(e))
    }
    pub fn binding_type(&self, id: SymbolId) -> Option<&Scheme> {
        self.bindings.get(&id)
    }
    pub fn callee_type(&self, e: &Expr) -> Option<&Type> {
        self.callees.get(&address(e))
    }
    pub fn operation_of(&self, e: &Expr) -> Option<&Operation> {
        self.operations.get(&address(e))
    }
}
type R<T> = Result<T, InferError>;
type Env = HashMap<String, Scheme>;
fn b(t: Type) -> Box<Type> {
    Box::new(t)
}
fn p(t: Prim) -> Type {
    Type::Prim(t)
}
fn result(ok: Type, err: Type) -> Type {
    Type::Result(b(ok), b(err))
}
fn s(t: Type) -> Type {
    Type::Seq(b(t))
}
fn o(t: Type) -> Type {
    Type::Optional(b(t))
}
fn a(t: Type) -> Type {
    Type::Action(b(t))
}
fn f(arg: Type, result: Type) -> Type {
    Type::Fun(b(arg), b(result))
}
fn function(args: impl DoubleEndedIterator<Item = Type>, out: Type) -> Type {
    args.rev().fold(out, |out, arg| f(arg, out))
}
fn mono(ty: Type) -> Scheme {
    Scheme {
        vars: vec![],
        ty,
        constraints: vec![],
    }
}
fn syntactic_value(e: &Expr) -> bool {
    match e {
        Expr::Int(_)
        | Expr::Num(_)
        | Expr::Str(_)
        | Expr::Char(_)
        | Expr::Bool(_)
        | Expr::Unit
        | Expr::Name(_)
        | Expr::Ref(_)
        | Expr::Lambda(..)
        | Expr::TypedLambda(..) => true,
        Expr::Tuple(es) | Expr::Array(es) => es.iter().all(syntactic_value),
        Expr::Record(fs) | Expr::NamedRecord(_, fs) => fs.iter().all(|(_, x)| syntactic_value(x)),
        _ => false,
    }
}
fn scan_dependencies(
    e: &Expr,
    res: &Resolution,
    local: &HashSet<SymbolId>,
    out: &mut HashSet<SymbolId>,
) {
    if let Some(id) = res.reference(e) {
        if local.contains(&id) {
            out.insert(id);
        }
    }
    use Expr::*;
    let mut visit = |e: &Expr| scan_dependencies(e, res, local, out);
    match e {
        Raise(x) | Propagate(x) => visit(x),
        Recover(value, cases) => {
            visit(value);
            for case in cases {
                visit(&case.body);
            }
        }
        Call(_, xs) | Array(xs) | Tuple(xs) | Pipeline(xs) => {
            for x in xs {
                visit(x)
            }
        }
        Lambda(_, x) | TypedLambda(_, _, x) => visit(x),
        Cases(cs, alt) => {
            for (c, e) in cs {
                visit(c);
                visit(e);
            }
            visit(alt);
        }
        Record(fs) | NamedRecord(_, fs) => {
            for (_, e) in fs {
                visit(e);
            }
        }
        Block(ds, tail) | Action(ds, tail) | Fallible(ds, tail) => {
            for d in ds {
                if let Stmt::Def { body, .. } | Stmt::Effect { body, .. } = d {
                    visit(body);
                }
            }
            if let Some(t) = tail {
                visit(t);
            }
        }
        _ => {}
    }
}
fn exact(t: Prim) -> bool {
    matches!(t, Prim::N | Prim::Z | Prim::Q | Prim::D)
}
fn num(t: Prim) -> bool {
    exact(t) || t == Prim::R
}
fn type_has_nominal(t: &Type) -> bool {
    match t {
        Type::Nominal(_) => true,
        Type::Seq(t) | Type::Table(t) | Type::Optional(t) | Type::Action(t) => type_has_nominal(t),
        Type::Fun(a, b) | Type::Dict(a, b) | Type::Result(a, b) => {
            type_has_nominal(a) || type_has_nominal(b)
        }
        Type::Tuple(ts) => ts.iter().any(type_has_nominal),
        Type::Record(row) => row.fields.iter().any(|(_, t)| type_has_nominal(t)),
        _ => false,
    }
}
fn solve_numeric(a: Prim, b: Prim) -> Option<Prim> {
    use crate::builtins::{promote, Numeric as K};
    fn to(x: Prim) -> Option<K> {
        Some(match x {
            Prim::N => K::N,
            Prim::Z => K::Z,
            Prim::Q => K::Q,
            Prim::D => K::D,
            Prim::R => K::R,
            _ => return None,
        })
    }
    Some(match promote(to(a)?, to(b)?)? {
        K::N => Prim::N,
        K::Z => Prim::Z,
        K::Q => Prim::Q,
        K::D => Prim::D,
        K::R => Prim::R,
    })
}

#[derive(Clone)]
struct Signature {
    args: Vec<Type>,
    out: Type,
    explicit: HashMap<String, Type>,
    qualifiers: Vec<Constraint>,
}
struct Engine<'r> {
    cx: InferCtx,
    result: InferResult,
    resolver: &'r Resolution,
    schemas: HashMap<NominalId, Row>,
    schema_scopes: Vec<HashMap<String, NominalId>>,
    sums: HashMap<NominalId, Vec<SymbolId>>,
    boundaries: Vec<(usize, Type)>,
    discarded_results: Vec<(Type, usize, usize)>,
    entry_natural_tail: Option<usize>,
    entry_boundary: Option<usize>,
    raise_payload: Option<usize>,
    saw_result: bool,
    start: usize,
    literals: Vec<(Type, i128, usize, usize)>,
    active: Option<std::ops::Range<usize>>,
    empty_sequences: Vec<(Type, usize, usize)>,
    explicit_variables: HashSet<u32>,
    qualifiers: &'r std::collections::BTreeMap<usize, Vec<(String, String)>>,
}
impl Engine<'_> {
    fn err(&self, text: impl Into<String>) -> InferError {
        InferError {
            kind: InferErrorKind::TypeError,
            start: self.start,
            end: self
                .active
                .as_ref()
                .filter(|s| s.start == self.start)
                .map_or(self.start, |s| s.end),
            expected: None,
            actual: None,
            message: text.into(),
        }
    }
    fn unsupported(&self, text: impl Into<String>) -> InferError {
        InferError {
            kind: InferErrorKind::Unsupported,
            start: self.start,
            end: self
                .active
                .as_ref()
                .filter(|s| s.start == self.start)
                .map_or(self.start, |s| s.end),
            expected: None,
            actual: None,
            message: text.into(),
        }
    }
    fn error_conflict(&self, actual: &Type, expected: &Type) -> Option<(Type, Type)> {
        match (actual, expected) {
            (Type::Action(x), Type::Action(y)) => self.error_conflict(x, y),
            (Type::Result(_, a), Type::Result(_, b))
                if a != b && self.cx.free_vars(a).is_empty() && self.cx.free_vars(b).is_empty() =>
            {
                Some((a.as_ref().clone(), b.as_ref().clone()))
            }
            _ => None,
        }
    }
    fn incompatible(&self, actual: &Type, expected: &Type) -> InferError {
        let mut e = self.err(format!(
            "incompatible error type: expected {expected}, found {actual}"
        ));
        e.kind = InferErrorKind::IncompatibleError;
        e.expected = Some(expected.to_string());
        e.actual = Some(actual.to_string());
        e
    }
    fn unify(&mut self, x: &Type, y: &Type) -> R<()> {
        // The generic unifier cannot resolve nominal schema names. Check any
        // pending class obligations that are about to take on a nominal type.
        let left = self.cx.resolve(x);
        let right = self.cx.resolve(y);
        let action_result = matches!((&left, &right), (Type::Action(a), Type::Action(b))
            if matches!(a.as_ref(), Type::Result(..))
                && !matches!(b.as_ref(), Type::Result(..) | Type::Var(_) | Type::Never));
        if action_result
            || (matches!(left, Type::Result(..))
                && !matches!(right, Type::Result(..) | Type::Var(_) | Type::Never))
        {
            let mut e = self.err(format!(
                "a fallible result cannot be used as {right} without handling its error"
            ));
            e.kind = InferErrorKind::ResultNeedsHandling;
            e.expected = Some(right.to_string());
            e.actual = Some(left.to_string());
            return Err(e);
        }
        let has_nominal = type_has_nominal(&left) || type_has_nominal(&right);
        let error_conflict = if self.saw_result {
            self.error_conflict(&left, &right)
        } else {
            None
        };
        let pending = if has_nominal {
            self.cx.constraints()
        } else {
            Vec::new()
        };
        self.cx
            .unify_committing(x, y)
            .map_err(|e| match error_conflict {
                Some((actual, expected)) => self.incompatible(&actual, &expected),
                None => self.err(e),
            })?;
        for c in pending {
            self.nominal_evidence(c.class, &c.ty, &mut HashSet::new())?;
        }
        Ok(())
    }
    fn req(&mut self, c: Class, t: &Type) -> R<()> {
        self.nominal_evidence(c, t, &mut HashSet::new())?;
        self.cx.require(c, t).map_err(|e| self.err(e))
    }
    fn nominal_evidence(&mut self, c: Class, t: &Type, seen: &mut HashSet<NominalId>) -> R<()> {
        if !matches!(c, Class::Eq | Class::Hash) {
            return Ok(());
        }
        match self.cx.resolve(t) {
            Type::Nominal(n) if seen.insert(n.clone()) => {
                // Hash remains intentionally unavailable for nominal records.
                if c == Class::Hash {
                    return Err(
                        self.err(format!("nominal record {} does not satisfy hash", n.name))
                    );
                }
                if let Some(ids) = self.sums.get(&n).cloned() {
                    for id in ids {
                        let payload = self.result.constructors[&id].payload.clone();
                        for t in payload {
                            self.nominal_evidence(c, &t, seen)?;
                            self.cx.require(c, &t).map_err(|text| {
                                self.err(format!(
                                    "constructor {} payload: {text}",
                                    self.resolver.symbol(id).name
                                ))
                            })?;
                        }
                    }
                } else {
                    let row = self
                        .schemas
                        .get(&n)
                        .cloned()
                        .ok_or_else(|| self.err(format!("unknown schema {}", n.name)))?;
                    for (field, t) in row.fields {
                        self.nominal_evidence(c, &t, seen)?;
                        self.cx.require(c, &t).map_err(|text| {
                            self.err(format!("field {}.{field}: {text}", n.name))
                        })?;
                    }
                }
            }
            Type::Seq(x) | Type::Table(x) | Type::Optional(x) => {
                self.nominal_evidence(c, &x, seen)?
            }
            Type::Dict(k, v) | Type::Result(k, v) => {
                self.nominal_evidence(c, &k, seen)?;
                self.nominal_evidence(c, &v, seen)?;
            }
            Type::Tuple(ts) => {
                for t in ts {
                    self.nominal_evidence(c, &t, seen)?;
                }
            }
            Type::Record(r) => {
                for (_, t) in r.fields {
                    self.nominal_evidence(c, &t, seen)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn annotation(&mut self, t: &TypeAnnotation, vars: &mut HashMap<String, Type>) -> R<Type> {
        let start = self.start;
        let active = self.active.take();
        // Program annotations carry authored source positions; standalone library
        // descriptors are checked by their schema construction call site.
        if !t.span.is_empty() {
            self.start = t.span.start;
            self.active = Some(t.span.clone());
        }
        let checked = self.annotation_inner(t, vars);
        self.start = start;
        self.active = active;
        checked
    }
    fn annotation_inner(
        &mut self,
        t: &TypeAnnotation,
        vars: &mut HashMap<String, Type>,
    ) -> R<Type> {
        use TypeAnnotationKind as K;
        Ok(match &t.kind {
            K::Primitive(p0) => p(match p0 {
                TypePrimitive::N => Prim::N,
                TypePrimitive::Z => Prim::Z,
                TypePrimitive::Q => Prim::Q,
                TypePrimitive::D => Prim::D,
                TypePrimitive::R => Prim::R,
                TypePrimitive::S => Prim::S,
                TypePrimitive::B => Prim::B,
                TypePrimitive::C => Prim::C,
                TypePrimitive::Unit => Prim::Unit,
            }),
            K::Named(name) => Type::Nominal(
                self.schema_id(name)
                    .ok_or_else(|| self.err(format!("unknown type {name}")))?
                    .clone(),
            ),
            K::Variable(name) => vars
                .entry(name.clone())
                .or_insert_with(|| self.cx.fresh())
                .clone(),
            K::Hole => self.cx.fresh(),
            K::Optional(t) => o(self.annotation(t, vars)?),
            K::Action(t) => a(self.annotation(t, vars)?),
            K::Table(t) => Type::Table(b(self.annotation(t, vars)?)),
            K::PositionalTable(ts) => Type::Table(b(Type::Tuple(
                ts.iter()
                    .map(|x| self.annotation(x, vars))
                    .collect::<R<_>>()?,
            ))),
            K::Sequence(t) => s(self.annotation(t, vars)?),
            K::Tuple(ts) => Type::Tuple(
                ts.iter()
                    .map(|x| self.annotation(x, vars))
                    .collect::<R<_>>()?,
            ),
            K::Dictionary(k, v) => {
                Type::Dict(b(self.annotation(k, vars)?), b(self.annotation(v, vars)?))
            }
            K::Record(fs) => Type::Record(Row {
                fields: fs
                    .iter()
                    .map(|(name, t)| Ok((name.clone(), self.annotation(t, vars)?)))
                    .collect::<R<_>>()?,
                tail: None,
            }),
            K::Result(ok, err) => {
                self.saw_result = true;
                result(self.annotation(ok, vars)?, self.annotation(err, vars)?)
            }
            K::Function(xs, y) => {
                let args = xs
                    .iter()
                    .map(|x| self.annotation(x, vars))
                    .collect::<R<Vec<_>>>()?;
                function(args.into_iter(), self.annotation(y, vars)?)
            }
        })
    }
    fn signature(
        &mut self,
        params: &[String],
        types: &[Option<TypeAnnotation>],
        out: Option<&TypeAnnotation>,
    ) -> R<Signature> {
        let mut vars = HashMap::new();
        let mut args = Vec::new();
        for (i, _) in params.iter().enumerate() {
            args.push(match types.get(i).and_then(Option::as_ref) {
                Some(t) => self.annotation(t, &mut vars)?,
                None => self.cx.fresh(),
            })
        }
        let out = if let Some(t) = out {
            self.annotation(t, &mut vars)?
        } else {
            self.cx.fresh()
        };
        Ok(Signature {
            args,
            out,
            explicit: vars,
            qualifiers: vec![],
        })
    }
    fn standard_type(name: &str) -> Type {
        Type::Nominal(NominalId {
            id: crate::errors041::owner_id(name).expect("known standard error type"),
            name: name.to_owned(),
        })
    }
    fn install_standard_errors(&mut self, imports: &[String]) -> R<()> {
        for &name in crate::errors041::OWNERS {
            if !imports
                .iter()
                .any(|module| crate::parser::standard_owners(module).contains(&name))
            {
                continue;
            }
            let Type::Nominal(id) = Self::standard_type(name) else {
                unreachable!()
            };
            self.schema_scopes[0].insert(name.into(), id);
        }
        for &name in crate::errors041::OWNERS {
            let Type::Nominal(owner) = Self::standard_type(name) else {
                unreachable!()
            };
            let mut ids = Vec::new();
            for v in crate::errors041::variants(name) {
                if let Some(id) = self.resolver.standard_constructor(name, v.name) {
                    let mut payload = Vec::new();
                    for t in v.payload {
                        payload.push(if crate::errors041::owner_id(t).is_some() {
                            Self::standard_type(t)
                        } else {
                            let meta = crate::parser::parse_type_annotation(t)
                                .map_err(|e| self.err(e.message))?;
                            self.annotation(&meta, &mut HashMap::new())?
                        });
                    }
                    self.result.bindings.insert(
                        id,
                        mono(function(
                            payload.iter().cloned(),
                            Type::Nominal(owner.clone()),
                        )),
                    );
                    self.result.constructors.insert(
                        id,
                        Constructor {
                            owner: owner.clone(),
                            id,
                            payload,
                        },
                    );
                    ids.push(id);
                }
            }
            if !ids.is_empty() {
                self.sums.insert(owner, ids);
            }
        }
        Ok(())
    }
    fn schema_id(&self, name: &str) -> Option<&NominalId> {
        self.schema_scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name))
    }
    fn schema_headers(&mut self, defs: &[Stmt]) -> R<()> {
        for stmt in defs {
            if let Stmt::Schema { name, start, .. } | Stmt::Sum { name, start, .. } = stmt {
                self.start = *start;
                if matches!(name.as_str(), "n" | "z" | "q" | "d" | "r" | "b" | "s" | "c") {
                    return Err(self.err(format!("{name} is reserved in the type namespace")));
                }
                let symbol = self
                    .resolver
                    .declaration(stmt)
                    .ok_or_else(|| self.err("missing nominal declaration identity"))?;
                let id = NominalId {
                    id: symbol.0,
                    name: name.clone(),
                };
                self.schema_scopes
                    .last_mut()
                    .expect("lexical schema scope")
                    .insert(name.clone(), id.clone());
                self.result.bindings.insert(symbol, mono(Type::Nominal(id)));
            }
        }
        for stmt in defs {
            match stmt {
                Stmt::Schema {
                    name,
                    fields,
                    start,
                } => {
                    self.start = *start;
                    let id = self
                        .schema_id(name)
                        .expect("schema header installed")
                        .clone();
                    self.install_schema(id, fields)?;
                }
                Stmt::Sum {
                    name,
                    variants,
                    start,
                } => {
                    self.start = *start;
                    let owner = self.schema_id(name).expect("sum header installed").clone();
                    let mut ids = Vec::new();
                    for variant in variants {
                        self.start = variant.start;
                        let id = self.resolver.variant(variant).ok_or_else(|| {
                            self.err(format!("missing constructor identity for {}", variant.name))
                        })?;
                        let mut payload = Vec::new();
                        for spelling in &variant.payload {
                            let mut vars = HashMap::new();
                            let t = self.annotation(spelling, &mut vars)?;
                            if !vars.is_empty() || !self.cx.free_vars(&t).is_empty() {
                                return Err(
                                    self.err("sum constructor payload requires a closed type")
                                );
                            }
                            payload.push(t);
                        }
                        let ty = function(payload.iter().cloned(), Type::Nominal(owner.clone()));
                        self.result.bindings.insert(id, mono(ty));
                        self.result.constructors.insert(
                            id,
                            Constructor {
                                owner: owner.clone(),
                                id,
                                payload,
                            },
                        );
                        ids.push(id);
                    }
                    self.sums.insert(owner, ids);
                }
                _ => {}
            }
        }
        Ok(())
    }
    fn constructor_type(&self, e: &Expr) -> Option<Type> {
        let c = self.result.constructors.get(&self.resolver.reference(e)?)?;
        Some(function(
            c.payload.iter().cloned(),
            Type::Nominal(c.owner.clone()),
        ))
    }
    fn at_boundary(&mut self, e: &Expr) -> R<Type> {
        let (id, err) = self
            .boundaries
            .last()
            .ok_or_else(|| self.err("no lexically enclosing fallible boundary"))?
            .clone();
        self.result.boundaries.insert(address(e), id);
        Ok(err)
    }
    fn recover(
        &mut self,
        env: &Env,
        value: &Expr,
        cases: &[RecoveryCase],
        want: Option<&Type>,
        effects: bool,
    ) -> R<Type> {
        let ok = want.cloned().unwrap_or_else(|| self.cx.fresh());
        let err = self.cx.fresh();
        let v = self.expr(env, value, Some(&result(ok.clone(), err.clone())), effects)?;
        if cases.is_empty() {
            return Err(self.err("recovery needs at least one case"));
        }
        let mut seen = HashSet::new();
        let mut catchall = false;
        for case in cases {
            let previous = self.start;
            self.start = case.start;
            if catchall {
                return Err(self.err("unreachable recovery case after catchall"));
            }
            if let Type::Nominal(owner) = self.cx.resolve(&err) {
                if self
                    .sums
                    .get(&owner)
                    .is_some_and(|ids| !ids.is_empty() && ids.iter().all(|id| seen.contains(id)))
                {
                    return Err(
                        self.err("unreachable recovery case; all constructors are already covered")
                    );
                }
            }
            let mut nested = env.clone();
            if let Some(id) = self.resolver.recovery_constructor(case) {
                let c = self
                    .result
                    .constructors
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| self.err("unknown recovery constructor"))?;
                if !seen.insert(id) {
                    return Err(self.err("duplicate recovery constructor case"));
                }
                self.unify(&err, &Type::Nominal(c.owner))?;
                let names = match &case.pattern {
                    RecoveryPattern::Constructor(_, xs) => xs.as_slice(),
                    _ => &[],
                };
                if names.len() != c.payload.len() {
                    return Err(self.err("wrong recovery constructor arity"));
                }
                let ids = self.resolver.recovery_bindings(case);
                for ((name, ty), id) in names
                    .iter()
                    .zip(c.payload)
                    .filter(|(n, _)| n.as_str() != "_")
                    .zip(ids)
                {
                    nested.insert(name.clone(), mono(ty.clone()));
                    self.result.bindings.insert(*id, mono(ty));
                }
            } else {
                catchall = true;
                match &case.pattern {
                    RecoveryPattern::Bind(name) => {
                        nested.insert(name.clone(), mono(err.clone()));
                        if let Some(id) = self.resolver.recovery_bindings(case).first() {
                            self.result.bindings.insert(*id, mono(err.clone()));
                        }
                    }
                    RecoveryPattern::Wildcard => {}
                    RecoveryPattern::Constructor(..) => {
                        return Err(self.err("unknown recovery constructor"))
                    }
                }
            }
            self.expr(&nested, &case.body, Some(&ok), effects)?;
            self.start = previous;
        }
        if !catchall {
            let Type::Nominal(owner) = self.cx.resolve(&err) else {
                return Err(self.err("nonexhaustive recovery; add a catchall"));
            };
            let variants = self
                .sums
                .get(&owner)
                .ok_or_else(|| self.err("nonexhaustive recovery; add a catchall"))?;
            let missed = variants
                .iter()
                .filter(|id| !seen.contains(id))
                .map(|id| self.resolver.symbol(*id).name.as_str())
                .collect::<Vec<_>>();
            if !missed.is_empty() {
                return Err(self.err(format!(
                    "nonexhaustive recovery; missing {}",
                    missed.join(", ")
                )));
            }
        }
        Ok(if v == Type::Never { Type::Never } else { ok })
    }
    fn install_schema(&mut self, id: NominalId, fields: &[(String, TypeAnnotation)]) -> R<()> {
        let mut row = Row {
            fields: vec![],
            tail: None,
        };
        for (name, t) in fields {
            let mut vars = HashMap::new();
            let x = self.annotation(t, &mut vars)?;
            if !vars.is_empty() {
                return Err(self.err(format!("nominal field {name} requires a closed type")));
            }
            if row.fields.iter().any(|(k, _)| k == name) {
                return Err(self.err(format!("duplicate field {name}")));
            }
            row.fields.push((name.clone(), x));
        }
        self.schemas.insert(id, row);
        Ok(())
    }
    fn int(&mut self, n: i128, want: Option<&Type>) -> R<Type> {
        match want.map(|w| self.cx.resolve(w)) {
            Some(Type::Prim(Prim::N)) if n < 0 => {
                Err(self.err("negative integer literal cannot have type n"))
            }
            Some(Type::Prim(t)) if num(t) => Ok(p(t)),
            _ => {
                let t = self.cx.fresh();
                self.req(Class::Num, &t)?;
                let end = self
                    .active
                    .as_ref()
                    .filter(|s| s.start == self.start)
                    .map_or(self.start, |s| s.end);
                self.literals.push((t.clone(), n, self.start, end));
                Ok(t)
            }
        }
    }
    fn default_literals(&mut self, from: usize) -> R<()> {
        let entries = self.literals[from..].to_vec();
        for (t, n, start, end) in entries {
            match self.cx.resolve(&t) {
                Type::Var(_) => self.unify(&t, &p(Prim::Z))?,
                Type::Prim(Prim::N) if n < 0 => {
                    return Err(InferError {
                        kind: InferErrorKind::TypeError,
                        start,
                        end,
                        expected: None,
                        actual: None,
                        message: "negative integer literal cannot have type n".into(),
                    })
                }
                _ => {}
            }
        }
        Ok(())
    }
    fn lookup(&mut self, env: &Env, n: &str) -> R<Type> {
        let sc = env
            .get(n)
            .ok_or_else(|| self.err(format!("unknown value {n}")))?;
        let (t, cs) = self.cx.instantiate(sc);
        for c in cs {
            self.req(c.class, &c.ty)?;
        }
        Ok(t)
    }
    fn reset_boundary<T>(&mut self, op: impl FnOnce(&mut Self) -> R<T>) -> R<T> {
        let saved = std::mem::take(&mut self.boundaries);
        let outcome = op(self);
        self.boundaries = saved;
        outcome
    }
    fn block_body(
        &mut self,
        env: &Env,
        defs: &[Stmt],
        tail: Option<&Expr>,
        want: Option<&Type>,
        effects: bool,
    ) -> R<Type> {
        let mut nested = env.clone();
        self.schema_scopes.push(HashMap::new());
        let checked = (|| {
            self.schema_headers(defs)?;
            self.local_defs(defs, &mut nested, effects)?;
            let terminal = defs.iter().any(|stmt| match stmt {
                Stmt::Def { params, body, .. } if params.is_empty() => {
                    self.result.type_of(body) == Some(&Type::Never)
                }
                Stmt::Effect { body, .. } => self.result.type_of(body) == Some(&Type::Never),
                _ => false,
            });
            let t = if let Some(ex) = tail {
                self.expr(&nested, ex, (!terminal).then_some(want).flatten(), effects)?
            } else {
                p(Prim::Unit)
            };
            Ok(if terminal { Type::Never } else { t })
        })();
        self.schema_scopes.pop();
        checked
    }
    fn expr(&mut self, env: &Env, e: &Expr, want: Option<&Type>, effects: bool) -> R<Type> {
        let previous_start = self.start;
        let previous_span = self.active.take();
        self.active = self.resolver.span(e);
        if let Some(start) = self.resolver.offset(e) {
            self.start = start;
        }
        let outcome = self.expr_inner(env, e, want, effects);
        self.start = previous_start;
        self.active = previous_span;
        outcome
    }
    fn expr_inner(&mut self, env: &Env, e: &Expr, want: Option<&Type>, effects: bool) -> R<Type> {
        use Prim::*;
        let t = match e {
            Expr::Int(n) if self.entry_natural_tail == Some(address(e)) => {
                self.int(*n, Some(&p(N)))?
            }
            Expr::Int(n) => self.int(*n, want)?,
            Expr::Num(raw) => {
                let k = match raw.as_bytes().last() {
                    Some(b'n') => N,
                    Some(b'z') => Z,
                    Some(b'q') => Q,
                    Some(b'd') => D,
                    Some(b'r') => R,
                    _ if raw.contains('.') => Q,
                    _ => Z,
                };
                if k == N && raw.starts_with('-') {
                    return Err(self.err("negative natural literal"));
                }
                if matches!(k, N | Z) && raw.contains('.') {
                    return Err(self.err("integral suffix on a fraction"));
                }
                p(k)
            }
            Expr::Str(_) => p(S),
            Expr::Char(_) => p(C),
            Expr::Bool(_) => p(B),
            Expr::Unit => p(Unit),
            Expr::Name(n) => match self.constructor_type(e) {
                Some(t) => t,
                None => self.lookup(env, n)?,
            },
            Expr::Ref(n) => {
                if let Some(t) = self.constructor_type(e) {
                    t
                } else if env.contains_key(n) {
                    self.lookup(env, n)?
                } else {
                    self.builtin_ref(n, want)?
                }
            }
            Expr::Tuple(es) => {
                let target = want.map(|t| self.cx.resolve(t));
                let xs = match &target {
                    Some(Type::Tuple(xs)) if xs.len() == es.len() => Some(xs),
                    _ => None,
                };
                let ts = es
                    .iter()
                    .enumerate()
                    .map(|(i, x)| self.expr(env, x, xs.map(|ts| &ts[i]), effects))
                    .collect::<crate::infer::R<Vec<_>>>()?;
                if ts.contains(&Type::Never) {
                    Type::Never
                } else {
                    Type::Tuple(ts)
                }
            }
            Expr::Array(es) => {
                let element = match want.map(|t| self.cx.resolve(t)) {
                    Some(Type::Seq(t)) => *t,
                    _ => self.cx.fresh(),
                };
                if es.is_empty() {
                    let end = self.active.as_ref().map_or(self.start, |span| span.end);
                    self.empty_sequences
                        .push((element.clone(), self.start, end));
                }
                let mut bottom = false;
                for e in es {
                    bottom |= self.expr(env, e, Some(&element), effects)? == Type::Never;
                }
                if bottom {
                    Type::Never
                } else {
                    s(element)
                }
            }
            Expr::Record(fs) | Expr::NamedRecord(_, fs) => {
                let nominal = if let Expr::NamedRecord(name, _) = e {
                    let id = self
                        .resolver
                        .reference(e)
                        .ok_or_else(|| self.err(format!("missing identity for schema {name}")))?;
                    Some(NominalId {
                        id: id.0,
                        name: name.clone(),
                    })
                } else {
                    None
                };
                let expected = match nominal.as_ref() {
                    Some(id) => Some(
                        self.schemas
                            .get(id)
                            .cloned()
                            .ok_or_else(|| self.err(format!("unknown schema {}", id.name)))?,
                    ),
                    _ => match want.map(|t| self.cx.resolve(t)) {
                        Some(Type::Record(r)) => Some(r),
                        _ => None,
                    },
                };
                let mut fields = Vec::new();
                for (n, ex) in fs {
                    if fields.iter().any(|(k, _)| k == n) {
                        return Err(self.err(format!("duplicate field {n}")));
                    }
                    let typ = expected
                        .as_ref()
                        .and_then(|r| r.fields.iter().find(|(name, _)| name == n).map(|(_, t)| t));
                    fields.push((n.clone(), self.expr(env, ex, typ, effects)?));
                }
                if fields.iter().any(|(_, t)| t == &Type::Never) {
                    Type::Never
                } else if let Some(id) = nominal {
                    let typ = Type::Record(expected.expect("nominal schema"));
                    self.unify(&Type::Record(Row { fields, tail: None }), &typ)?;
                    Type::Nominal(id)
                } else {
                    Type::Record(Row { fields, tail: None })
                }
            }
            Expr::Cases(cases, alt) => {
                let t = want.cloned().unwrap_or_else(|| self.cx.fresh());
                let mut normal = false;
                let mut reachable = true;
                for (cond, body) in cases {
                    let c = self.expr(env, cond, Some(&p(B)), effects)?;
                    if c == Type::Never {
                        reachable = false;
                    }
                    let ty = self.expr(env, body, Some(&t), effects)?;
                    if reachable && ty != Type::Never {
                        normal = true;
                    }
                }
                let other = self.expr(env, alt, Some(&t), effects)?;
                if normal || (reachable && other != Type::Never) {
                    t
                } else {
                    Type::Never
                }
            }
            Expr::Lambda(ps, body) | Expr::TypedLambda(ps, _, body) => {
                let anns = match e {
                    Expr::TypedLambda(_, ts, _) => ts.as_slice(),
                    _ => &[],
                };
                let sig = self.signature(ps, anns, None)?;
                let t = function(sig.args.iter().cloned(), sig.out.clone());
                if let Some(w) = want {
                    self.unify(&t, w)?;
                }
                let mut nested = env.clone();
                for ((name, t), id) in ps
                    .iter()
                    .zip(sig.args.iter())
                    .zip(self.resolver.lambda_parameters(e))
                {
                    nested.insert(name.clone(), mono(t.clone()));
                    self.result.bindings.insert(*id, mono(t.clone()));
                }
                // Parameter IDs are metadata, not a prerequisite for lexical checking.
                for (name, t) in ps.iter().zip(sig.args) {
                    nested.insert(name.clone(), mono(t));
                }
                self.reset_boundary(|this| this.expr(&nested, body, Some(&sig.out), false))?;
                t
            }
            Expr::Block(defs, tail) => self.block_body(env, defs, tail.as_deref(), want, false)?,
            Expr::Action(defs, tail) => {
                let expected = match want.map(|x| self.cx.resolve(x)) {
                    Some(Type::Action(t)) => Some(*t),
                    _ => None,
                };
                a(self.reset_boundary(|this| {
                    this.block_body(env, defs, tail.as_deref(), expected.as_ref(), true)
                })?)
            }
            Expr::Fallible(defs, tail) => {
                self.saw_result = true;
                let (ok, err) = match want.map(|x| self.cx.resolve(x)) {
                    Some(Type::Result(ok, err)) => (*ok, *err),
                    _ => (self.cx.fresh(), self.cx.fresh()),
                };
                let id = address(e);
                self.result.boundaries.insert(id, id);
                self.boundaries.push((id, err.clone()));
                let output = self.block_body(env, defs, tail.as_deref(), Some(&ok), effects);
                self.boundaries.pop();
                let output = output?;
                if output != Type::Never {
                    self.unify(&ok, &output)?;
                } else if self.entry_boundary == Some(id)
                    && matches!(self.cx.resolve(&ok), Type::Var(_))
                {
                    // A top-level boundary with no successful control-flow
                    // path has a trivially valid unit process success type.
                    self.unify(&ok, &p(Unit))?;
                }
                result(ok, err)
            }
            Expr::Raise(value) => {
                let err = self.at_boundary(e)?;
                let old = self.raise_payload.replace(address(value));
                let typed = self.expr(env, value, Some(&err), effects);
                self.raise_payload = old;
                typed?;
                Type::Never
            }
            Expr::Propagate(value) => {
                let err = self.at_boundary(e)?;
                let ok = want.cloned().unwrap_or_else(|| self.cx.fresh());
                if self.expr(env, value, Some(&result(ok.clone(), err)), effects)? == Type::Never {
                    Type::Never
                } else {
                    ok
                }
            }
            Expr::Recover(value, cases) => {
                self.saw_result = true;
                self.recover(env, value, cases, want, effects)?
            }
            Expr::Pipeline(stages) => {
                let Some((head, tail)) = stages.split_first() else {
                    return Err(self.err("empty pipeline"));
                };
                let mut t = self.expr(
                    env,
                    head,
                    if tail.is_empty() { want } else { None },
                    effects,
                )?;
                let mut nested = env.clone();
                let mut prev = head;
                let mut terminal = t == Type::Never;
                for (i, ex) in tail.iter().enumerate() {
                    let possibly_result =
                        matches!(self.cx.resolve(&t), Type::Result(..) | Type::Var(_));
                    let used = !possibly_result
                        || self.resolver.placeholder(ex).is_some_and(|id| {
                            let mut found = HashSet::new();
                            scan_dependencies(ex, self.resolver, &HashSet::from([id]), &mut found);
                            found.contains(&id)
                        });
                    if !used {
                        let at = self.resolver.offset(prev).unwrap_or(self.start);
                        let span = self.resolver.span(prev).unwrap_or(at..at);
                        self.discarded_results
                            .push((t.clone(), span.start, span.end));
                    }
                    nested.insert("$".into(), mono(t.clone()));
                    t = self.expr(
                        &nested,
                        ex,
                        if !terminal && i + 1 == tail.len() {
                            want
                        } else {
                            None
                        },
                        effects,
                    )?;
                    terminal |= t == Type::Never;
                    prev = ex;
                }
                if terminal {
                    Type::Never
                } else {
                    t
                }
            }
            Expr::Call(n, args) => {
                if let Some(callee) = self.constructor_type(e) {
                    self.result.callees.insert(address(e), callee.clone());
                    self.apply(env, callee, args, want, effects)?
                } else if env.contains_key(n) {
                    let callee = self.lookup(env, n)?;
                    self.result.callees.insert(address(e), callee.clone());
                    self.apply(env, callee, args, want, effects)?
                } else {
                    self.builtin_call(env, e, n, args, want, effects)?
                }
            }
        };
        if let Some(w) = want {
            if t != Type::Never {
                if self.raise_payload == Some(address(e)) {
                    let target = self.cx.resolve(w);
                    let actual = self.cx.resolve(&t);
                    if target != actual
                        && self.cx.free_vars(&target).is_empty()
                        && self.cx.free_vars(&actual).is_empty()
                    {
                        return Err(self.incompatible(&actual, &target));
                    }
                }
                self.unify(&t, w)?;
            }
        }
        self.result.expressions.insert(address(e), t.clone());
        Ok(t)
    }
    fn apply(
        &mut self,
        env: &Env,
        mut callee: Type,
        args: &[Expr],
        want: Option<&Type>,
        effects: bool,
    ) -> R<Type> {
        let mut bottom = false;
        // Check strict operands before the final contextual return type when
        // an argument contains an immediate lexical escape.
        let delay = !self.boundaries.is_empty() && args.iter().any(immediate_raise);
        for (i, ex) in args.iter().enumerate() {
            let arg = self.cx.fresh();
            let out = if !delay && i + 1 == args.len() {
                want.cloned().unwrap_or_else(|| self.cx.fresh())
            } else {
                self.cx.fresh()
            };
            self.unify(&callee, &f(arg.clone(), out.clone()))?;
            bottom |= self.expr(env, ex, Some(&arg), effects)? == Type::Never;
            callee = out;
        }
        Ok(if bottom { Type::Never } else { callee })
    }
    fn select(&mut self, sel: &str, src: &Type) -> R<Type> {
        let src = self.cx.resolve(src);
        match src {
            Type::Seq(t) | Type::Table(t) => Ok(s(self.select(sel, &t)?)),
            Type::Tuple(ts) => {
                if let Ok(i) = sel.parse::<usize>() {
                    if let Some(t) = i.checked_sub(1).and_then(|i| ts.get(i)) {
                        return Ok(t.clone());
                    }
                }
                Err(self.err(format!("tuple has no position .{sel}")))
            }
            Type::Nominal(n) => self
                .schemas
                .get(&n)
                .and_then(|r| r.fields.iter().find(|(k, _)| k == sel))
                .map(|(_, t)| t.clone())
                .ok_or_else(|| self.err(format!("schema {} has no field {sel}", n.name))),
            Type::Record(r) => {
                if let Some((_, t)) = r.fields.iter().find(|(n, _)| n == sel) {
                    return Ok(t.clone());
                }
                if r.tail.is_none() {
                    return Err(self.err(format!("record has no field {sel}")));
                }
                let out = self.cx.fresh();
                let row = Type::Record(Row {
                    fields: vec![(sel.into(), out.clone())],
                    tail: Some(self.cx.fresh_row()),
                });
                self.unify(&Type::Record(r), &row)?;
                Ok(out)
            }
            Type::Var(_) => {
                if sel.as_bytes().first().is_some_and(u8::is_ascii_digit) {
                    return Err(self.unsupported("tuple projection needs a concrete tuple arity"));
                }
                let out = self.cx.fresh();
                let row = Type::Record(Row {
                    fields: vec![(sel.into(), out.clone())],
                    tail: Some(self.cx.fresh_row()),
                });
                self.unify(&src, &row)?;
                Ok(out)
            }
            _ => Err(self.err(format!("cannot select .{sel} from {src}"))),
        }
    }
    fn require_exact_numeric(&mut self, t: &Type, n: &str) -> R<()> {
        let t = self.cx.resolve(t);
        match t {
            Type::Prim(p) if exact(p) => Ok(()),
            Type::Var(_)
                if self
                    .literals
                    .iter()
                    .rev()
                    .any(|(lit, _, _, _)| self.cx.resolve(lit) == t) =>
            {
                self.unify(&t, &p(Prim::Z))
            }
            Type::Var(_) => {
                Err(self.err(format!("ambiguous {n}: supply an exact numeric annotation")))
            }
            _ => Err(self.err(format!("{n} expects an exact numeric type, found {t}"))),
        }
    }
    fn numeric_binary(&mut self, n: &str, ta: &Type, tb: &Type, out: Option<&Type>) -> R<Type> {
        use Prim::*;
        let ta = self.cx.resolve(ta);
        let tb = self.cx.resolve(tb);
        if [&ta, &tb]
            .iter()
            .any(|t| matches!(t, Type::Seq(el) if matches!(el.as_ref(), Type::Seq(_))))
        {
            return Err(self.err("numeric lifting is limited to one sequence dimension"));
        }
        let out = out.map(|t| self.cx.resolve(t));
        if let Type::Seq(a) = &ta {
            let se = match &out {
                Some(Type::Seq(t)) => Some(t.as_ref()),
                _ => None,
            };
            let b = match &tb {
                Type::Seq(b) => b.as_ref(),
                _ => &tb,
            };
            return Ok(s(self.numeric_binary(n, a, b, se)?));
        }
        if let Type::Seq(b) = &tb {
            let se = match &out {
                Some(Type::Seq(t)) => Some(t.as_ref()),
                _ => None,
            };
            return Ok(s(self.numeric_binary(n, &ta, b, se)?));
        }
        if n == "^" {
            self.unify(&tb, &p(N))?;
            self.req(Class::Num, &ta)?;
            return Ok(ta);
        }
        self.req(Class::Num, &ta)?;
        self.req(Class::Num, &tb)?;
        let k = match (&ta, &tb) {
            (Type::Prim(a), Type::Prim(b)) => p(solve_numeric(*a, *b)
                .ok_or_else(|| self.err("r does not implicitly mix with exact numerical types"))?),
            (Type::Var(_), Type::Prim(_)) => {
                self.unify(&ta, &tb)?;
                tb.clone()
            }
            (Type::Prim(_), Type::Var(_)) => {
                self.unify(&tb, &ta)?;
                ta.clone()
            }
            (Type::Var(_), Type::Var(_)) => {
                self.unify(&ta, &tb)?;
                if let Some(t @ Type::Prim(_)) = out.as_ref() {
                    if n != "/" || t == &p(R) {
                        self.unify(&ta, t)?;
                    }
                }
                ta.clone()
            }
            _ => return Err(self.err("expected numerical operands")),
        };
        let k = self.cx.resolve(&k);
        if matches!(n, "//" | "%" | "..") {
            match k {
                Type::Prim(N | Z) => {}
                Type::Var(_) => self.unify(&k, &p(Z))?,
                _ => return Err(self.err(format!("{n} requires integral operands"))),
            }
        }
        Ok(if n == "/" {
            if k == p(R) {
                p(R)
            } else {
                self.require_exact_numeric(&k, "division")?;
                p(Q)
            }
        } else {
            k
        })
    }
    fn binary_rel(&mut self, n: &str, a: &Type, b: &Type) -> R<Type> {
        let at = self.cx.resolve(a);
        let bt = self.cx.resolve(b);
        if n != "==" {
            if [&at, &bt]
                .iter()
                .any(|t| matches!(t, Type::Seq(el) if matches!(el.as_ref(), Type::Seq(_))))
            {
                return Err(self.err("elementwise comparison is limited to one sequence dimension; use == for nested structural equality"));
            }
            match (&at, &bt) {
                (Type::Seq(a), Type::Seq(b)) => return Ok(s(self.binary_rel(n, a, b)?)),
                (Type::Seq(a), b) => return Ok(s(self.binary_rel(n, a, b)?)),
                (a, Type::Seq(b)) => return Ok(s(self.binary_rel(n, a, b)?)),
                _ => {}
            }
        }
        let k = if let (Type::Prim(a), Type::Prim(b)) = (&at, &bt) {
            if num(*a) && num(*b) {
                p(solve_numeric(*a, *b).ok_or_else(|| {
                    self.err("r does not implicitly mix with exact numerical types")
                })?)
            } else {
                self.unify(&at, &bt)?;
                at
            }
        } else {
            self.unify(&at, &bt)?;
            at
        };
        self.req(
            if matches!(n, "<" | "<=" | ">" | ">=") {
                Class::Ord
            } else {
                Class::Eq
            },
            &k,
        )?;
        Ok(p(Prim::B))
    }
    fn unary_fn(
        &mut self,
        env: &Env,
        ex: &Expr,
        arg: &Type,
        want: Option<&Type>,
        effect: bool,
    ) -> R<Type> {
        let out = want.cloned().unwrap_or_else(|| self.cx.fresh());
        self.expr(env, ex, Some(&f(arg.clone(), out.clone())), effect)?;
        Ok(out)
    }
    fn collection(&mut self, t: &Type) -> R<Type> {
        match self.cx.resolve(t) {
            Type::Seq(x) | Type::Table(x) => Ok(*x),
            t @ Type::Var(_) => {
                let out = self.cx.fresh();
                self.unify(&t, &s(out.clone()))?;
                Ok(out)
            }
            t => Err(self.err(format!("expected a sequence or table, found {t}"))),
        }
    }
    fn seq_element(&mut self, t: &Type) -> R<Type> {
        let out = self.cx.fresh();
        self.unify(t, &s(out.clone()))?;
        Ok(out)
    }
    fn row(&self, t: &Type) -> R<Row> {
        match self.cx.resolve(t) {
            Type::Record(r) => Ok(r),
            Type::Nominal(n) => self
                .schemas
                .get(&n)
                .cloned()
                .ok_or_else(|| self.err(format!("unknown schema {}", n.name))),
            x => Err(self.err(format!("expected a record, found {x}"))),
        }
    }
    fn builtin_ref(&mut self, n: &str, want: Option<&Type>) -> R<Type> {
        if fallible_builtin(n) {
            self.saw_result = true;
        }
        if n.starts_with('.') && n != ".:" && n != ".." {
            let src = self.cx.fresh();
            let out = self.cx.fresh();
            let t = f(src.clone(), out.clone());
            if let Some(w) = want {
                self.unify(&t, w)?;
            }
            let actual = self.select(&n[1..], &src)?;
            self.unify(&out, &actual)?;
            return Ok(t);
        }
        let Some(spec) = crate::builtins::lookup(n) else {
            return Err(self.unsupported(format!("unknown builtin reference &{n}")));
        };
        if spec.op.is_none() || spec.availability != crate::builtins::Availability::Runtime {
            return Err(self.unsupported(format!(
                "{} is recognized but not implemented",
                spec.canonical
            )));
        }
        if spec.effect == crate::builtins::Effect::ExecuteAction
            || matches!(spec.canonical, "??" | "@" | "@@")
        {
            return Err(self.err(format!(
                "{} is a special form, not a function reference",
                spec.canonical
            )));
        }
        let args = (0..spec.arity).map(|_| self.cx.fresh()).collect::<Vec<_>>();
        let out = self.cx.fresh();
        let fun = function(args.iter().cloned(), out.clone());
        if let Some(w) = want {
            self.unify(&fun, w)?;
        }
        let typed_args = args.iter().map(|t| self.cx.resolve(t)).collect::<Vec<_>>();
        let ret = self.builtin_types(spec.canonical, &typed_args, Some(&out))?;
        self.unify(&ret, &out)?;
        Ok(fun)
    }
    fn known_callee(&self, env: &Env, e: &Expr) -> bool {
        match e {
            Expr::Name(name) | Expr::Ref(name) => {
                env.contains_key(name) || self.constructor_type(e).is_some()
            }
            _ => false,
        }
    }
    fn bundle_arguments(
        &mut self,
        env: &Env,
        callee: &Expr,
        bundle: &Expr,
        out: Type,
        effect: bool,
    ) -> R<Type> {
        let tuple = self.expr(env, bundle, None, effect)?;
        if tuple == Type::Never {
            self.expr(env, callee, None, effect)?;
            return Ok(Type::Never);
        }
        let Type::Tuple(xs) = self.cx.resolve(&tuple) else {
            return Err(self.err("@@ requires a tuple bundle"));
        };
        let expected = function(xs.into_iter(), out.clone());
        let callee = self.expr(env, callee, Some(&expected), effect)?;
        Ok(if callee == Type::Never {
            Type::Never
        } else {
            out
        })
    }
    fn builtin_call(
        &mut self,
        env: &Env,
        call: &Expr,
        n: &str,
        args: &[Expr],
        want: Option<&Type>,
        effect: bool,
    ) -> R<Type> {
        use Prim::*;
        let spec = crate::builtins::lookup(n);
        if let Some(spec) = spec {
            if spec.arity != args.len() {
                return Err(self.err(format!(
                    "{} expects {} operands",
                    spec.canonical, spec.arity
                )));
            }
            if spec.availability == crate::builtins::Availability::Recognized || spec.op.is_none() {
                return Err(self.unsupported(format!(
                    "{} is recognized but not implemented",
                    spec.canonical
                )));
            }
        }
        let n = spec.map_or(n, |s| s.canonical);
        if fallible_builtin(n) {
            self.saw_result = true;
        }
        let result = match n {
            "@" => {
                let arg = self.cx.fresh();
                let out = want.cloned().unwrap_or_else(|| self.cx.fresh());
                let expected = f(arg.clone(), out.clone());
                // Known function signatures provide argument context, for
                // example when a checked CSV action needs its row annotation.
                // Overloaded operator/selector refs instead need the operand.
                let known = self.known_callee(env, &args[0]);
                let (callee, input) = if known {
                    (
                        self.expr(env, &args[0], Some(&expected), effect)?,
                        self.expr(env, &args[1], Some(&arg), effect)?,
                    )
                } else {
                    let operand = self.expr(env, &args[1], Some(&arg), effect)?;
                    let callee = self.expr(env, &args[0], Some(&expected), effect)?;
                    (callee, operand)
                };
                if callee == Type::Never || input == Type::Never {
                    Type::Never
                } else {
                    out
                }
            }
            "@@" => {
                let out = want.cloned().unwrap_or_else(|| self.cx.fresh());
                if let Expr::Tuple(xs) = &args[1] {
                    if self.known_callee(env, &args[0]) {
                        let params = (0..xs.len()).map(|_| self.cx.fresh()).collect::<Vec<_>>();
                        let expected = function(params.iter().cloned(), out.clone());
                        let callee = self.expr(env, &args[0], Some(&expected), effect)?;
                        let args = self.expr(env, &args[1], Some(&Type::Tuple(params)), effect)?;
                        if callee == Type::Never || args == Type::Never {
                            Type::Never
                        } else {
                            out
                        }
                    } else {
                        self.bundle_arguments(env, &args[0], &args[1], out, effect)?
                    }
                } else {
                    self.bundle_arguments(env, &args[0], &args[1], out, effect)?
                }
            }
            "@:" | "?:" | "=:" | ">:" | "<:" => {
                let src = self.expr(env, &args[1], None, effect)?;
                let res = self.cx.resolve(&src);
                let elt = match &res {
                    Type::Dict(_, t) if n == "@:" => t.as_ref().clone(),
                    _ => self.collection(&src)?,
                };
                match (&res, n) {
                    (Type::Dict(k, _), "@:") => {
                        let result = self.unary_fn(env, &args[0], &elt, None, effect)?;
                        Type::Dict(k.clone(), b(result))
                    }
                    (_, "@:") => {
                        let result = self.unary_fn(env, &args[0], &elt, None, effect)?;
                        s(result)
                    }
                    (_, "?:") => {
                        let predicate = &args[0];
                        if matches!(
                            predicate,
                            Expr::Lambda(..) | Expr::TypedLambda(..) | Expr::Ref(_)
                        ) {
                            self.unary_fn(env, predicate, &elt, Some(&p(B)), effect)?;
                        } else {
                            let pred = self.expr(env, predicate, None, effect)?;
                            match self.cx.resolve(&pred) {
                                Type::Never => return Ok(Type::Never),
                                Type::Seq(_) => self.unify(&pred, &s(p(B)))?,
                                _ => self.unify(&pred, &f(elt.clone(), p(B)))?,
                            }
                        }
                        src
                    }
                    (_, "=:") => {
                        let k = self.unary_fn(env, &args[0], &elt, None, effect)?;
                        self.req(Class::Hash, &k)?;
                        Type::Dict(b(k), b(src))
                    }
                    _ => {
                        let k = self.unary_fn(env, &args[0], &elt, None, effect)?;
                        self.req(Class::Ord, &k)?;
                        src
                    }
                }
            }
            "df.agg" => {
                let src = self.expr(env, &args[2], None, effect)?;
                let Type::Table(row) = self.cx.resolve(&src) else {
                    return Err(self.err("df.agg expects a table"));
                };
                let key = self.unary_fn(env, &args[0], &row, None, effect)?;
                self.req(Class::Hash, &key)?;
                let summary = match want.map(|x| self.cx.resolve(x)) {
                    Some(Type::Table(row)) => *row,
                    _ => self.cx.fresh(),
                };
                self.expr(
                    env,
                    &args[1],
                    Some(&f(key, f(src, summary.clone()))),
                    effect,
                )?;
                self.row(&summary)?;
                Type::Table(b(summary))
            }
            "df.extend" => {
                let src = self.expr(env, &args[1], None, effect)?;
                let Type::Table(row) = self.cx.resolve(&src) else {
                    return Err(self.err("df.extend expects a table"));
                };
                let mut out = self.row(&row)?;
                let columns = self.expr(env, &args[0], None, effect)?;
                let columns = self.row(&columns)?;
                if columns.fields.is_empty() {
                    return Err(self.err("df.extend requires at least one new column"));
                }
                for (n, t) in columns.fields {
                    if out.fields.iter().any(|(k, _)| k == &n) {
                        return Err(
                            self.err(format!("df.extend cannot replace existing column {n}"))
                        );
                    }
                    out.fields.push((n, self.seq_element(&t)?));
                }
                Type::Table(b(Type::Record(out)))
            }
            "#[" => {
                let Expr::Array(rows) = &args[0] else {
                    return Err(self.err("expected table rows"));
                };
                let row = match want.map(|x| self.cx.resolve(x)) {
                    Some(Type::Table(row)) => *row,
                    _ => self.cx.fresh(),
                };
                for ex in rows {
                    self.expr(env, ex, Some(&row), effect)?;
                }
                self.result
                    .expressions
                    .insert(address(&args[0]), s(row.clone()));
                match self.cx.resolve(&row) {
                    Type::Record(_) | Type::Nominal(_) | Type::Tuple(_) => {}
                    _ => return Err(self.err("table rows need a tuple or record schema")),
                }
                Type::Table(b(row))
            }
            "#:" => {
                let target = match want.map(|x| self.cx.resolve(x)) {
                    Some(Type::Table(row)) => Some(*row),
                    _ => None,
                };
                let struct_want = match target.as_ref() {
                    Some(Type::Record(r)) if matches!(&args[0], Expr::Record(_)) => {
                        Some(Type::Record(Row {
                            fields: r
                                .fields
                                .iter()
                                .map(|(n, t)| (n.clone(), s(t.clone())))
                                .collect(),
                            tail: r.tail,
                        }))
                    }
                    _ => None,
                };
                let src = self.expr(env, &args[0], struct_want.as_ref(), effect)?;
                let r = match self.cx.resolve(&src) {
                    Type::Seq(t) => match self.cx.resolve(&t) {
                        Type::Record(_) | Type::Nominal(_) | Type::Tuple(_) => *t,
                        _ => return Err(self.err("#: expects tuple or record rows")),
                    },
                    Type::Nominal(_) | Type::Record(_) => {
                        let cols = self.row(&src)?;
                        if cols.fields.is_empty() {
                            return Err(self.err("#: requires at least one column"));
                        }
                        let mut fields = Vec::new();
                        for (name, t) in cols.fields {
                            fields.push((name, self.seq_element(&t)?));
                        }
                        Type::Record(Row { fields, tail: None })
                    }
                    _ => return Err(self.err("#: expects row sequence or record of columns")),
                };
                Type::Table(b(r))
            }
            "!" => {
                if !effect {
                    return Err(self.err("cannot execute an action in a pure context"));
                }
                let out = want.cloned().unwrap_or_else(|| self.cx.fresh());
                if self.expr(env, &args[0], Some(&a(out.clone())), effect)? == Type::Never {
                    Type::Never
                } else {
                    out
                }
            }
            "csv.read" => {
                let path = self.expr(env, &args[0], Some(&p(S)), effect)?;
                if path == Type::Never {
                    Type::Never
                } else {
                    self.csv_read_type(want)?
                }
            }
            "csv.write" => {
                let path = self.expr(env, &args[0], Some(&p(S)), effect)?;
                let tt = self.expr(env, &args[1], None, effect)?;
                if path == Type::Never || tt == Type::Never {
                    Type::Never
                } else {
                    let Type::Table(row) = self.cx.resolve(&tt) else {
                        return Err(self.err("csv.write expects a named or structural table"));
                    };
                    self.csv_row(&row)?;
                    a(result(p(Unit), Self::standard_type("csverr")))
                }
            }
            "?0" => {
                let x = self.cx.fresh();
                o(x)
            }
            "?1" => {
                let x = match want.map(|t| self.cx.resolve(t)) {
                    Some(Type::Optional(x)) => *x,
                    _ => self.cx.fresh(),
                };
                self.expr(env, &args[0], Some(&x), effect)?;
                o(x)
            }
            "??" => {
                let t = want.cloned().unwrap_or_else(|| self.cx.fresh());
                self.expr(env, &args[1], Some(&o(t.clone())), effect)?;
                self.expr(env, &args[0], Some(&t), effect)?;
                t
            }
            ".:" => {
                let t = self.expr(env, &args[1], None, effect)?;
                match self.cx.resolve(&t) {
                    Type::Dict(k, v) => {
                        self.expr(env, &args[0], Some(&k), effect)?;
                        o(*v)
                    }
                    Type::Seq(t) | Type::Table(t) => {
                        self.expr(env, &args[0], Some(&p(N)), effect)?;
                        o(*t)
                    }
                    x => return Err(self.err(format!("cannot index {x}"))),
                }
            }
            "/:" | "\\:" => {
                let acc_expect = match (n, want.map(|w| self.cx.resolve(w))) {
                    ("/:", t) => t,
                    (_, Some(Type::Seq(t))) => Some(*t),
                    _ => None,
                };
                let acc = self.expr(env, &args[1], acc_expect.as_ref(), effect)?;
                let seq = self.expr(env, &args[2], None, effect)?;
                let item = self.seq_element(&seq)?;
                self.expr(
                    env,
                    &args[0],
                    Some(&f(acc.clone(), f(item, acc.clone()))),
                    effect,
                )?;
                if n == "/:" {
                    acc
                } else {
                    s(acc)
                }
            }
            _ => {
                let xs = args
                    .iter()
                    .enumerate()
                    .map(|(i, x)| {
                        let context = match (n, i) {
                            ("^", 1) | ("take" | "[:" | "drop", 0) | ("slice", 0 | 1) => Some(p(N)),
                            ("take" | "drop" | "[:", 1)
                            | ("slice", 2)
                            | ("rev" | "=." | "tr", 0)
                            | ("+:", 0 | 1) => want.map(|w| self.cx.resolve(w)).filter(|t| {
                                matches!(t, Type::Seq(_) | Type::Table(_) | Type::Prim(Prim::S))
                            }),
                            ("flat", 0) => match want.map(|w| self.cx.resolve(w)) {
                                Some(t @ Type::Seq(_)) => Some(s(t)),
                                _ => None,
                            },
                            ("&:" | "*:", i @ (0 | 1)) => match want.map(|w| self.cx.resolve(w)) {
                                Some(Type::Seq(pair)) => match *pair {
                                    Type::Tuple(ts) if ts.len() == 2 => Some(s(ts[i].clone())),
                                    _ => None,
                                },
                                Some(Type::Table(pair)) if n == "*:" => match *pair {
                                    Type::Tuple(ts) if ts.len() == 2 => {
                                        Some(Type::Table(b(ts[i].clone())))
                                    }
                                    _ => None,
                                },
                                _ => None,
                            },
                            _ => None,
                        };
                        self.expr(env, x, context.as_ref(), effect)
                    })
                    .collect::<crate::infer::R<Vec<_>>>()?;
                if xs.contains(&Type::Never) {
                    Type::Never
                } else {
                    self.builtin_types(n, &xs, want)?
                }
            }
        };
        self.result.operations.insert(
            address(call),
            if matches!(n, "+" | "-" | "*" | "/" | "//" | "%" | "^" | "-:" | "|") {
                Operation::Numeric {
                    operator: n.to_owned(),
                    result: result.clone(),
                }
            } else {
                Operation::Builtin(n.to_owned())
            },
        );
        Ok(result)
    }
    fn csv_read_type(&mut self, want: Option<&Type>) -> R<Type> {
        let ok = match want.map(|t| self.cx.resolve(t)) {
            Some(Type::Action(t)) => match *t {
                Type::Result(t, _) => match *t {
                    Type::Table(row) => {
                        self.csv_row(&row)?;
                        Type::Table(row)
                    }
                    _ => s(s(p(Prim::S))),
                },
                _ => s(s(p(Prim::S))),
            },
            _ => s(s(p(Prim::S))),
        };
        Ok(a(result(ok, Self::standard_type("csverr"))))
    }
    fn csv_row(&mut self, t: &Type) -> R<()> {
        let r = self.row(t)?;
        if r.fields.is_empty() || r.tail.is_some() {
            return Err(self.err("CSV requires a closed nonempty row schema"));
        }
        for (name, t) in r.fields {
            let t = self.cx.resolve(&t);
            let t = if let Type::Optional(t) = t { *t } else { t };
            if !matches!(
                t,
                Type::Prim(Prim::N | Prim::Z | Prim::Q | Prim::R | Prim::B | Prim::S | Prim::C)
            ) {
                return Err(self.err(format!("unsupported CSV column type for {name}: {t}")));
            }
        }
        Ok(())
    }
    fn builtin_types(&mut self, n: &str, xs: &[Type], want: Option<&Type>) -> R<Type> {
        use Prim::*;
        let arity = crate::builtins::lookup(n)
            .map(|x| x.arity)
            .or_else(|| n.starts_with('.').then_some(1));
        if let Some(arity) = arity {
            if xs.len() != arity {
                return Err(self.err(format!("{n} expects {arity} operands")));
            }
        }
        Ok(match n {
            "@" => {
                let out = want.cloned().unwrap_or_else(|| self.cx.fresh());
                self.unify(&xs[0], &f(xs[1].clone(), out.clone()))?;
                out
            }
            "@@" => {
                let Type::Tuple(args) = self.cx.resolve(&xs[1]) else {
                    return Err(
                        self.err("tuple application requires a statically known tuple bundle")
                    );
                };
                let out = want.cloned().unwrap_or_else(|| self.cx.fresh());
                self.unify(&xs[0], &function(args.into_iter(), out.clone()))?;
                out
            }
            "/:" | "\\:" => {
                let item = self.seq_element(&xs[2])?;
                self.unify(&xs[0], &f(xs[1].clone(), f(item, xs[1].clone())))?;
                if n == "/:" {
                    xs[1].clone()
                } else {
                    s(xs[1].clone())
                }
            }
            ".:" => match self.cx.resolve(&xs[1]) {
                Type::Seq(t) | Type::Table(t) => {
                    self.unify(&xs[0], &p(N))?;
                    o(*t)
                }
                Type::Dict(k, v) => {
                    self.unify(&xs[0], &k)?;
                    o(*v)
                }
                other => {
                    return Err(self.err(format!(
                        "lookup needs a known sequence, table or dictionary, found {other}"
                    )))
                }
            },
            "@:" | "?:" | "=:" | ">:" | "<:" => {
                let src = self.cx.resolve(&xs[1]);
                let item = match &src {
                    Type::Seq(x) | Type::Table(x) => x.as_ref().clone(),
                    Type::Dict(_, v) if n == "@:" => v.as_ref().clone(),
                    _ => return Err(self.err(format!("{n} reference needs a known iterable type"))),
                };
                let key = self.cx.fresh();
                if n == "?:" {
                    if !matches!(self.cx.resolve(&xs[0]), Type::Seq(_)) {
                        self.unify(&xs[0], &f(item, p(B)))?;
                    } else {
                        self.unify(&xs[0], &s(p(B)))?;
                    }
                    src
                } else {
                    self.unify(&xs[0], &f(item, key.clone()))?;
                    match n {
                        "@:" => match src {
                            Type::Dict(k, _) => Type::Dict(k, b(key)),
                            _ => s(key),
                        },
                        "=:" => {
                            self.req(Class::Hash, &key)?;
                            Type::Dict(b(key), b(src))
                        }
                        _ => {
                            self.req(Class::Ord, &key)?;
                            src
                        }
                    }
                }
            }
            "?1" => o(xs[0].clone()),
            "??" => {
                self.unify(&xs[1], &o(xs[0].clone()))?;
                xs[0].clone()
            }
            "+" | "-" | "*" | "/" | "//" | "%" | "^" => {
                self.numeric_binary(n, &xs[0], &xs[1], want)?
            }
            ".." => {
                if xs
                    .iter()
                    .any(|t| matches!(self.cx.resolve(t), Type::Seq(_)))
                {
                    return Err(self.err("range bounds must be scalar exact integers"));
                }
                let w = match want.map(|t| self.cx.resolve(t)) {
                    Some(Type::Seq(t)) => Some(*t),
                    _ => None,
                };
                s(self.numeric_binary(n, &xs[0], &xs[1], w.as_ref())?)
            }
            "<" | "<=" | ">" | ">=" | "=" | "/=" | "==" => self.binary_rel(n, &xs[0], &xs[1])?,
            "~" => {
                self.unify(&xs[0], &p(B))?;
                p(B)
            }
            "&&" | "||" => {
                fn bool_shape(t: &Type) -> Type {
                    if let Type::Seq(t) = t {
                        s(bool_shape(t))
                    } else {
                        p(Prim::B)
                    }
                }
                let a = self.cx.resolve(&xs[0]);
                let b = self.cx.resolve(&xs[1]);
                if [&a, &b]
                    .iter()
                    .any(|t| matches!(t, Type::Seq(el) if matches!(el.as_ref(), Type::Seq(_))))
                {
                    return Err(self.err("logical lifting is limited to one sequence dimension"));
                }
                let ash = bool_shape(&a);
                let bsh = bool_shape(&b);
                self.unify(&a, &ash)?;
                self.unify(&b, &bsh)?;
                fn depth(t: &Type) -> usize {
                    if let Type::Seq(t) = t {
                        1 + depth(t)
                    } else {
                        0
                    }
                }
                if depth(&ash) >= depth(&bsh) {
                    ash
                } else {
                    bsh
                }
            }
            "-:" | "|" | "^/" => {
                self.req(Class::Num, &xs[0])?;
                match (n, self.cx.resolve(&xs[0])) {
                    ("^/", _) => p(R),
                    ("-:", Type::Prim(N)) => p(Z),
                    ("-:", t @ Type::Var(_)) => {
                        self.require_exact_numeric(&t, "negation")?;
                        self.cx.resolve(&t)
                    }
                    (_, t) => t,
                }
            }
            ":n" | ":z" | ":q" | ":d" | ":r" => {
                self.req(Class::Num, &xs[0])?;
                if n == ":d" {
                    return Err(
                        self.err("decimal conversion is not executable in the reference subset")
                    );
                }
                p(match n {
                    ":n" => N,
                    ":z" => Z,
                    ":q" => Q,
                    ":d" => D,
                    _ => R,
                })
            }
            "#" => match self.cx.resolve(&xs[0]) {
                Type::Seq(_) | Type::Table(_) | Type::Dict(..) | Type::Prim(S) => p(N),
                Type::Var(_) => {
                    return Err(self.unsupported(
                        "cardinality on an unknown collection shape requires annotation",
                    ))
                }
                t => return Err(self.err(format!("cannot take cardinality of {t}"))),
            },
            "+/" | "*/" | "avg" | "med" | "var" | "sd" | ">/" | "</" | "&&/" | "||/" => {
                let elt = self.seq_element(&xs[0])?;
                if matches!(n, "&&/" | "||/") {
                    self.unify(&elt, &p(B))?;
                    p(B)
                } else {
                    self.req(Class::Num, &elt)?;
                    match n {
                        "avg" | "med" | "var" => {
                            self.require_exact_numeric(&elt, n)?;
                            o(p(Q))
                        }
                        "sd" => o(p(R)),
                        ">/" | "</" => o(elt),
                        _ => elt,
                    }
                }
            }
            "pct" => {
                self.require_exact_numeric(&xs[0], n)?;
                let t = self.seq_element(&xs[1])?;
                self.require_exact_numeric(&t, n)?;
                o(p(Q))
            }
            "?+" | "?!" => {
                let t = self.cx.fresh();
                self.unify(&xs[0], &o(t.clone()))?;
                if n == "?+" {
                    p(B)
                } else {
                    t
                }
            }
            "csv.read" => {
                self.unify(&xs[0], &p(S))?;
                self.csv_read_type(want)?
            }
            "csv.write" => {
                self.unify(&xs[0], &p(S))?;
                let Type::Table(row) = self.cx.resolve(&xs[1]) else {
                    return Err(self.err("csv.write expects a statically known table schema"));
                };
                self.csv_row(&row)?;
                a(result(p(Unit), Self::standard_type("csverr")))
            }
            "df.agg" => {
                let src = self.cx.resolve(&xs[2]);
                let Type::Table(row) = &src else {
                    return Err(self.err("df.agg expects a statically known table schema"));
                };
                let key = self.cx.fresh();
                self.unify(&xs[0], &f(row.as_ref().clone(), key.clone()))?;
                self.req(Class::Hash, &key)?;
                let output = match want.map(|w| self.cx.resolve(w)) {
                    Some(Type::Table(row)) => *row,
                    _ => self.cx.fresh(),
                };
                self.unify(&xs[1], &f(key, f(src, output.clone())))?;
                self.row(&output)?;
                Type::Table(b(output))
            }
            "df.extend" => {
                let Type::Table(row) = self.cx.resolve(&xs[1]) else {
                    return Err(self.err("df.extend expects a statically known table schema"));
                };
                let mut out = self.row(&row)?;
                let columns = self.row(&xs[0])?;
                if columns.fields.is_empty() {
                    return Err(self.err("df.extend requires at least one new column"));
                }
                for (name, t) in columns.fields {
                    if out.fields.iter().any(|(k, _)| k == &name) {
                        return Err(
                            self.err(format!("df.extend cannot replace existing column {name}"))
                        );
                    }
                    out.fields.push((name, self.seq_element(&t)?));
                }
                Type::Table(b(Type::Record(out)))
            }
            "io.print" | "io.println" => a(result(p(Unit), Self::standard_type("ioerr"))),
            "io.read" | "io.exists" | "io.write" => {
                for path in xs {
                    self.unify(path, &p(S))?;
                }
                let ok = match n {
                    "io.read" => p(S),
                    "io.exists" => p(B),
                    _ => p(Unit),
                };
                a(result(ok, Self::standard_type("ioerr")))
            }
            "math.div" => {
                for x in xs {
                    self.require_exact_numeric(x, "math.div")?;
                }
                result(p(Q), Self::standard_type("numerr"))
            }
            "take" | "[:" | "drop" | "slice" | "rev" => {
                let (ns, last) = xs.split_at(xs.len() - 1);
                for i in ns {
                    self.unify(i, &p(N))?;
                }
                match self.cx.resolve(&last[0]) {
                    t @ (Type::Seq(_) | Type::Table(_) | Type::Prim(S)) => t,
                    Type::Var(_) => {
                        return Err(self.unsupported(format!("{n} needs a known collection shape")))
                    }
                    t => {
                        return Err(
                            self.err(format!("{n} expects sequence, table, or string; found {t}"))
                        )
                    }
                }
            }
            "flat" | "tr" => {
                let el = self.cx.fresh();
                self.unify(&xs[0], &s(s(el.clone())))?;
                if n == "flat" {
                    s(el)
                } else {
                    s(s(el))
                }
            }
            "=." => {
                let el = self.seq_element(&xs[0])?;
                self.req(Class::Eq, &el)?;
                s(el)
            }
            "#:" => {
                let source = self.cx.resolve(&xs[0]);
                let row =
                    match source {
                        Type::Seq(row)
                            if matches!(
                                row.as_ref(),
                                Type::Record(_) | Type::Nominal(_) | Type::Tuple(_)
                            ) =>
                        {
                            *row
                        }
                        Type::Record(_) | Type::Nominal(_) => {
                            let cols = self.row(&source)?;
                            if cols.fields.is_empty() {
                                return Err(self.err("#: requires at least one column"));
                            }
                            let fields = cols
                                .fields
                                .into_iter()
                                .map(|(name, t)| Ok((name, self.seq_element(&t)?)))
                                .collect::<crate::infer::R<_>>()?;
                            Type::Record(Row { fields, tail: None })
                        }
                        _ => return Err(self.err(
                            "#: reference needs a known sequence of rows or record-of-columns type",
                        )),
                    };
                Type::Table(b(row))
            }
            ":#" => {
                let row = self.cx.fresh();
                self.unify(&xs[0], &Type::Table(b(row.clone())))?;
                s(row)
            }
            "+:" => {
                self.unify(&xs[0], &xs[1])?;
                self.collection(&xs[0])?;
                xs[0].clone()
            }
            "&:" => {
                let a = self.seq_element(&xs[0])?;
                let b = self.seq_element(&xs[1])?;
                s(Type::Tuple(vec![a, b]))
            }
            "*:" => match (self.cx.resolve(&xs[0]), self.cx.resolve(&xs[1])) {
                (Type::Table(a), Type::Table(b1)) => Type::Table(b(Type::Tuple(vec![*a, *b1]))),
                (Type::Seq(a), Type::Seq(b1)) => s(Type::Tuple(vec![*a, *b1])),
                _ => {
                    return Err(
                        self.err("*: expects two sequences or two tables of compatible kinds")
                    )
                }
            },
            n if n.starts_with('.') && n != ".:" => self.select(&n[1..], &xs[0])?,
            n => return Err(self.unsupported(format!("{n} is not covered by Draft 0.4 inference"))),
        })
    }
    fn bind(&mut self, env: &mut Env, stmt: &Stmt, name: &str, scheme: Scheme) {
        env.insert(name.to_owned(), scheme.clone());
        if let Some(id) = self.resolver.declaration(stmt) {
            self.result.bindings.insert(id, scheme);
        }
    }
    fn body(&mut self, env: &Env, stmt: &Stmt, sig: &Signature, effect: bool) -> R<()> {
        let Stmt::Def {
            name,
            params,
            body,
            start,
            ..
        } = stmt
        else {
            unreachable!()
        };
        self.start = *start;
        if params.is_empty() {
            self.expr(env, body, Some(&sig.out), effect)
                .map_err(|mut e| {
                    e.message = format!("{name}: {}", e.message);
                    e
                })?;
            return Ok(());
        }
        let mut nested = env.clone();
        let ids = self.resolver.parameters(stmt);
        for (i, (name, t)) in params.iter().zip(&sig.args).enumerate() {
            nested.insert(name.clone(), mono(t.clone()));
            if let Some(id) = ids.get(i) {
                self.result.bindings.insert(*id, mono(t.clone()));
            }
        }
        self.reset_boundary(|this| this.expr(&nested, body, Some(&sig.out), false))
            .map_err(|mut e| {
                e.message = format!("{name}: {}", e.message);
                e
            })?;
        Ok(())
    }
    fn finish_signature(&mut self, sig: &Signature, outer: &Env) -> R<()> {
        if sig.explicit.is_empty() {
            return Ok(());
        }
        let ids = sig
            .explicit
            .values()
            .filter_map(|t| {
                if let Type::Var(id) = t {
                    Some(*id)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        let outer_types = outer.values().cloned().collect::<Vec<_>>();
        if self.cx.rigid_escapes(&ids, &outer_types) {
            return Err(
                self.err("a quantified signature variable escapes into a captured outer binding")
            );
        }
        for c in self.cx.constraints() {
            let t = self.cx.resolve(&c.ty);
            if self.cx.free_vars(&t).iter().any(|id| ids.contains(id))
                && !sig
                    .qualifiers
                    .iter()
                    .any(|q| q.class == c.class && self.cx.resolve(&q.ty) == t)
            {
                return Err(self.err(format!("annotated signature lacks {:?} {t}", c.class)));
            }
        }
        for id in ids {
            self.cx.unmark_rigid(id)
        }
        Ok(())
    }
    fn def_signature(&mut self, stmt: &Stmt) -> R<Signature> {
        let Stmt::Def {
            params,
            param_types,
            result_type,
            start,
            ..
        } = stmt
        else {
            unreachable!()
        };
        self.start = *start;
        let mut sig = self.signature(params, param_types, result_type.as_ref())?;
        if let Some(qs) = self.qualifiers.get(start) {
            for (c, v) in qs {
                let class = match c.as_str() {
                    "num" => Class::Num,
                    "eq" => Class::Eq,
                    "ord" => Class::Ord,
                    "hash" => Class::Hash,
                    _ => return Err(self.err(format!("unknown type constraint {c}"))),
                };
                let ty = sig
                    .explicit
                    .get(v)
                    .ok_or_else(|| {
                        self.err(format!("constraint variable {v} is not in the signature"))
                    })?
                    .clone();
                sig.qualifiers.push(Constraint { class, ty });
            }
        }
        for t in sig.explicit.values() {
            if let Type::Var(id) = t {
                self.cx.mark_rigid(*id);
                self.explicit_variables.insert(*id);
            }
        }
        for c in &sig.qualifiers {
            self.req(c.class, &c.ty)?;
        }
        Ok(sig)
    }
    fn scheme(&self, env: &Env, typ: &Type, allow: bool) -> Scheme {
        if !allow {
            return mono(self.cx.resolve(typ));
        }
        let candidate = self.cx.generalize(typ, &[], &[]);
        if candidate.vars.is_empty() {
            return candidate;
        }
        let others = env.values().cloned().collect::<Vec<_>>();
        self.cx.generalize(typ, &others, &self.cx.constraints())
    }
    fn closed_integer_type(&mut self, e: &Expr, env: &Env) -> Option<Type> {
        // This is equivalent to defaulting each connected literal metavariable to
        // z. Only recognize closed scalar integer terms whose operators preserve z.
        // Stage metadata until the entire term has been classified.
        fn collect<'a>(e: &'a Expr, env: &Env, out: &mut Vec<&'a Expr>) -> bool {
            match e {
                Expr::Int(_) => {}
                Expr::Call(n, args)
                    if matches!(n.as_str(), "+" | "-" | "*")
                        && args.len() == 2
                        && !env.contains_key(n) =>
                {
                    if !args.iter().all(|x| collect(x, env, out)) {
                        return false;
                    }
                }
                _ => return false,
            }
            out.push(e);
            true
        }
        let mut nodes = Vec::with_capacity(8);
        if !collect(e, env, &mut nodes) {
            return None;
        }
        let z = p(Prim::Z);
        for node in nodes {
            let addr = address(node);
            self.result.expressions.insert(addr, z.clone());
            if let Expr::Call(op, _) = node {
                self.result.operations.insert(
                    addr,
                    Operation::Numeric {
                        operator: op.clone(),
                        result: z.clone(),
                    },
                );
            }
        }
        Some(z)
    }
    fn group(&mut self, defs: Vec<&Stmt>, env: &mut Env, effect: bool) -> R<()> {
        // Ordinary immutable values do not add a recursive header. Infer them
        // against the existing environment instead of cloning every prior value.
        if let [stmt @ Stmt::Def {
            name,
            params,
            body,
            result_type,
            start,
            ..
        }] = defs.as_slice()
        {
            if params.is_empty() {
                if result_type.is_none() && !self.qualifiers.contains_key(start) {
                    if let Some(ty) = self.closed_integer_type(body, env) {
                        self.bind(env, stmt, name, mono(ty));
                        return Ok(());
                    }
                }
                let sig = self.def_signature(stmt)?;
                let pos = self.literals.len();
                self.body(env, stmt, &sig, effect)?;
                self.default_literals(pos)?;
                self.start = *start;
                self.finish_signature(&sig, env)?;
                if matches!(body,Expr::Array(es) if es.is_empty())
                    && matches!(self.cx.resolve(&sig.out),Type::Seq(x) if matches!(*x,Type::Var(_)))
                {
                    return Err(self.err("empty sequence needs an element type annotation"));
                }
                let sc = self.scheme(env, &sig.out, syntactic_value(body));
                self.bind(env, stmt, name, sc);
                return Ok(());
            }
        }
        let outer = env.clone();
        let mut sigs = Vec::new();
        let literals = self.literals.len();
        for stmt in defs {
            match stmt {
                Stmt::Schema { .. } | Stmt::Sum { .. } => {}
                Stmt::Def { name, params, .. } => {
                    let sig = self.def_signature(stmt)?;
                    if !params.is_empty() {
                        let t = function(sig.args.iter().cloned(), sig.out.clone());
                        env.insert(name.clone(), mono(t));
                    }
                    sigs.push((stmt, sig));
                }
                Stmt::Effect { name, body, start } => {
                    self.start = *start;
                    if !effect && self.boundaries.is_empty() {
                        return Err(self.err("expression statement in pure block"));
                    }
                    let t = self.expr(env, body, None, effect)?;
                    if let Some(n) = name {
                        self.bind(env, stmt, n, mono(t));
                    } else {
                        let span = self.resolver.span(body).unwrap_or(*start..*start);
                        self.discarded_results.push((t, span.start, span.end));
                    }
                }
            }
        }
        for (stmt, sig) in &sigs {
            self.body(env, stmt, sig, effect)?;
        }
        self.default_literals(literals)?;
        for (stmt, sig) in sigs {
            let Stmt::Def {
                params,
                name,
                body,
                start,
                ..
            } = stmt
            else {
                unreachable!()
            };
            self.start = *start;
            self.finish_signature(&sig, &outer)?;
            let t = function(sig.args.iter().cloned(), sig.out.clone());
            if params.is_empty()
                && matches!(body,Expr::Array(es) if es.is_empty())
                && matches!(self.cx.resolve(&t), Type::Seq(x) if matches!(*x, Type::Var(_)))
            {
                return Err(self.err("empty sequence needs an element type annotation"));
            }
            let sc = self.scheme(&outer, &t, !params.is_empty() || syntactic_value(body));
            self.bind(env, stmt, name, sc);
        }
        Ok(())
    }
    fn local_defs(&mut self, defs: &[Stmt], env: &mut Env, effect: bool) -> R<()> {
        // Order value visibility by the declaration itself. A local recursive
        // group stays monomorphic until all transitively referenced functions
        // have been checked, then its variables can be freshly instantiated.
        let base = env.clone();
        let local = defs
            .iter()
            .filter_map(|stmt| match stmt {
                Stmt::Def { params, .. } if !params.is_empty() => self.resolver.declaration(stmt),
                _ => None,
            })
            .collect::<HashSet<_>>();
        let local_names = local
            .iter()
            .map(|&id| self.resolver.symbol(id).name.as_str())
            .collect::<HashSet<_>>();
        let mut signatures = HashMap::new();
        let mut deps = HashMap::new();
        for stmt in defs {
            match stmt {
                Stmt::Schema { .. } | Stmt::Sum { .. } => {}
                Stmt::Def {
                    name, params, body, ..
                } if !params.is_empty() => {
                    let id = self
                        .resolver
                        .declaration(stmt)
                        .ok_or_else(|| self.err("missing local callable symbol"))?;
                    let sig = self.def_signature(stmt)?;
                    env.insert(
                        name.clone(),
                        mono(function(sig.args.iter().cloned(), sig.out.clone())),
                    );
                    let mut incoming = HashSet::new();
                    scan_dependencies(body, self.resolver, &local, &mut incoming);
                    deps.insert(id, incoming);
                    signatures.insert(id, sig);
                }
                _ => {}
            }
        }
        let mut pending: Vec<(SymbolId, &Stmt, Signature)> = Vec::new();
        let mut checked = HashSet::new();
        for stmt in defs {
            match stmt {
                Stmt::Def { params, .. } if !params.is_empty() => {
                    let id = self.resolver.declaration(stmt).expect("local symbol");
                    let sig = signatures.remove(&id).expect("local header signature");
                    let pos = self.literals.len();
                    self.body(env, stmt, &sig, effect)?;
                    self.default_literals(pos)?;
                    checked.insert(id);
                    pending.push((id, stmt, sig));
                    // A candidate can be generalized exactly when its whole
                    // reachable local-callable set has been checked.
                    let ready = |i| {
                        let mut stack = vec![i];
                        let mut seen = HashSet::new();
                        while let Some(id) = stack.pop() {
                            if !seen.insert(id) {
                                continue;
                            }
                            if !checked.contains(&id) {
                                return false;
                            }
                            if let Some(ds) = deps.get(&id) {
                                stack.extend(ds.iter().copied());
                            }
                        }
                        true
                    };
                    let mut done = Vec::new();
                    let mut waiting = Vec::new();
                    for item in pending.drain(..) {
                        if ready(item.0) {
                            done.push(item);
                        } else {
                            waiting.push(item);
                        }
                    }
                    pending = waiting;
                    if !done.is_empty() {
                        let mut visible = base.clone();
                        visible.extend(
                            env.iter()
                                .filter(|(n, _)| !local_names.contains(n.as_str()))
                                .map(|(n, t)| (n.clone(), t.clone())),
                        );
                        for (_, stmt, sig) in done {
                            let Stmt::Def { name, start, .. } = stmt else {
                                unreachable!()
                            };
                            self.start = *start;
                            self.finish_signature(&sig, &visible)?;
                            let t = function(sig.args.into_iter(), sig.out);
                            let sc = self.scheme(&visible, &t, true);
                            self.bind(env, stmt, name, sc);
                        }
                    }
                }
                Stmt::Schema { .. } | Stmt::Sum { .. } => {}
                _ => self.group(vec![stmt], env, effect)?,
            }
        }
        Ok(())
    }
}
fn visit_inference_exprs(p: &Program, mut f: impl FnMut(&Expr) -> R<()>) -> R<()> {
    fn defs(ds: &[Stmt], f: &mut impl FnMut(&Expr) -> R<()>) -> R<()> {
        for s in ds {
            if let Stmt::Def { body, .. } | Stmt::Effect { body, .. } = s {
                visit(body, f)?;
            }
        }
        Ok(())
    }
    fn visit(e: &Expr, f: &mut impl FnMut(&Expr) -> R<()>) -> R<()> {
        use Expr::*;
        f(e)?;
        match e {
            Call(_, xs) | Tuple(xs) | Array(xs) | Pipeline(xs) => {
                for x in xs {
                    visit(x, f)?;
                }
            }
            Record(fs) | NamedRecord(_, fs) => {
                for (_, x) in fs {
                    visit(x, f)?;
                }
            }
            Block(ss, tail) | Action(ss, tail) | Fallible(ss, tail) => {
                defs(ss, f)?;
                if let Some(x) = tail {
                    visit(x, f)?;
                }
            }
            Cases(cs, x) => {
                for (c, b) in cs {
                    visit(c, f)?;
                    visit(b, f)?;
                }
                visit(x, f)?;
            }
            Lambda(_, x) | TypedLambda(_, _, x) | Raise(x) | Propagate(x) => visit(x, f)?,
            Recover(x, cs) => {
                visit(x, f)?;
                for c in cs {
                    visit(&c.body, f)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    defs(&p.defs, &mut f)?;
    if let Some(e) = &p.entry {
        visit(e, &mut f)?;
    }
    Ok(())
}
fn immediate_raise(e: &Expr) -> bool {
    use Expr::*;
    match e {
        Raise(_) => true,
        Call(_, xs) | Tuple(xs) | Array(xs) | Pipeline(xs) => xs.iter().any(immediate_raise),
        Record(fs) | NamedRecord(_, fs) => fs.iter().any(|(_, x)| immediate_raise(x)),
        Block(ss, tail) => {
            ss.iter().any(|s| match s {
                Stmt::Def { params, body, .. } if params.is_empty() => immediate_raise(body),
                Stmt::Effect { body, .. } => immediate_raise(body),
                _ => false,
            }) || tail.as_deref().is_some_and(immediate_raise)
        }
        Cases(cs, x) => {
            cs.iter()
                .any(|(c, b)| immediate_raise(c) || immediate_raise(b))
                || immediate_raise(x)
        }
        Recover(x, cs) => immediate_raise(x) || cs.iter().any(|c| immediate_raise(&c.body)),
        Propagate(x) => immediate_raise(x),
        _ => false,
    }
}
fn fallible_builtin(name: &str) -> bool {
    matches!(
        name,
        "io.print"
            | "io.println"
            | "io.read"
            | "io.write"
            | "io.exists"
            | "csv.read"
            | "csv.write"
            | "math.div"
    )
}
fn find_entry_natural_tail(p: &Program) -> Option<usize> {
    let Expr::Action(_, Some(tail)) = p.entry.as_ref()? else {
        return None;
    };
    let Expr::Fallible(_, Some(final_)) = tail.as_ref() else {
        return None;
    };
    matches!(final_.as_ref(),Expr::Int(x) if *x>=0).then(|| address(final_))
}
pub fn infer(p: &Program, resolver: &Resolution) -> R<InferResult> {
    let mut e = Engine {
        cx: InferCtx::new(),
        result: InferResult::default(),
        resolver,
        schemas: HashMap::new(),
        schema_scopes: vec![HashMap::new()],
        sums: HashMap::new(),
        boundaries: vec![],
        discarded_results: vec![],
        entry_natural_tail: find_entry_natural_tail(p),
        entry_boundary: p.entry.as_ref().and_then(|e| {
            if let Expr::Action(_, Some(tail)) = e {
                matches!(tail.as_ref(), Expr::Fallible(..)).then(|| address(tail))
            } else {
                None
            }
        }),
        raise_payload: None,
        saw_result: false,
        start: 0,
        literals: vec![],
        active: None,
        empty_sequences: vec![],
        explicit_variables: HashSet::new(),
        qualifiers: &p.constraints,
    };
    let mut env = Env::new();
    e.install_standard_errors(&p.imports)?;
    e.schema_headers(&p.defs)?;
    for group in &resolver.groups {
        e.group(group.iter().map(|i| &p.defs[*i]).collect(), &mut env, false)?;
    }
    if let Some(entry) = &p.entry {
        e.start = p.entry_start.unwrap_or(0);
        let pos = e.literals.len();
        e.expr(&env, entry, None, false)?;
        e.default_literals(pos)?;
    }
    if e.saw_result {
        for (t, start, end) in &e.discarded_results {
            if matches!(e.cx.resolve(t), Type::Result(..)) {
                return Err(InferError{kind:InferErrorKind::TypeError,start:*start,end:*end,expected:None,actual:None,message:"result-valued statement would discard a failure; propagate, recover, or bind its result".into()});
            }
        }
        let quantified = e
            .result
            .bindings
            .values()
            .flat_map(|sc| sc.vars.iter().copied())
            .collect::<HashSet<_>>();
        visit_inference_exprs(p, |ex| {
            if let Some(t @ (Type::Var(_) | Type::Result(..))) = e.result.type_of(ex) {
                let t = e.cx.resolve(t);
                if matches!(t, Type::Result(..))
                    && e.cx.free_vars(&t).iter().any(|id| !quantified.contains(id))
                {
                    let at = resolver.offset(ex).unwrap_or(0);
                    let span = resolver.span(ex).unwrap_or(at..at);
                    return Err(InferError {
                        kind: InferErrorKind::TypeError,
                        start: span.start,
                        end: span.end,
                        expected: None,
                        actual: None,
                        message: "ambiguous fallible result: annotate the success and error types"
                            .into(),
                    });
                }
            }
            Ok(())
        })?;
    }
    for (element, start, end) in &e.empty_sequences {
        if e.cx
            .free_vars(element)
            .iter()
            .any(|id| !e.explicit_variables.contains(id))
        {
            return Err(InferError {
                kind: InferErrorKind::TypeError,
                start: *start,
                end: *end,
                expected: None,
                actual: None,
                message: "empty sequence has an ambiguous element type; add an annotation".into(),
            });
        }
    }
    for typ in e.result.expressions.values_mut() {
        *typ = e.cx.resolve(typ);
    }
    for typ in e.result.callees.values_mut() {
        *typ = e.cx.resolve(typ);
    }
    for scheme in e.result.bindings.values_mut() {
        scheme.ty = e.cx.resolve(&scheme.ty);
        for c in &mut scheme.constraints {
            c.ty = e.cx.resolve(&c.ty);
        }
    }
    for op in e.result.operations.values_mut() {
        if let Operation::Numeric { result, .. } = op {
            *result = e.cx.resolve(result);
        }
    }
    for (name, row) in e.schemas {
        let Type::Record(row) = e.cx.resolve(&Type::Record(row)) else {
            unreachable!()
        };
        e.result.schemas.insert(name, row);
    }
    Ok(e.result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn check(source: &str) -> R<InferResult> {
        let p = crate::parser::parse(source).unwrap();
        let names = crate::resolve::resolve(&p).unwrap();
        infer(&p, &names)
    }
    #[test]
    fn independent_generic_instantiations_and_recursive_scc() {
        check(
            r#"id(x) := x; a := id 1; b := id "x";
          even(x:n):b := ? (= x 0) 1b (odd (- x 1));
          odd(x:n):b := ? (= x 0) 0b (even (- x 1));
          c := even 2;"#,
        )
        .unwrap();
        assert_eq!(
            check("loop(x) := loop [x];").unwrap_err().kind,
            InferErrorKind::TypeError
        );
    }
    #[test]
    fn higher_order_and_open_rows() {
        check(
            r#"
          apply(g,x) := @ g x;
          id(v) := v;
          value(row) := .val row;
          a := apply &id 3;
          b := value .{ label := "a"; val := 1; };
          c := value .{ val := 1.25; flag := 1b; };
        "#,
        )
        .unwrap();
        assert_eq!(
            check("value(r) := .val r; a := value .{ nope := 0; };")
                .unwrap_err()
                .kind,
            InferErrorKind::TypeError
        );
    }
    #[test]
    fn annotated_signature_constraints_and_nominal_evidence() {
        check("f(x:['a]):b | eq 'a := == x x; >> (f);").unwrap();
        let e = check("f(x:['a]):b := == x x; >> (f);").unwrap_err();
        assert_eq!(e.kind, InferErrorKind::TypeError);
        assert!(e.message.contains("lacks"));
        let e=check("schema :: { callback:z->z; }; id(x) := x; same(x) := == x x; data := schema { callback:=&id; }; b := same data;").unwrap_err();
        assert_eq!(e.kind, InferErrorKind::TypeError);
        assert!(e.message.contains("callback"), "{e:?}");
        check("schema :: { count:z; }; same(x) := == x x; data := schema { count:=2; }; b := same data;").unwrap();
    }
    #[test]
    fn value_restriction_and_empty_ambiguity() {
        assert!(check(r#"id(x) := x; x := ? 1b &id &id; a := @ x 1; b := @ x "s";"#).is_err());
        assert!(check("empty := [];").is_err());
        assert!(check("bad(x) := == [] [];")
            .unwrap_err()
            .message
            .contains("ambiguous"));
        assert!(check("bad(x) := { y := ([],x); x };")
            .unwrap_err()
            .message
            .contains("ambiguous"));
        check("empty:[z] := [];").unwrap();
        check("f(x:['a]):b | eq 'a := == x [];").unwrap();
    }
    #[test]
    fn local_functions_generalize_after_acyclic_and_recursive_groups() {
        check(
            r#"run(u:()):() := {
            id(x) := x;
            a := id 1;
            b := id "one";
            first(x) := second x;
            second(y) := y;
            c := first 1;
            d := first "two";
            ()
        };"#,
        )
        .unwrap();
    }
    #[test]
    fn diagnostics_have_the_expression_span_when_resolved() {
        let source = "good := 1;\ninvalid := * 2 0b;";
        let e = check(source).unwrap_err();
        assert_eq!(e.start, source.find('*').unwrap());
        assert_eq!(e.end, source.len() - 1);
    }
    #[test]
    fn schemas_keep_lexical_identity_and_fields() {
        let inferred = check(
            r#"
            rr :: { x:z; };
            take_outer(v:rr):z := .x v;
            outer:rr := rr { x:=7; };
            val:s := {
                rr :: { y:s; };
                inner:rr := rr { y:="inside"; };
                seen:z := take_outer outer;
                .y inner
            };
            after:rr := rr { x:=8; };
            next:z := take_outer after;
        "#,
        )
        .unwrap();
        let shapes = inferred
            .schemas
            .iter()
            .filter(|(id, _)| id.name == "rr")
            .collect::<Vec<_>>();
        assert_eq!(shapes.len(), 2);
        assert_ne!(shapes[0].0.id, shapes[1].0.id);
        let e = check("rr :: { v:z; }; bad:rr := { rr :: { v:z; }; rr { v:=1; } };").unwrap_err();
        assert_eq!(e.kind, InferErrorKind::TypeError);
    }

    #[test]
    fn rejects_skolem_escape_from_nested_signature() {
        let e = check(
            r#"
            outer(f) := {
                pretend(x:'a):'a := @ f x;
                pretend "s"
            };
        "#,
        )
        .unwrap_err();
        assert!(e.message.contains("escapes"), "{e:?}");
        check(r#"id(x:'a):'a := x; i := id 7; s := id "s";"#).unwrap();
        check("sq(x:'a):'a | num 'a := * x x; a := sq 4;").unwrap();
    }
    #[test]
    fn closed_default_integer_terms_preserve_context_and_evidence() {
        let p=crate::parser::parse(
            "u := * (+ 2 3) 4; n:n := + 2 3; q:q := + 2 3; r := + n 4; fnn(x:q):q := + x 1; x := fnn 5;"
        ).unwrap();
        let names = crate::resolve::resolve(&p).unwrap();
        let r = infer(&p, &names).unwrap();
        let body = |i: usize| match &p.defs[i] {
            Stmt::Def { body, .. } => body,
            _ => unreachable!(),
        };
        let z = Type::Prim(Prim::Z);
        assert_eq!(r.type_of(body(0)), Some(&z));
        assert_eq!(
            r.operation_of(body(0)),
            Some(&Operation::Numeric {
                operator: "*".into(),
                result: z.clone()
            })
        );
        let Expr::Call(_, xs) = body(0) else {
            unreachable!()
        };
        assert_eq!(r.type_of(&xs[0]), Some(&z));
        assert_eq!(r.type_of(&xs[1]), Some(&z));
        assert_eq!(r.type_of(body(1)), Some(&Type::Prim(Prim::N)));
        assert_eq!(r.type_of(body(2)), Some(&Type::Prim(Prim::Q)));
        assert_eq!(r.type_of(body(3)), Some(&Type::Prim(Prim::N)));
        assert_eq!(
            r.callee_type(body(5)),
            Some(&f(Type::Prim(Prim::Q), Type::Prim(Prim::Q)))
        );
    }
    #[test]
    fn result_boundaries_skips_and_handler_types() {
        let src = r#"
            fail :: { | code z; | other; };
            div(x:z):z^fail := ^{ ? 1b (^: code x) x };
            value := ^= div 7 { code x : x; other : 1; };
            !{ ^{ ^? div 0; () } };
        "#;
        let p = crate::parser::parse(src).unwrap();
        let names = crate::resolve::resolve(&p).unwrap();
        let r = infer(&p, &names).unwrap();
        assert_eq!(r.boundaries.len(), 4);
        assert_eq!(
            r.boundaries.values().copied().collect::<HashSet<_>>().len(),
            2
        );
        assert_eq!(
            r.constructors
                .values()
                .filter(|c| c.owner.name == "fail")
                .count(),
            2
        );
        let err =
            check("fail :: { | a; | b; }; f(x:z):z^fail := ^{ ^: a }; s := ^= f 0 { a : 0; };")
                .unwrap_err();
        assert!(err.message.contains("nonexhaustive"), "{err:?}");
        let err =
            check("fail :: { | a; }; f(x:z):z^fail := ^{ ^: a }; s := ^= f 0 { a : 0; _ : 1; };")
                .unwrap_err();
        assert!(err.message.contains("unreachable"), "{err:?}");
    }

    #[test]
    fn rejects_executable_result_ambiguity_and_unconsumed_pipeline() {
        check("make(x) := ^{ x };").unwrap();
        let e = check("make(x) := ^{ x }; !{ n := make 1; () };").unwrap_err();
        assert!(e.message.contains("ambiguous"), "{e:?}");
        let e = check("err :: { | bad; }; fail(x:z):z^err := ^{ ^: bad }; !{ |{ fail 0z; () } };")
            .unwrap_err();
        assert!(e.message.contains("discard"), "{e:?}");
        check("err :: { | bad; }; fail(x:z):z^err := ^{ ^: bad }; !{ |{ fail 0z; ^= $ { bad : 0z; }; () } };").unwrap();
    }

    #[test]
    fn resets_lexical_error_boundary_at_closures_functions_and_actions() {
        for source in [
            r#"e :: { | wrong; }; f(x:z):z^e := ^{ f2 := \u -> ^: wrong; x };"#,
            "e :: { | wrong; }; f(x:z):z^e := ^{ f2(u:()):z := ^: wrong; x };",
            "e :: { | wrong; }; f(x:z):z^e := ^{ suspended := !{ ^: wrong }; x };",
        ] {
            let err = check(source).unwrap_err();
            assert!(err.message.contains("boundary"), "{err:?}");
        }
        let err = check("f(x:z):z^s := ^{ ^: x };").unwrap_err();
        assert_eq!(err.kind, InferErrorKind::IncompatibleError);
    }
    #[test]
    fn staged_builtin_references_preserve_arity_and_collection_context() {
        let src = r#"
            sum(x,y:z):z := + x y;
            foldsum := @ &/: &sum;
            total:z := @ (@ foldsum 0) [1 2 3];
            accs:[z] := \: &sum 0 [];
            joined:[q] := +: [] [];
            paired:[(z,s)] := &: [] [];
            crossed:[(z,s)] := *: [1] ["x"];
            idx:n->[z]->?z := &.:;
            found:?z := @ (@ idx 1) [7];
            measure:[q]->?q := &var;
            stat:?q := @ measure [1 3];
        "#;
        check(src).unwrap();
    }

    #[test]
    fn higher_order_actions_receive_schema_and_filter_uses_the_actual_mask_type() {
        check(
            r#"
            << "csv";
            row :: { value:q; };
            typed(action:!(#row^csverr)):!(#row^csverr) := action;
            load := @ &typed csv.read "nothing.csv";
            flags(u:()):[b] := [1b 0b];
            kept:[z] := ?: flags () [2 3];
        "#,
        )
        .unwrap();
    }

    #[test]
    fn annotation_failure_points_at_the_unresolved_named_type() {
        let src = "f(x:?absent):z := 1;";
        let err = check(src).unwrap_err();
        assert_eq!(&src[err.start..err.end], "absent");
        assert!(err.message.contains("unknown type"));
    }
}
