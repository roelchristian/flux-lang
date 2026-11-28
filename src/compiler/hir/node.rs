use super::{CoreOp, LiftMode, Span};
use crate::{
    core::operation::Operation,
    resolve::SymbolId,
    types::{NominalId, Scheme, Type},
};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ExprId(pub usize);

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
    /// Stable position in the source Program syntax index, independent of
    /// the HIR arena's allocation or traversal.
    pub source_id: crate::parser::SyntaxId,
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
pub struct ValuePattern {
    pub span: Span,
    pub kind: ValuePatternKind,
}

#[derive(Clone, Debug)]
pub enum ValuePatternKind {
    Wildcard,
    Bind(SymbolId),
    Constructor(SymbolId, Vec<ValuePattern>),
    Tuple(Vec<ValuePattern>),
    Record(Vec<(Box<str>, ValuePattern)>),
    Optional(Option<Box<ValuePattern>>),
    Bool(bool),
    Unit,
}

#[derive(Clone, Debug)]
pub struct MatchArm {
    pub pattern: ValuePattern,
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
    Match {
        value: ExprId,
        arms: Vec<MatchArm>,
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
