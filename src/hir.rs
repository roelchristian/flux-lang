//! Typed, owned execution IR. Surface AST addresses are used only during lowering.
use crate::{
    infer::{InferResult, Operation},
    parser::{Expr, Program, RecoveryPattern as SyntaxPattern, Stmt},
    resolve::{Resolution, SymbolId},
    types::{NominalId, Scheme, Type},
};
use std::{collections::HashMap, fmt, rc::Rc};

// AST currently retains nearest declaration/entry offsets, not token end spans.
pub use crate::diagnostic::Span;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ExprId(pub usize);
#[repr(usize)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum CoreOp {
    Add,
    Sub,
    Mul,
    Div,
    Quotient,
    Remainder,
    Pow,
    Negate,
    Abs,
    Equal,
    NotEqual,
    EqualWhole,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Not,
    Range,
    Sum,
    Product,
    Count,
    TableRows,
    TableFrom,
    Rows,
    Filter,
    Group,
    Aggregate,
    Extend,
    SortAsc,
    SortDesc,
    Concat,
    Distinct,
    Lookup,
    Zip,
    Cartesian,
    Take,
    Drop,
    Slice,
    Reverse,
    Flatten,
    Transpose,
    Variance,
    Percentile,
    Min,
    Max,
    All,
    Any,
    Fold,
    Scan,
    CsvRead,
    CsvWrite,
    Mean,
    Median,
    StdDev,
    Apply,
    ApplyTuple,
    Map,
    Execute,
    IoRead,
    IoWrite,
    IoExists,
    IoPrint,
    CheckedDiv,
    Quantize,
    Sqrt,
    Present,
    Absent,
    Has,
    Unwrap,
    Default,
    ToNatural,
    ToInteger,
    ToRational,
    ToDecimal,
    ToReal,
}

/// Static shape execution plan for direct pointwise operations. Pairwise
/// lifting includes a runtime equality check on the two sequence lengths.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiftMode {
    Scalar,
    LeftSequence,
    RightSequence,
    Pairwise,
}
impl LiftMode {
    pub(crate) fn from_shapes(left: bool, right: bool) -> Self {
        match (left, right) {
            (false, false) => Self::Scalar,
            (true, false) => Self::LeftSequence,
            (false, true) => Self::RightSequence,
            (true, true) => Self::Pairwise,
        }
    }
}

impl CoreOp {
    pub(crate) fn lift_arity(self) -> Option<usize> {
        crate::builtins::for_operation(self)
            .filter(|spec| spec.shape == crate::builtins::Shape::Elementwise)
            .map(|spec| spec.arity)
    }
    fn from_name(name: &str) -> Option<Self> {
        crate::builtins::lookup(name).and_then(|builtin| builtin.op)
    }
}

#[derive(Clone, Debug)]
pub enum Literal {
    Integer(i128),
    Numeric(Box<str>),
    Bool(bool),
    String(Box<str>),
    Char(char),
    Unit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TableSchema {
    Named(Vec<(String, Type)>),
    Positional(Vec<Type>),
}
#[derive(Clone, Debug)]
pub struct ExprNode {
    pub span: Span,
    pub ty: Type,
    pub table_schema: Option<Rc<TableSchema>>,
    /// Resolved pointwise plan for direct calls; checked callable instances supply indirect plans.
    pub lift_mode: Option<LiftMode>,
    /// Zonked callable signature from the checker at a direct named use.
    pub callee_type: Option<Type>,
    /// Checker evidence, when the inference engine recorded a resolved operation.
    pub evidence: Option<Operation>,
    pub kind: ExprKind,
}
#[derive(Clone, Debug)]
pub enum Target {
    Symbol(SymbolId),
    Builtin(CoreOp),
    Constructor(SymbolId),
    Projection(Box<str>),
}

#[derive(Clone, Debug)]
pub struct Definition {
    pub symbol: SymbolId,
    pub span: Span,
    pub scheme: Scheme,
    pub formals: Vec<SymbolId>,
    pub body: ExprId,
}

#[derive(Clone, Debug)]
pub enum Step {
    Define(Definition),
    Evaluate {
        binding: Option<SymbolId>,
        body: ExprId,
    },
}

#[derive(Clone, Debug)]
pub struct PipelineStage {
    /// Present only in subsequent stages; binds the result of the preceding stage.
    pub input: Option<SymbolId>,
    pub expr: ExprId,
}
/// A checker's lexical escape target, renumbered deterministically in the
/// owned HIR; no source-tree address is retained at execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct BoundaryId(pub usize);
#[derive(Clone, Debug)]
pub struct ConstructorDef {
    pub owner: NominalId,
    pub id: SymbolId,
    pub name: Rc<str>,
    pub payload: Vec<Type>,
}
#[derive(Clone, Debug)]
pub enum Pattern {
    Wildcard,
    Bind(SymbolId),
    Constructor {
        id: SymbolId,
        fields: Vec<Option<SymbolId>>,
    },
}
#[derive(Clone, Debug)]
pub struct RecoveryArm {
    pub pattern: Pattern,
    pub body: ExprId,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Literal(Literal),
    Load(SymbolId),
    Reference(Target),
    Call(Target, Vec<ExprId>),
    Tuple(Vec<ExprId>),
    Sequence(Vec<ExprId>),
    Record(Vec<(Box<str>, ExprId)>),
    Cases {
        branches: Vec<(ExprId, ExprId)>,
        otherwise: ExprId,
    },
    Closure {
        formals: Vec<SymbolId>,
        body: ExprId,
    },
    Pipeline(Vec<PipelineStage>),
    Fallible {
        boundary: BoundaryId,
        steps: Vec<Step>,
        tail: Option<ExprId>,
    },
    Raise {
        boundary: BoundaryId,
        payload: ExprId,
    },
    Propagate {
        boundary: BoundaryId,
        result: ExprId,
    },
    Recover {
        result: ExprId,
        arms: Vec<RecoveryArm>,
    },
    Block {
        steps: Vec<Step>,
        tail: Option<ExprId>,
        suspended: bool,
    },
}

#[derive(Clone, Debug)]
pub struct HirProgram {
    pub arena: Vec<ExprNode>,
    pub constructors: HashMap<SymbolId, ConstructorDef>,
    nominal_schemas: HashMap<NominalId, Rc<TableSchema>>,
    standard_constructors: HashMap<(String, String), SymbolId>,
    /// Dependency order. All function symbols in a group are introduced before any initializer.
    pub groups: Vec<Vec<Definition>>,
    pub entry: Option<ExprId>,
}
impl HirProgram {
    pub fn standard_constructor(&self, owner: &str, name: &str) -> Option<SymbolId> {
        self.standard_constructors
            .get(&(owner.to_owned(), name.to_owned()))
            .copied()
    }
    /// Layout witness for a checked callable instance, independent of row values.
    pub(crate) fn instance_schema(&self, ty: &Type) -> Option<Rc<TableSchema>> {
        match ty {
            Type::Action(out) | Type::Result(out, _) | Type::Fun(_, out) => {
                self.instance_schema(out)
            }
            Type::Table(row) => match row.as_ref() {
                Type::Record(row) if row.tail.is_none() => {
                    Some(Rc::new(TableSchema::Named(row.fields.clone())))
                }
                Type::Tuple(fields) => Some(Rc::new(TableSchema::Positional(fields.clone()))),
                Type::Nominal(id) => self.nominal_schemas.get(id).cloned(),
                _ => None,
            },
            _ => None,
        }
    }
    pub fn node(&self, id: ExprId) -> &ExprNode {
        &self.arena[id.0]
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BoundaryKind {
    /// This language feature still runs outside the typed execution backend.
    Unsupported,
    /// The resolver/typechecker omitted evidence for AST the backend does cover.
    MissingMetadata,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Boundary {
    pub kind: BoundaryKind,
    pub span: Span,
    pub message: String,
}
impl fmt::Display for Boundary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            BoundaryKind::Unsupported => {
                write!(f, "typed HIR migration boundary: {}", self.message)
            }
            BoundaryKind::MissingMetadata => {
                write!(f, "typed HIR metadata error: {}", self.message)
            }
        }
    }
}
impl std::error::Error for Boundary {}
impl Boundary {
    fn unsupported(span: Span, message: impl Into<String>) -> Self {
        Self {
            kind: BoundaryKind::Unsupported,
            span,
            message: message.into(),
        }
    }
    fn missing(span: Span, message: impl Into<String>) -> Self {
        Self {
            kind: BoundaryKind::MissingMetadata,
            span,
            message: message.into(),
        }
    }
    pub fn is_migration(&self) -> bool {
        self.kind == BoundaryKind::Unsupported
    }
}

fn csv_returns_table(ty: &Type) -> bool {
    match ty {
        Type::Fun(_, out) | Type::Action(out) | Type::Result(out, _) => csv_returns_table(out),
        Type::Table(_) => true,
        _ => false,
    }
}
struct Lower<'a> {
    inference: &'a InferResult,
    resolution: &'a Resolution,
    arena: Vec<ExprNode>,
    boundaries: HashMap<usize, BoundaryId>,
}

pub fn lower(
    program: &Program,
    inference: &InferResult,
    resolution: &Resolution,
) -> Result<HirProgram, Boundary> {
    let mut lower = Lower {
        inference,
        resolution,
        arena: Vec::new(),
        boundaries: HashMap::new(),
    };
    let mut groups = Vec::new();
    for group in &resolution.groups {
        let mut definitions = Vec::new();
        for &i in group {
            let d = program.defs.get(i).ok_or_else(|| {
                Boundary::missing(Span::at(0), "invalid top-level declaration index")
            })?;
            match d {
                Stmt::Def { .. } => definitions.push(lower.definition(d)?),
                Stmt::Schema { .. } | Stmt::Sum { .. } => {}
                Stmt::Effect { start, .. } => {
                    return Err(Boundary::unsupported(
                        Span::at(*start),
                        "top-level effect statement",
                    ))
                }
            }
        }
        groups.push(definitions);
    }
    let entry = program
        .entry
        .as_ref()
        .map(|e| lower.expr(e, Span::at(program.entry_start.unwrap_or(0))))
        .transpose()?;
    let constructors = inference
        .constructors
        .iter()
        .map(|(&id, c)| {
            let name = crate::visible_symbol(&resolution.symbol(id).name);
            (
                id,
                ConstructorDef {
                    owner: c.owner.clone(),
                    id: c.id,
                    name: Rc::from(name),
                    payload: c.payload.clone(),
                },
            )
        })
        .collect();
    Ok(HirProgram {
        arena: lower.arena,
        constructors,
        nominal_schemas: inference
            .schemas
            .iter()
            .map(|(id, row)| (id.clone(), Rc::new(TableSchema::Named(row.fields.clone()))))
            .collect(),
        standard_constructors: resolution
            .standard_constructors()
            .map(|(owner, name, id)| ((owner.to_owned(), name.to_owned()), id))
            .collect(),
        groups,
        entry,
    })
}

impl Lower<'_> {
    fn boundary(&mut self, e: &Expr, span: Span) -> Result<BoundaryId, Boundary> {
        let checked = self
            .inference
            .boundary_of(e)
            .ok_or_else(|| Boundary::missing(span, "checked caret boundary"))?;
        let next = BoundaryId(self.boundaries.len());
        Ok(*self.boundaries.entry(checked).or_insert(next))
    }
    fn steps(&mut self, stmts: &[Stmt]) -> Result<Vec<Step>, Boundary> {
        let mut steps = Vec::new();
        for s in stmts {
            match s {
                Stmt::Def { .. } => steps.push(Step::Define(self.definition(s)?)),
                Stmt::Effect { name, body, start } => {
                    let span = Span::at(*start);
                    let binding = if name.is_some() {
                        Some(
                            self.resolution
                                .declaration(s)
                                .ok_or_else(|| Boundary::missing(span, "effect binding symbol"))?,
                        )
                    } else {
                        None
                    };
                    steps.push(Step::Evaluate {
                        binding,
                        body: self.expr(body, span)?,
                    });
                }
                Stmt::Schema { .. } | Stmt::Sum { .. } => {}
            }
        }
        Ok(steps)
    }
    fn table_schema(&self, ty: &Type) -> Option<Rc<TableSchema>> {
        match ty {
            Type::Action(out) | Type::Fun(_, out) | Type::Result(out, _) => self.table_schema(out),
            Type::Table(row) => Some(Rc::new(match row.as_ref() {
                Type::Record(row) if row.tail.is_none() => TableSchema::Named(row.fields.clone()),
                Type::Tuple(fields) => TableSchema::Positional(fields.clone()),
                Type::Nominal(name) => {
                    TableSchema::Named(self.inference.nominal_schema(name)?.fields.clone())
                }
                _ => return None,
            })),
            _ => None,
        }
    }
    fn definition(&mut self, d: &Stmt) -> Result<Definition, Boundary> {
        let Stmt::Def { body, start, .. } = d else {
            unreachable!()
        };
        let span = Span::at(*start);
        let symbol = self
            .resolution
            .declaration(d)
            .ok_or_else(|| Boundary::missing(span, "definition symbol"))?;
        let scheme = self
            .inference
            .binding_type(symbol)
            .cloned()
            .ok_or_else(|| Boundary::missing(span, "definition type scheme"))?;
        let formals = self.resolution.parameters(d).to_vec();
        Ok(Definition {
            symbol,
            span,
            scheme,
            formals,
            body: self.expr(body, span)?,
        })
    }
    fn target(&self, e: &Expr, name: &str, span: Span) -> Result<Target, Boundary> {
        if let Some(id) = self.resolution.reference(e) {
            if self.inference.constructor(id).is_some() {
                return Ok(Target::Constructor(id));
            }
            if !matches!(
                self.resolution.symbol(id).kind,
                crate::resolve::SymbolKind::External
            ) {
                return Ok(Target::Symbol(id));
            }
        }
        if let Some(field) = name
            .strip_prefix('.')
            .filter(|f| !f.is_empty() && *f != ":" && *f != ".")
        {
            return Ok(Target::Projection(field.into()));
        }
        CoreOp::from_name(name).map(Target::Builtin).ok_or_else(|| {
            Boundary::unsupported(span, format!("builtin or library operation `{name}`"))
        })
    }
    fn list(&mut self, es: &[Expr], span: Span) -> Result<Vec<ExprId>, Boundary> {
        es.iter().map(|e| self.expr(e, span)).collect()
    }
    fn expr(&mut self, e: &Expr, span: Span) -> Result<ExprId, Boundary> {
        let span = self
            .resolution
            .span(e)
            .map(|r| Span::new(r.start, r.end))
            .or_else(|| self.resolution.offset(e).map(Span::at))
            .unwrap_or(span);
        use ExprKind::*;
        let kind = match e {
            Expr::Int(n) => Literal(self::Literal::Integer(*n)),
            Expr::Num(n) => Literal(self::Literal::Numeric(n.as_str().into())),
            Expr::Bool(b) => Literal(self::Literal::Bool(*b)),
            Expr::Str(s) => Literal(self::Literal::String(s.as_str().into())),
            Expr::Char(c) => Literal(self::Literal::Char(*c)),
            Expr::Unit => Literal(self::Literal::Unit),
            Expr::Name(s) if s == "?0" => Call(Target::Builtin(CoreOp::Absent), Vec::new()),
            Expr::Name(_) => {
                let id = self
                    .resolution
                    .reference(e)
                    .ok_or_else(|| Boundary::missing(span, "value reference"))?;
                if self.inference.constructor(id).is_some() {
                    Call(Target::Constructor(id), Vec::new())
                } else {
                    Load(id)
                }
            }
            Expr::Ref(s) => {
                let target = self.target(e, s, span)?;
                if matches!(
                    target,
                    Target::Builtin(
                        CoreOp::Apply | CoreOp::ApplyTuple | CoreOp::Execute | CoreOp::Default
                    )
                ) {
                    return Err(Boundary::unsupported(
                        span,
                        format!("first-class reference to special syntax `{s}`"),
                    ));
                }
                Reference(target)
            }
            Expr::Call(s, a) => Call(self.target(e, s, span)?, self.list(a, span)?),
            Expr::Tuple(es) => Tuple(self.list(es, span)?),
            Expr::Array(es) => Sequence(self.list(es, span)?),
            Expr::Cases(branches, otherwise) => {
                let branches = branches
                    .iter()
                    .map(|(a, b)| Ok((self.expr(a, span)?, self.expr(b, span)?)))
                    .collect::<Result<_, _>>()?;
                Cases {
                    branches,
                    otherwise: self.expr(otherwise, span)?,
                }
            }
            Expr::Lambda(formals, body) | Expr::TypedLambda(formals, _, body) => {
                let ids = self.resolution.lambda_parameters(e).to_vec();
                if formals.len() != ids.len() {
                    return Err(Boundary::missing(span, "lambda parameter symbols"));
                }
                Closure {
                    formals: ids,
                    body: self.expr(body, span)?,
                }
            }
            Expr::Pipeline(es) => {
                let mut stages = Vec::new();
                for (i, x) in es.iter().enumerate() {
                    let input = if i == 0 {
                        None
                    } else {
                        Some(
                            self.resolution
                                .placeholder(x)
                                .ok_or_else(|| Boundary::missing(span, "pipeline stage symbol"))?,
                        )
                    };
                    stages.push(PipelineStage {
                        input,
                        expr: self.expr(x, span)?,
                    });
                }
                Pipeline(stages)
            }
            Expr::Block(stmts, tail) | Expr::Action(stmts, tail) => Block {
                steps: self.steps(stmts)?,
                tail: tail.as_deref().map(|v| self.expr(v, span)).transpose()?,
                suspended: matches!(e, Expr::Action(..)),
            },
            Expr::Fallible(stmts, tail) => Fallible {
                boundary: self.boundary(e, span)?,
                steps: self.steps(stmts)?,
                tail: tail.as_deref().map(|v| self.expr(v, span)).transpose()?,
            },
            Expr::Raise(payload) => Raise {
                boundary: self.boundary(e, span)?,
                payload: self.expr(payload, span)?,
            },
            Expr::Propagate(result) => Propagate {
                boundary: self.boundary(e, span)?,
                result: self.expr(result, span)?,
            },
            Expr::Recover(result, cases) => {
                let result = self.expr(result, span)?;
                let mut arms = Vec::new();
                for case in cases {
                    let span = Span::at(case.start);
                    let mut ids = self.resolution.recovery_bindings(case).iter().copied();
                    let missing = || Boundary::missing(span, "recovery binder");
                    let pattern = match &case.pattern {
                        SyntaxPattern::Wildcard => Pattern::Wildcard,
                        SyntaxPattern::Bind(_) => Pattern::Bind(ids.next().ok_or_else(missing)?),
                        SyntaxPattern::Constructor(_, names) => {
                            let id =
                                self.resolution.recovery_constructor(case).ok_or_else(|| {
                                    Boundary::missing(span, "recovery constructor identity")
                                })?;
                            let mut fields = Vec::new();
                            for n in names {
                                fields.push(if n == "_" {
                                    None
                                } else {
                                    Some(ids.next().ok_or_else(missing)?)
                                });
                            }
                            Pattern::Constructor { id, fields }
                        }
                    };
                    if ids.next().is_some() {
                        return Err(Boundary::missing(span, "surplus recovery binder"));
                    }
                    arms.push(RecoveryArm {
                        pattern,
                        body: self.expr(&case.body, span)?,
                        span,
                    });
                }
                Recover { result, arms }
            }
            Expr::Record(fields) | Expr::NamedRecord(_, fields) => Record(
                fields
                    .iter()
                    .map(|(k, v)| Ok((k.as_str().into(), self.expr(v, span)?)))
                    .collect::<Result<_, _>>()?,
            ),
        };
        let ty = self
            .inference
            .type_of(e)
            .cloned()
            .ok_or_else(|| Boundary::missing(span, "inferred expression type"))?;
        let table_schema = self.table_schema(&ty);
        if matches!(
            &kind,
            Call(
                Target::Builtin(
                    CoreOp::TableRows | CoreOp::TableFrom | CoreOp::Aggregate | CoreOp::Extend
                ),
                _
            )
        ) && table_schema.is_none()
        {
            return Err(Boundary::unsupported(
                span,
                "open table result schema does not have execution layout",
            ));
        }
        if matches!(
            &kind,
            Call(Target::Builtin(CoreOp::CsvRead), _) | Reference(Target::Builtin(CoreOp::CsvRead))
        ) && csv_returns_table(&ty)
            && table_schema.is_none()
        {
            return Err(Boundary::unsupported(
                span,
                "typed CSV read requires a closed row schema",
            ));
        }
        let lift_mode = if let Call(Target::Builtin(op), args) = &kind {
            if let Some(arity) = op.lift_arity() {
                if args.len() != arity {
                    return Err(Boundary::missing(span, "invalid direct lifting arity"));
                }
                let sequence = |i: usize| {
                    args.get(i)
                        .is_some_and(|id| matches!(self.arena[id.0].ty, Type::Seq(_)))
                };
                Some(LiftMode::from_shapes(sequence(0), sequence(1)))
            } else {
                None
            }
        } else {
            None
        };
        let evidence = self.inference.operation_of(e).cloned();
        let id = ExprId(self.arena.len());
        self.arena.push(ExprNode {
            span,
            ty,
            table_schema,
            lift_mode,
            callee_type: self.inference.callee_type(e).cloned(),
            evidence,
            kind,
        });
        Ok(id)
    }
}
