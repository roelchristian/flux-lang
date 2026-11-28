//! Source AST, structural child traversal and linkable module metadata.

use super::{Origin, TypeAnnotation};
use std::{
    collections::{BTreeMap, HashSet},
    ops::Range,
};

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub defs: Vec<Stmt>,
    pub imports: Vec<String>,
    pub entry: Option<Expr>,
    /// Zero-based byte offset of the entry’s opening `!`, if any.
    pub entry_start: Option<usize>,
    /// Explicit (class, type variable) qualifiers by declaration byte offset.
    pub constraints: BTreeMap<usize, Vec<(String, String)>>,
    pub exports: Vec<String>,
    pub(super) origins: Vec<Origin>,
    pub(super) members: Vec<ModuleMember>,
    pub(super) formatting_applications: Vec<FormattingApplication>,
}
/// Original source ranges for one ordinary prefix or tacit application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormattingApplication {
    /// From the operator through its final operand, excluding outer grouping.
    pub span: Range<usize>,
    pub operator: Range<usize>,
    /// Operand ranges include any explicit grouping and exclude surrounding trivia.
    pub operands: Vec<Range<usize>>,
    pub tacit: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Def {
        name: String,
        params: Vec<String>,
        param_types: Vec<Option<TypeAnnotation>>,
        result_type: Option<TypeAnnotation>,
        body: Expr,
        start: usize,
    },
    Sum {
        name: String,
        type_params: Vec<String>,
        variants: Vec<Variant>,
        start: usize,
    },
    Schema {
        name: String,
        fields: Vec<(String, TypeAnnotation)>,
        start: usize,
    },
    Effect {
        name: Option<String>,
        body: Expr,
        start: usize,
    },
}
#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub name: String,
    pub payload: Vec<TypeAnnotation>,
    pub start: usize,
}
#[derive(Debug, Clone, PartialEq)]
pub struct RecoveryCase {
    pub pattern: RecoveryPattern,
    pub body: Expr,
    pub start: usize,
}
#[derive(Debug, Clone, PartialEq, Hash)]
pub enum RecoveryPattern {
    Wildcard,
    Bind(String),
    Constructor(String, Vec<String>),
}
#[derive(Debug, Clone, PartialEq)]
pub struct MatchCase {
    pub pattern: MatchPattern,
    pub body: Expr,
    pub start: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MatchPattern {
    pub span: Range<usize>,
    pub kind: MatchPatternKind,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MatchPatternKind {
    Wildcard,
    Bind(String),
    Constructor(String, Vec<MatchPattern>),
    Tuple(Vec<MatchPattern>),
    Record(Vec<(String, MatchPattern)>),
    Optional(Option<Box<MatchPattern>>),
    Bool(bool),
    Unit,
}

/// Postorder syntax identity, scoped to one complete Program. Independent
/// parses and clones of the same syntax have identical IDs. Structural
/// edits or changes to the order of linked modules may renumber them.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct SyntaxId(pub usize);

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Int(i128),
    Str(String),
    Bool(bool),
    Unit,
    Name(String),
    Call(String, Vec<Expr>),
    Ref(String),
    Tuple(Vec<Expr>),
    Array(Vec<Expr>),
    Cases(Vec<(Expr, Expr)>, Box<Expr>),
    Lambda(Vec<String>, Box<Expr>),
    TypedLambda(Vec<String>, Vec<Option<TypeAnnotation>>, Box<Expr>),
    Pipeline(Vec<Expr>),
    Record(Vec<(String, Expr)>),
    Block(Vec<Stmt>, Option<Box<Expr>>),
    Action(Vec<Stmt>, Option<Box<Expr>>),
    Fallible(Vec<Stmt>, Option<Box<Expr>>),
    /// Fallible region inside a combined action; its unresolved error defaults to unit.
    FallibleActionBody(Vec<Stmt>, Option<Box<Expr>>),
    Raise(Box<Expr>),
    Propagate(Box<Expr>),
    /// Immediate execution statement in a combined action, checked before elaboration.
    AutoPropagate(Box<Expr>),
    Recover(Box<Expr>, Vec<RecoveryCase>),
    Match(Box<Expr>, Vec<MatchCase>),
    Num(String),
    Char(char),
    NamedRecord(String, Vec<(String, Expr)>),
}

impl Stmt {
    pub(crate) fn start(&self) -> usize {
        match self {
            Self::Def { start, .. }
            | Self::Effect { start, .. }
            | Self::Schema { start, .. }
            | Self::Sum { start, .. } => *start,
        }
    }

    pub(crate) fn body_and_arity(&self) -> Option<(&Expr, usize)> {
        match self {
            Self::Def { params, body, .. } => Some((body, params.len())),
            Self::Effect { body, .. } => Some((body, 0)),
            Self::Schema { .. } | Self::Sum { .. } => None,
        }
    }
}

impl Expr {
    // Structural children in parser origin order. Not an evaluation walk:
    // all branches and latent bodies are included, even when never executed.
    pub(crate) fn try_for_each_child<'a, E>(
        &'a self,
        mut visit: impl FnMut(&'a Expr) -> Result<(), E>,
    ) -> Result<(), E> {
        match self {
            Self::Call(_, es) | Self::Tuple(es) | Self::Array(es) | Self::Pipeline(es) => {
                es.iter().try_for_each(visit)
            }
            Self::Cases(pairs, fallback) => {
                for (cond, branch) in pairs {
                    visit(cond)?;
                    visit(branch)?;
                }
                visit(fallback)
            }
            Self::Lambda(_, body)
            | Self::TypedLambda(_, _, body)
            | Self::Raise(body)
            | Self::Propagate(body)
            | Self::AutoPropagate(body) => visit(body),
            Self::Record(fields) | Self::NamedRecord(_, fields) => {
                fields.iter().try_for_each(|(_, e)| visit(e))
            }
            Self::Recover(body, arms) => {
                visit(body)?;
                arms.iter().try_for_each(|a| visit(&a.body))
            }
            Self::Match(body, arms) => {
                visit(body)?;
                arms.iter().try_for_each(|a| visit(&a.body))
            }
            Self::Block(stmts, tail)
            | Self::Action(stmts, tail)
            | Self::Fallible(stmts, tail)
            | Self::FallibleActionBody(stmts, tail) => {
                for (body, _) in stmts.iter().filter_map(Stmt::body_and_arity) {
                    visit(body)?;
                }
                tail.iter().try_for_each(|e| visit(e))
            }
            Self::Int(_)
            | Self::Str(_)
            | Self::Bool(_)
            | Self::Char(_)
            | Self::Unit
            | Self::Num(_)
            | Self::Name(_)
            | Self::Ref(_) => Ok(()),
        }
    }

    pub(crate) fn for_each_child<'a>(&'a self, mut visit: impl FnMut(&'a Expr)) {
        self.try_for_each_child(|e| {
            visit(e);
            Ok::<(), std::convert::Infallible>(())
        })
        .unwrap();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemberKind {
    Callable(usize),
    Value,
    Record,
    Sum,
    Constructor(usize),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleMember {
    pub name: String,
    pub linked: String,
    pub kind: MemberKind,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedModule {
    pub path: String,
    pub members: Vec<ModuleMember>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportDecl {
    pub path: String,
    pub alias: String,
    pub start: usize,
}

impl Program {
    /// Available only for parses that explicitly requested layout metadata.
    pub fn formatting_applications(&self) -> &[FormattingApplication] {
        &self.formatting_applications
    }

    pub fn module_members(&self) -> Vec<ModuleMember> {
        self.members.clone()
    }
    /// Prepend declaration-only, independently parsed dependencies in dependency
    /// order. Their globally rebased expression origins are retained.
    pub fn merge_libraries(&mut self, libraries: impl IntoIterator<Item = Program>) {
        let mut defs = Vec::new();
        let mut origins = Vec::new();
        let mut imports = Vec::new();
        let mut seen = HashSet::new();
        for mut lib in libraries {
            assert!(
                lib.entry.is_none(),
                "cannot link a script entry as a library"
            );
            defs.append(&mut lib.defs);
            origins.append(&mut lib.origins);
            self.constraints.append(&mut lib.constraints);
            for x in lib.imports {
                if seen.insert(x.clone()) {
                    imports.push(x);
                }
            }
        }
        defs.append(&mut self.defs);
        self.defs = defs;
        origins.append(&mut self.origins);
        self.origins = origins;
        for x in self.imports.drain(..) {
            if seen.insert(x.clone()) {
                imports.push(x);
            }
        }
        self.imports = imports;
    }
}
