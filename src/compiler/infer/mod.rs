//! Checked rank-one inference and its public typed metadata.
mod annotations;
mod builtin;
mod collections;
mod constraints;
mod declarations;
mod effects;
mod expressions;
mod matches;
mod nominal;
mod numeric;

#[cfg(test)]
mod tests;

use crate::parser::{Expr, Program, Stmt, SyntaxId};
use crate::resolve::{ProgramShape, Resolution, SymbolId, SyntaxLocator};
use crate::types::{Constraint, InferCtx, NominalId, Prim, Row, Scheme, Type};
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
    pub constraint: Option<String>,
}
impl std::fmt::Display for InferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for InferError {}
pub use crate::core::operation::Operation;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Constructor {
    pub owner: NominalId,
    pub id: SymbolId,
    pub payload: Vec<Type>,
}
#[derive(Debug, Default)]
pub struct InferResult {
    syntax: std::sync::Arc<SyntaxLocator>,
    pub(crate) program_shape: Option<std::sync::Arc<ProgramShape>>,
    pub expressions: HashMap<SyntaxId, Type>,
    pub bindings: HashMap<SymbolId, Scheme>,
    pub operations: HashMap<SyntaxId, Operation>,
    pub callees: HashMap<SyntaxId, Type>,
    pub schemas: HashMap<NominalId, Row>,
    pub constructors: HashMap<SymbolId, Constructor>,
    pub boundaries: HashMap<SyntaxId, SyntaxId>,
}
impl InferResult {
    pub fn constructor(&self, id: SymbolId) -> Option<&Constructor> {
        self.constructors.get(&id)
    }
    pub fn boundary_of(&self, e: &Expr) -> Option<SyntaxId> {
        self.boundary_at(self.syntax.id(e)?)
    }
    pub fn boundary_at(&self, id: SyntaxId) -> Option<SyntaxId> {
        self.boundaries.get(&id).copied()
    }
    pub fn nominal_schema(&self, id: &NominalId) -> Option<&Row> {
        self.schemas.get(id)
    }
    pub fn type_of(&self, e: &Expr) -> Option<&Type> {
        self.type_at(self.syntax.id(e)?)
    }
    pub fn type_at(&self, id: SyntaxId) -> Option<&Type> {
        self.expressions.get(&id)
    }
    pub fn binding_type(&self, id: SymbolId) -> Option<&Scheme> {
        self.bindings.get(&id)
    }
    pub fn callee_type(&self, e: &Expr) -> Option<&Type> {
        self.callee_type_at(self.syntax.id(e)?)
    }
    pub fn callee_type_at(&self, id: SyntaxId) -> Option<&Type> {
        self.callees.get(&id)
    }
    pub fn operation_of(&self, e: &Expr) -> Option<&Operation> {
        self.operation_at(self.syntax.id(e)?)
    }
    pub fn operation_at(&self, id: SyntaxId) -> Option<&Operation> {
        self.operations.get(&id)
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
    Scheme::mono(ty)
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
    sum_arities: HashMap<NominalId, usize>,
    boundaries: Vec<(SyntaxId, Type, bool)>,
    combined_error_defaults: Vec<Type>,
    raised_error_types: Vec<Type>,
    discarded_results: Vec<(Type, usize, usize)>,
    entry_natural_tail: Option<SyntaxId>,
    entry_boundary: Option<SyntaxId>,
    raise_payload: Option<SyntaxId>,
    saw_result: bool,
    start: usize,
    literals: Vec<(Type, bool, usize, usize)>,
    active: Option<std::ops::Range<usize>>,
    empty_sequences: Vec<(Type, usize, usize)>,
    explicit_variables: HashSet<u32>,
    qualifiers: &'r std::collections::BTreeMap<usize, Vec<(String, String)>>,
}

impl Engine<'_> {
    fn syntax_id(&self, expr: &Expr) -> SyntaxId {
        self.resolver.syntax_id(expr).expect("indexed expression")
    }
}

fn visit_inference_exprs(p: &Program, mut visit: impl FnMut(&Expr) -> R<()>) -> R<()> {
    fn walk(e: &Expr, visit: &mut impl FnMut(&Expr) -> R<()>) -> R<()> {
        visit(e)?;
        e.try_for_each_child(|child| walk(child, visit))
    }

    for (body, _) in p.defs.iter().filter_map(Stmt::body_and_arity) {
        walk(body, &mut visit)?;
    }
    if let Some(e) = &p.entry {
        walk(e, &mut visit)?;
    }
    Ok(())
}
fn find_entry_natural_tail(p: &Program, resolver: &Resolution) -> Option<SyntaxId> {
    let Expr::Action(_, Some(tail)) = p.entry.as_ref()? else {
        return None;
    };
    let (Expr::Fallible(_, Some(final_)) | Expr::FallibleActionBody(_, Some(final_))) =
        tail.as_ref()
    else {
        return None;
    };
    let nonnegative = match final_.as_ref() {
        Expr::Int(x) => *x >= 0,
        Expr::Num(raw) => !raw.is_empty() && raw.bytes().all(|b| b.is_ascii_digit()),
        _ => false,
    };
    nonnegative.then(|| resolver.syntax_id(final_)).flatten()
}
pub fn infer(p: &Program, resolver: &Resolution) -> R<InferResult> {
    let compatible = resolver.for_program(p).ok_or_else(|| InferError {
        kind: InferErrorKind::TypeError,
        start: 0,
        end: 0,
        message: "resolution metadata belongs to a different syntax tree".into(),
        expected: None,
        actual: None,
        constraint: None,
    })?;
    infer_live(p, compatible.as_ref())
}

/// The compiler pipeline borrows one unchanged program across its stages.
pub(crate) fn infer_live(p: &Program, resolver: &Resolution) -> R<InferResult> {
    let mut e = Engine {
        cx: InferCtx::new(),
        result: InferResult {
            syntax: resolver.syntax.clone(),
            program_shape: resolver.program_shape.clone(),
            ..InferResult::default()
        },
        resolver,
        schemas: HashMap::new(),
        schema_scopes: vec![HashMap::new()],
        sums: HashMap::new(),
        sum_arities: HashMap::new(),
        boundaries: vec![],
        combined_error_defaults: vec![],
        raised_error_types: vec![],
        discarded_results: vec![],
        entry_natural_tail: find_entry_natural_tail(p, resolver),
        entry_boundary: p.entry.as_ref().and_then(|e| {
            if let Expr::Action(_, Some(tail)) = e {
                matches!(
                    tail.as_ref(),
                    Expr::Fallible(..) | Expr::FallibleActionBody(..)
                )
                .then(|| resolver.syntax_id(tail))
                .flatten()
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
    e.default_combined_errors(0, std::iter::empty(), &env)?;
    if e.saw_result {
        for (t, start, end) in &e.discarded_results {
            if matches!(e.cx.resolve(t), Type::Result(..)) {
                return Err(InferError{kind:InferErrorKind::TypeError,start:*start,end:*end,expected:None,actual:None,constraint:None,message:"result-valued statement would discard a failure; propagate, recover, or bind its result".into()});
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
                        constraint: None,
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
                constraint: None,
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
