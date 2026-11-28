//! Canonical Flux types and type/row unification.
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::ops::ControlFlow::{self, Break, Continue};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Prim {
    N,
    Z,
    Q,
    D,
    R,
    B,
    S,
    C,
    Unit,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Type {
    Var(u32),
    Prim(Prim),
    Seq(Box<Type>),
    Tuple(Vec<Type>),
    Dict(Box<Type>, Box<Type>),
    Optional(Box<Type>),
    Result(Box<Type>, Box<Type>),
    /// Internal type for an expression that does not return normally.
    Never,
    Action(Box<Type>),
    Fun(Box<Type>, Box<Type>),
    Table(Box<Type>),
    Nominal(NominalId),
    /// A saturated application of a parameterized nominal sum or built-in array.
    Applied(NominalId, Vec<Type>),
    Record(Row),
}

/// Identity and user-facing spelling of a lexical schema declaration.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NominalId {
    pub id: usize,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Row {
    pub fields: Vec<(String, Type)>,
    pub tail: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Class {
    Num,
    Eq,
    Ord,
    Hash,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Constraint {
    pub class: Class,
    pub ty: Type,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scheme {
    pub vars: Vec<u32>,
    pub ty: Type,
    pub constraints: Vec<Constraint>,
    /// Quantified row IDs, with the field labels excluded from each tail.
    pub row_lacks: BTreeMap<u32, BTreeSet<String>>,
}

impl Scheme {
    pub fn mono(ty: Type) -> Self {
        Self {
            vars: vec![],
            ty,
            constraints: vec![],
            row_lacks: BTreeMap::new(),
        }
    }
}

impl Type {
    /// Canonical rank-independent dense array type.
    pub fn dense_array(element: Type) -> Self {
        Self::Applied(
            NominalId {
                id: usize::MAX,
                name: "array".into(),
            },
            vec![element],
        )
    }

    pub fn array_element(&self) -> Option<&Type> {
        match self {
            Self::Applied(id, args)
                if id.id == usize::MAX && id.name == "array" && args.len() == 1 =>
            {
                args.first()
            }
            _ => None,
        }
    }

    /// Select the first matching node in preorder, including record fields.
    pub(crate) fn find_map<'a, B>(
        &'a self,
        mut select: impl FnMut(&'a Type) -> Option<B>,
    ) -> Option<B> {
        self.walk(&mut |t| select(t).map_or(Continue(()), Break))
            .break_value()
    }

    fn walk<'a, B>(&'a self, visit: &mut impl FnMut(&'a Type) -> ControlFlow<B>) -> ControlFlow<B> {
        visit(self)?;
        match self {
            Self::Seq(t) | Self::Optional(t) | Self::Action(t) | Self::Table(t) => t.walk(visit),
            Self::Dict(a, b) | Self::Fun(a, b) | Self::Result(a, b) => {
                a.walk(visit)?;
                b.walk(visit)
            }
            Self::Tuple(ts) | Self::Applied(_, ts) => ts.iter().try_for_each(|t| t.walk(visit)),
            Self::Record(row) => row.fields.iter().try_for_each(|(_, t)| t.walk(visit)),
            Self::Var(_) | Self::Prim(_) | Self::Nominal(_) | Self::Never => Continue(()),
        }
    }

    fn visit(&self, mut visit: impl FnMut(&Type)) {
        let _: ControlFlow<()> = self.walk(&mut |t| {
            visit(t);
            Continue(())
        });
    }

    fn try_visit<E>(&self, mut visit: impl FnMut(&Type) -> Result<(), E>) -> Result<(), E> {
        self.walk(&mut |t| match visit(t) {
            Ok(()) => Continue(()),
            Err(error) => Break(error),
        })
        .break_value()
        .map_or(Ok(()), Err)
    }
}

impl fmt::Display for Prim {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::N => "n",
            Self::Z => "z",
            Self::Q => "q",
            Self::D => "d",
            Self::R => "r",
            Self::B => "b",
            Self::S => "s",
            Self::C => "c",
            Self::Unit => "()",
        })
    }
}
impl fmt::Display for Class {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Num => "num",
            Self::Eq => "eq",
            Self::Ord => "ord",
            Self::Hash => "hash",
        })
    }
}
impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn operand(t: &Type, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            if matches!(t, Type::Fun(..)) {
                write!(f, "({t})")
            } else {
                write!(f, "{t}")
            }
        }
        fn result_operand(t: &Type, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            if matches!(t, Type::Fun(..) | Type::Result(..)) {
                write!(f, "({t})")
            } else {
                write!(f, "{t}")
            }
        }
        match self {
            Self::Var(v) => write!(f, "~t{v}"),
            Self::Prim(p) => write!(f, "{p}"),
            Self::Seq(t) => write!(f, "[{t}]"),
            Self::Tuple(ts) => {
                f.write_str("(")?;
                for (i, t) in ts.iter().enumerate() {
                    if i != 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{t}")?;
                }
                if ts.len() == 1 {
                    f.write_str(",")?;
                }
                f.write_str(")")
            }
            Self::Dict(k, v) => {
                f.write_str("[")?;
                operand(k, f)?;
                f.write_str(":")?;
                operand(v, f)?;
                f.write_str("]")
            }
            Self::Optional(t) => {
                f.write_str("?")?;
                result_operand(t, f)
            }
            Self::Result(ok, err) => {
                result_operand(ok, f)?;
                f.write_str("^")?;
                result_operand(err, f)
            }
            Self::Never => f.write_str("<never>"),
            Self::Action(t) => {
                f.write_str("!")?;
                result_operand(t, f)
            }
            Self::Fun(a, b) => {
                operand(a, f)?;
                write!(f, "->{b}")
            }
            Self::Table(t) => {
                f.write_str("#")?;
                result_operand(t, f)
            }
            Self::Nominal(n) => f.write_str(crate::visible_symbol(&n.name)),
            Self::Applied(n, args) => {
                write!(f, "{}[", crate::visible_symbol(&n.name))?;
                for (i, t) in args.iter().enumerate() {
                    if i != 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{t}")?;
                }
                f.write_str("]")
            }
            Self::Record(row) => write!(f, "{row}"),
        }
    }
}
impl fmt::Display for Row {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(".{")?;
        for (i, (n, t)) in self.fields.iter().enumerate() {
            if i != 0 {
                f.write_str(" ")?;
            }
            write!(f, "{n}:{t};")?;
        }
        if let Some(id) = self.tail {
            if !self.fields.is_empty() {
                f.write_str(" ")?;
            }
            write!(f, "| ~r{id}")?;
        }
        f.write_str("}")
    }
}
impl fmt::Display for Constraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.class, self.ty)
    }
}
impl fmt::Display for Scheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.ty)?;
        for (i, c) in self.constraints.iter().enumerate() {
            if i == 0 {
                f.write_str(" | ")?;
            } else {
                f.write_str(", ")?;
            }
            write!(f, "{c}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct InferCtx {
    next: u32,
    types: BTreeMap<u32, Type>,
    type_ids: BTreeSet<u32>,
    row_ids: BTreeSet<u32>,
    rigid: BTreeSet<u32>,
    rows: BTreeMap<u32, Row>,
    lacks: BTreeMap<u32, BTreeSet<String>>,
    pending: Vec<Constraint>,
}

type R<T> = Result<T, String>;

fn free(t: &Type, out: &mut BTreeSet<u32>) {
    t.visit(|t| {
        if let Type::Var(id) | Type::Record(Row { tail: Some(id), .. }) = t {
            out.insert(*id);
        }
    });
}

fn row_labels(t: &Type, out: &mut BTreeMap<u32, BTreeSet<String>>) {
    t.visit(|t| {
        if let Type::Record(Row {
            fields,
            tail: Some(id),
        }) = t
        {
            out.entry(*id)
                .or_default()
                .extend(fields.iter().map(|(name, _)| name.clone()));
        }
    });
}

fn remap(t: &Type, ids: &BTreeMap<u32, u32>) -> Type {
    map_type(
        t,
        &|v| Type::Var(ids.get(&v).copied().unwrap_or(v)),
        &|row| {
            Type::Record(Row {
                fields: row
                    .fields
                    .iter()
                    .map(|(name, t)| (name.clone(), remap(t, ids)))
                    .collect(),
                tail: row.tail.map(|id| ids.get(&id).copied().unwrap_or(id)),
            })
        },
    )
}
fn map_type(t: &Type, var: &impl Fn(u32) -> Type, row: &impl Fn(&Row) -> Type) -> Type {
    let inner = |t: &Type| Box::new(map_type(t, var, row));
    match t {
        Type::Var(v) => var(*v),
        Type::Record(r) => row(r),
        Type::Prim(_) | Type::Nominal(_) | Type::Never => t.clone(),
        Type::Seq(x) => Type::Seq(inner(x)),
        Type::Optional(x) => Type::Optional(inner(x)),
        Type::Result(x, y) => Type::Result(inner(x), inner(y)),
        Type::Action(x) => Type::Action(inner(x)),
        Type::Table(x) => Type::Table(inner(x)),
        Type::Dict(a, b) => Type::Dict(inner(a), inner(b)),
        Type::Fun(a, b) => Type::Fun(inner(a), inner(b)),
        Type::Tuple(ts) => Type::Tuple(ts.iter().map(|x| map_type(x, var, row)).collect()),
        Type::Applied(n, ts) => Type::Applied(
            n.clone(),
            ts.iter().map(|x| map_type(x, var, row)).collect(),
        ),
    }
}

impl InferCtx {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn fresh(&mut self) -> Type {
        let id = self.fresh_id();
        self.type_ids.insert(id);
        Type::Var(id)
    }
    /// Abstract variable from an explicitly quantified signature.
    pub fn fresh_rigid(&mut self) -> Type {
        let Type::Var(id) = self.fresh() else {
            unreachable!()
        };
        self.mark_rigid(id);
        Type::Var(id)
    }

    pub fn mark_rigid(&mut self, id: u32) {
        self.next = self.next.max(
            id.checked_add(1)
                .expect("type variable identifier space exhausted"),
        );
        self.rigid.insert(id);
    }

    pub fn unmark_rigid(&mut self, id: u32) {
        self.rigid.remove(&id);
    }

    pub fn free_vars(&self, t: &Type) -> BTreeSet<u32> {
        let mut vars = BTreeSet::new();
        free(&self.resolve(t), &mut vars);
        vars
    }

    /// Look for skolems in pre-existing bindings after checking a nested signature.
    /// Quantified variables in an existing scheme are abstract, not outer captures.
    pub fn rigid_escapes(&self, ids: &[u32], outer: &[Scheme]) -> bool {
        outer.iter().any(|s| {
            let vars = self.free_vars(&s.ty);
            let leaked = |v: &u32| !s.vars.contains(v) && ids.contains(v);
            vars.iter().any(leaked)
                || s.constraints
                    .iter()
                    .any(|c| self.free_vars(&c.ty).iter().any(leaked))
        })
    }

    pub fn fresh_row(&mut self) -> u32 {
        let id = self.fresh_id();
        self.row_ids.insert(id);
        id
    }
    fn fresh_id(&mut self) -> u32 {
        let id = self.next;
        self.next = self
            .next
            .checked_add(1)
            .expect("type variable identifier space exhausted");
        id
    }
    pub fn resolve(&self, ty: &Type) -> Type {
        map_type(
            ty,
            &|v| self.types.get(&v).map_or(Type::Var(v), |t| self.resolve(t)),
            &|row| Type::Record(self.resolve_row(row)),
        )
    }
    fn resolve_row(&self, row: &Row) -> Row {
        let mut fields = row
            .fields
            .iter()
            .map(|(k, t)| (k.clone(), self.resolve(t)))
            .collect::<Vec<_>>();
        let mut tail = row.tail;
        while let Some(next) = tail.and_then(|id| self.rows.get(&id)) {
            fields.extend(
                next.fields
                    .iter()
                    .map(|(k, t)| (k.clone(), self.resolve(t))),
            );
            tail = next.tail;
        }
        Row { fields, tail }
    }
    pub fn unify(&mut self, a: &Type, b: &Type) -> R<()> {
        let before = self.clone();
        let result = self.unify_committing(a, b);
        if result.is_err() {
            *self = before;
        }
        result
    }
    /// Fast path when the caller abandons inference entirely on failure.
    /// A failed call may leave partial bindings; alternative branches must use `unify`.
    pub(crate) fn unify_committing(&mut self, a: &Type, b: &Type) -> R<()> {
        self.register(a)?;
        self.register(b)?;
        self.unify_inner(a, b)?;
        self.check_pending()
    }

    fn register_kind(&mut self, id: u32, row: bool) -> R<()> {
        let (same, other) = if row {
            (&mut self.row_ids, &self.type_ids)
        } else {
            (&mut self.type_ids, &self.row_ids)
        };
        if other.contains(&id) {
            return Err(format!("variable {id} cannot have both type and row kind"));
        }
        self.next = self.next.max(
            id.checked_add(1)
                .ok_or("type variable identifier space exhausted")?,
        );
        same.insert(id);
        Ok(())
    }

    pub(crate) fn register(&mut self, t: &Type) -> R<()> {
        t.try_visit(|t| match t {
            Type::Var(id) => self.register_kind(*id, false),
            Type::Record(row) => {
                if let Some(id) = row.tail {
                    self.register_kind(id, true)?;
                }
                self.register_row(&self.resolve_row(row))
            }
            _ => Ok(()),
        })
    }
    fn add_lacks(&mut self, id: u32, labels: BTreeSet<String>) -> R<()> {
        self.register_kind(id, true)?;
        let known = self.lacks.entry(id).or_default();
        if self.rigid.contains(&id) {
            if let Some(label) = labels.difference(known).next() {
                return Err(format!(
                    "rigid row tail ~r{id} is not known to lack field {label}"
                ));
            }
        } else {
            known.extend(labels);
        }
        Ok(())
    }
    fn register_row(&mut self, row: &Row) -> R<()> {
        let mut names = BTreeSet::new();
        for (n, _) in &row.fields {
            if !names.insert(n.clone()) {
                return Err(format!("duplicate row field {n}"));
            }
        }
        if let Some(id) = row.tail {
            self.add_lacks(id, names)?;
        }
        Ok(())
    }
    fn unify_inner(&mut self, a: &Type, b: &Type) -> R<()> {
        let a = self.resolve(a);
        let b = self.resolve(b);
        use Type::*;
        match (&a, &b) {
            (Var(x), Var(y)) if x == y => Ok(()),
            (Var(x), Var(y)) if self.rigid.contains(x) && !self.rigid.contains(y) => {
                self.types.insert(*y, a.clone());
                Ok(())
            }
            (Var(v), t) | (t, Var(v)) => {
                if self.rigid.contains(v) {
                    return Err(format!("annotated type variable ~t{v} must remain distinct and general; cannot unify with {t}"));
                }
                let mut vars = BTreeSet::new();
                free(t, &mut vars);
                if vars.contains(v) {
                    return Err(format!("occurs check failed: ~t{v} occurs in {t}"));
                }
                self.types.insert(*v, t.clone());
                Ok(())
            }
            (Seq(x), Seq(y))
            | (Optional(x), Optional(y))
            | (Action(x), Action(y))
            | (Table(x), Table(y)) => self.unify_inner(x, y),
            (Dict(a, b), Dict(c, d)) | (Fun(a, b), Fun(c, d)) | (Result(a, b), Result(c, d)) => {
                self.unify_inner(a, c)?;
                self.unify_inner(b, d)
            }
            (Tuple(xs), Tuple(ys)) if xs.len() == ys.len() => {
                for (x, y) in xs.iter().zip(ys) {
                    self.unify_inner(x, y)?;
                }
                Ok(())
            }
            (Applied(x, xs), Applied(y, ys)) if x == y && xs.len() == ys.len() => {
                for (x, y) in xs.iter().zip(ys) {
                    self.unify_inner(x, y)?;
                }
                Ok(())
            }
            (Record(x), Record(y)) => self.unify_rows(x, y),
            (Nominal(x), Nominal(y)) | (Applied(x, _), Applied(y, _))
                if x != y && crate::visible_symbol(&x.name) == crate::visible_symbol(&y.name) =>
            {
                let left = crate::semantic::public_text(&x.name);
                let right = crate::semantic::public_text(&y.name);
                if left == right {
                    Err(format!(
                        "cannot unify distinct nominal declarations of {left}"
                    ))
                } else {
                    Err(format!(
                        "cannot unify distinct nominal types {left} and {right}"
                    ))
                }
            }
            _ if a == b => Ok(()),
            _ => Err(format!("cannot unify {a} with {b}")),
        }
    }
    fn bind_row(&mut self, id: u32, row: Row) -> R<()> {
        let row = self.resolve_row(&row);
        if row.tail == Some(id) && row.fields.is_empty() {
            return Ok(());
        }
        if self.rigid.contains(&id) {
            return Err(format!("annotated row variable ~r{id} must remain distinct and general; cannot unify with {row}"));
        }
        let mut vars = BTreeSet::new();
        free(&Type::Record(row.clone()), &mut vars);
        if vars.contains(&id) {
            return Err(format!("row occurs check failed: ~r{id} occurs in {row}"));
        }
        let banned = self.lacks.get(&id).cloned().unwrap_or_default();
        if let Some((n, _)) = row.fields.iter().find(|(n, _)| banned.contains(n)) {
            return Err(format!("duplicate row field {n}"));
        }
        self.register_row(&row)?;
        if let Some(t) = row.tail {
            self.add_lacks(t, banned)?;
        }
        self.rows.insert(id, row);
        Ok(())
    }
    fn unify_rows(&mut self, a: &Row, b: &Row) -> R<()> {
        self.register_row(a)?;
        self.register_row(b)?;
        let mut only_a = vec![];
        let mut only_b = vec![];
        for (n, ty) in &a.fields {
            if let Some((_, other)) = b.fields.iter().find(|(k, _)| k == n) {
                self.unify_inner(ty, other)
                    .map_err(|e| format!("field {n}: {e}"))?;
            } else {
                only_a.push((n.clone(), ty.clone()));
            }
        }
        for (n, ty) in &b.fields {
            if !a.fields.iter().any(|(k, _)| k == n) {
                only_b.push((n.clone(), ty.clone()));
            }
        }
        let absent = |fields: &[(String, Type)]| {
            format!(
                "missing row field {}",
                fields
                    .iter()
                    .map(|(n, _)| n.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        match (a.tail, b.tail) {
            (_, None) if !only_a.is_empty() => Err(absent(&only_a)),
            (None, _) if !only_b.is_empty() => Err(absent(&only_b)),
            (None, None) => Ok(()),
            (Some(x), None) => self.bind_row(
                x,
                Row {
                    fields: only_b,
                    tail: None,
                },
            ),
            (None, Some(y)) => self.bind_row(
                y,
                Row {
                    fields: only_a,
                    tail: None,
                },
            ),
            (Some(x), Some(y)) if x == y => {
                if only_a.is_empty() && only_b.is_empty() {
                    Ok(())
                } else {
                    Err(format!(
                        "shared row tail ~r{x} cannot absorb different fields"
                    ))
                }
            }
            (Some(x), Some(y))
                if self.rigid.contains(&x) && !self.rigid.contains(&y) && only_b.is_empty() =>
            {
                self.bind_row(
                    y,
                    Row {
                        fields: only_a,
                        tail: Some(x),
                    },
                )
            }
            (Some(x), Some(y))
                if self.rigid.contains(&y) && !self.rigid.contains(&x) && only_a.is_empty() =>
            {
                self.bind_row(
                    x,
                    Row {
                        fields: only_b,
                        tail: Some(y),
                    },
                )
            }
            (Some(x), Some(y)) if only_a.is_empty() && only_b.is_empty() => self.bind_row(
                x,
                Row {
                    fields: vec![],
                    tail: Some(y),
                },
            ),
            (Some(x), Some(y)) => {
                let tail = Some(self.fresh_row());
                self.bind_row(
                    x,
                    Row {
                        fields: only_b,
                        tail,
                    },
                )?;
                self.bind_row(
                    y,
                    Row {
                        fields: only_a,
                        tail,
                    },
                )
            }
        }
    }
    pub fn require(&mut self, class: Class, ty: &Type) -> R<()> {
        let mut leaf = Vec::new();
        Self::class_obligations(class, &self.resolve(ty), &mut leaf)?;
        self.register(ty)?;
        self.pending.extend(leaf);
        self.pending.sort();
        self.pending.dedup();
        Ok(())
    }

    pub fn constraints(&self) -> Vec<Constraint> {
        // Unification re-normalizes all pending obligations before committing.
        self.pending.clone()
    }

    fn check_pending(&mut self) -> R<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let mut keep = Vec::new();
        for c in &self.pending {
            Self::class_obligations(c.class, &self.resolve(&c.ty), &mut keep)?;
        }
        keep.sort();
        keep.dedup();
        self.pending = keep;
        Ok(())
    }

    fn class_obligations(class: Class, ty: &Type, out: &mut Vec<Constraint>) -> R<()> {
        use Class::*;
        use Type::*;
        let fail = || Err(format!("{ty} does not satisfy {class}"));
        match ty {
            Var(_) => out.push(Constraint {
                class,
                ty: ty.clone(),
            }),
            Prim(p) if class == Ord => {
                if !matches!(
                    p,
                    crate::types::Prim::N
                        | crate::types::Prim::Z
                        | crate::types::Prim::Q
                        | crate::types::Prim::D
                        | crate::types::Prim::R
                        | crate::types::Prim::S
                        | crate::types::Prim::C
                ) {
                    return fail();
                }
            }
            Prim(p)
                if class != Num
                    || matches!(
                        p,
                        crate::types::Prim::N
                            | crate::types::Prim::Z
                            | crate::types::Prim::Q
                            | crate::types::Prim::D
                            | crate::types::Prim::R
                    ) => {}
            Prim(_) => return fail(),
            _ if matches!(class, Num | Ord) => return fail(),
            Optional(x) => Self::class_obligations(class, x, out)?,
            Result(ok, err) if class == Eq => {
                Self::class_obligations(class, ok, out)?;
                Self::class_obligations(class, err, out)?;
            }
            Tuple(xs) => {
                for x in xs {
                    Self::class_obligations(class, x, out)?;
                }
            }
            Seq(x) if matches!(class, Eq | Ord) => Self::class_obligations(class, x, out)?,
            Table(x) if class == Eq => Self::class_obligations(class, x, out)?,
            Dict(k, v) if class == Eq => {
                Self::class_obligations(class, k, out)?;
                Self::class_obligations(class, v, out)?;
            }
            Record(row) if class == Eq => {
                for (_, t) in &row.fields {
                    Self::class_obligations(class, t, out)?;
                }
                if let Some(id) = row.tail {
                    out.push(Constraint {
                        class,
                        ty: Record(Row {
                            fields: vec![],
                            tail: Some(id),
                        }),
                    });
                }
            }
            // Nominal fields are supplied by the inference layer's schema environment.
            Nominal(_) if class == Eq => {}
            _ => return fail(),
        }
        Ok(())
    }

    /// Generalize variables free in this type, excluding variables free in the environment.
    /// Constraints belong to a scheme when they refer to one of its quantified variables.
    pub fn generalize(&self, ty: &Type, env: &[Scheme], constraints: &[Constraint]) -> Scheme {
        let ty = self.resolve(ty);
        let mut vars = BTreeSet::new();
        free(&ty, &mut vars);
        for s in env {
            let mut env_vars = BTreeSet::new();
            free(&self.resolve(&s.ty), &mut env_vars);
            for c in &s.constraints {
                free(&self.resolve(&c.ty), &mut env_vars);
            }
            for id in &s.vars {
                env_vars.remove(id);
            }
            vars.retain(|id| !env_vars.contains(id));
        }
        let mut constraints = constraints
            .iter()
            .map(|c| Constraint {
                class: c.class,
                ty: self.resolve(&c.ty),
            })
            .filter(|c| {
                let mut cv = BTreeSet::new();
                free(&c.ty, &mut cv);
                !cv.is_disjoint(&vars)
            })
            .collect::<Vec<_>>();
        constraints.sort();
        constraints.dedup();
        let mut row_lacks = BTreeMap::new();
        row_labels(&ty, &mut row_lacks);
        for c in &constraints {
            row_labels(&c.ty, &mut row_lacks);
        }
        row_lacks.retain(|id, labels| {
            if let Some(known) = self.lacks.get(id) {
                labels.extend(known.iter().cloned());
            }
            vars.contains(id)
        });
        Scheme {
            vars: vars.into_iter().collect(),
            ty,
            constraints,
            row_lacks,
        }
    }
    /// Return a freshly renamed type and constraints for the caller to enforce.
    pub fn instantiate(&mut self, scheme: &Scheme) -> (Type, Vec<Constraint>) {
        let mut vars = BTreeSet::new();
        free(&scheme.ty, &mut vars);
        for c in &scheme.constraints {
            free(&c.ty, &mut vars);
        }
        vars.extend(scheme.vars.iter().copied());
        if let Some(max) = vars.last() {
            self.next = self.next.max(
                max.checked_add(1)
                    .expect("type variable identifier space exhausted"),
            );
        }
        let mut rows = scheme.row_lacks.clone();
        row_labels(&scheme.ty, &mut rows);
        for c in &scheme.constraints {
            row_labels(&c.ty, &mut rows);
        }
        let ids = scheme
            .vars
            .iter()
            .map(|&v| {
                let id = self.fresh_id();
                if let Some(labels) = rows.remove(&v) {
                    self.row_ids.insert(id);
                    self.lacks.insert(id, labels);
                } else {
                    self.type_ids.insert(id);
                }
                (v, id)
            })
            .collect::<BTreeMap<_, _>>();
        let ty = remap(&scheme.ty, &ids);
        let constraints = scheme
            .constraints
            .iter()
            .map(|c| Constraint {
                class: c.class,
                ty: remap(&c.ty, &ids),
            })
            .collect();
        (ty, constraints)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(p: Prim) -> Type {
        Type::Prim(p)
    }
    fn row(fields: &[(&str, Type)], tail: Option<u32>) -> Type {
        Type::Record(Row {
            fields: fields
                .iter()
                .map(|(n, t)| (n.to_string(), t.clone()))
                .collect(),
            tail,
        })
    }

    #[test]
    fn unifies_through_constructors_and_rejects_cycles() {
        let mut ctx = InferCtx::new();
        let a = ctx.fresh();
        let b = ctx.fresh();
        let x = Type::Fun(
            Box::new(a.clone()),
            Box::new(Type::Optional(Box::new(b.clone()))),
        );
        let y = Type::Fun(
            Box::new(t(Prim::S)),
            Box::new(Type::Optional(Box::new(t(Prim::Q)))),
        );
        ctx.unify(&x, &y).unwrap();
        assert_eq!(ctx.resolve(&x), y);
        let c = ctx.fresh();
        assert!(ctx
            .unify(&c, &Type::Seq(Box::new(c.clone())))
            .unwrap_err()
            .contains("occurs check"));
        assert_eq!(ctx.resolve(&c), c);
    }

    #[test]
    fn quantified_instantiations_are_independent_and_constraints_rename() {
        let mut ctx = InferCtx::new();
        let a = ctx.fresh();
        let env_ty = ctx.fresh();
        let fun = Type::Fun(Box::new(a.clone()), Box::new(env_ty.clone()));
        ctx.require(Class::Num, &a).unwrap();
        let s = ctx.generalize(&fun, &[Scheme::mono(env_ty.clone())], &ctx.constraints());
        assert_eq!(s.vars.len(), 1);
        let (l, lc) = ctx.instantiate(&s);
        let (r, rc) = ctx.instantiate(&s);
        assert_ne!(l, r);
        assert_ne!(lc, rc);
        assert_eq!(lc[0].class, Class::Num);
        ctx.require(lc[0].class, &lc[0].ty).unwrap();
        assert!(ctx.unify(&lc[0].ty, &t(Prim::C)).is_err());
        assert_eq!(ctx.resolve(&lc[0].ty), lc[0].ty);
        ctx.unify(&lc[0].ty, &t(Prim::D)).unwrap();
        assert_eq!(ctx.resolve(&rc[0].ty), rc[0].ty);
    }

    #[test]
    fn open_rows_merge_then_close_without_losing_fields() {
        let mut ctx = InferCtx::new();
        let a = ctx.fresh_row();
        let b = ctx.fresh_row();
        let field = ctx.fresh();
        let x = row(&[("y", t(Prim::R)), ("x", field.clone())], Some(a));
        let y = row(&[("x", t(Prim::Z)), ("z", t(Prim::B))], Some(b));
        ctx.unify(&x, &y).unwrap();
        let z = row(
            &[("z", t(Prim::B)), ("x", t(Prim::Z)), ("y", t(Prim::R))],
            None,
        );
        ctx.unify(&x, &z).unwrap();
        let Type::Record(resolved) = ctx.resolve(&x) else {
            panic!()
        };
        assert_eq!(resolved.tail, None);
        assert_eq!(
            resolved
                .fields
                .iter()
                .map(|(n, _)| n.as_str())
                .collect::<Vec<_>>(),
            vec!["y", "x", "z"]
        );
        assert_eq!(ctx.resolve(&field), t(Prim::Z));
        assert!(ctx.unify(&x, &row(&[("x", t(Prim::Z))], None)).is_err());
    }

    #[test]
    fn rows_reject_duplicate_labels_cycles_and_conflicting_common_types() {
        for swap in [false, true] {
            let mut ctx = InferCtx::new();
            let a = ctx.fresh_row();
            let b = ctx.fresh_row();
            let x = row(&[("x", t(Prim::Z))], Some(a));
            let y = row(&[("x", t(Prim::B))], Some(b));
            assert!(ctx.unify(&x, &y).unwrap_err().contains("field x"));
            assert!(ctx
                .unify(&x, &row(&[("x", t(Prim::Z)), ("y", t(Prim::Z))], Some(a)))
                .is_err());
            let alias = row(&[], Some(a));
            let dup = row(&[("x", t(Prim::Z))], None);
            let err = if swap {
                ctx.unify(&dup, &alias)
            } else {
                ctx.unify(&alias, &dup)
            };
            // The failed unifications cannot have registered a lacks constraint.
            assert!(err.is_ok());
            assert!(ctx
                .unify(&x, &x)
                .unwrap_err()
                .contains("duplicate row field x"));
        }
    }

    #[test]
    fn row_quantification_freshens_tails() {
        let mut ctx = InferCtx::new();
        let a = ctx.fresh_row();
        let ty = row(&[("x", t(Prim::Z))], Some(a));
        let scheme = ctx.generalize(&ty, &[], &[]);
        let (l, _) = ctx.instantiate(&scheme);
        let (r, _) = ctx.instantiate(&scheme);
        assert_ne!(l, r);
        ctx.unify(&l, &row(&[("x", t(Prim::Z)), ("y", t(Prim::Q))], None))
            .unwrap();
        assert_eq!(ctx.resolve(&r), r);
    }

    #[test]
    fn composite_requirements_normalize_before_and_after_binding() {
        let mut cx = InferCtx::new();
        let a = cx.fresh();
        let b = cx.fresh();
        let c = cx.fresh();
        cx.require(
            Class::Eq,
            &Type::Seq(Box::new(Type::Optional(Box::new(a.clone())))),
        )
        .unwrap();
        cx.require(Class::Hash, &Type::Optional(Box::new(b.clone())))
            .unwrap();
        assert_eq!(
            cx.constraints(),
            vec![
                Constraint {
                    class: Class::Eq,
                    ty: a.clone()
                },
                Constraint {
                    class: Class::Hash,
                    ty: b.clone()
                },
            ]
        );
        cx.unify(&a, &Type::Tuple(vec![c.clone(), t(Prim::B)]))
            .unwrap();
        assert_eq!(
            cx.constraints()[0],
            Constraint {
                class: Class::Eq,
                ty: c.clone()
            }
        );
        let fun = Type::Fun(Box::new(t(Prim::N)), Box::new(t(Prim::N)));
        assert!(cx
            .unify(&c, &fun)
            .unwrap_err()
            .contains("does not satisfy eq"));
        assert!(cx
            .unify(&b, &fun)
            .unwrap_err()
            .contains("does not satisfy hash"));
        cx.unify(&c, &t(Prim::Z)).unwrap();
        cx.unify(&b, &t(Prim::S)).unwrap();
        assert!(cx.constraints().is_empty());
    }

    #[test]
    fn open_record_equality_normalizes_new_fields_after_row_binding() {
        let mut cx = InferCtx::new();
        let tail = cx.fresh_row();
        let a = cx.fresh();
        let b = cx.fresh();
        let open = row(&[("a", a.clone())], Some(tail));
        cx.require(Class::Eq, &open).unwrap();
        assert_eq!(
            cx.constraints(),
            vec![
                Constraint {
                    class: Class::Eq,
                    ty: a.clone()
                },
                Constraint {
                    class: Class::Eq,
                    ty: row(&[], Some(tail))
                },
            ]
        );
        cx.unify(&open, &row(&[("a", t(Prim::C)), ("b", b.clone())], None))
            .unwrap();
        assert_eq!(
            cx.constraints(),
            vec![Constraint {
                class: Class::Eq,
                ty: b.clone()
            }]
        );
        let action = Type::Action(Box::new(t(Prim::Unit)));
        assert!(cx
            .unify(&b, &action)
            .unwrap_err()
            .contains("does not satisfy eq"));
    }

    #[test]
    fn rigid_variables_reject_specialization_and_cannot_escape_as_outer_bindings() {
        let mut cx = InferCtx::new();
        let outer = cx.fresh();
        let fresh = cx.fresh();
        let rigid = cx.fresh_rigid();
        let other_rigid = cx.fresh_rigid();
        let Type::Var(rid) = rigid else { panic!() };
        assert!(cx.unify(&rigid, &other_rigid).is_err());
        assert!(cx
            .unify(&rigid, &t(Prim::Z))
            .unwrap_err()
            .contains(&format!("annotated type variable ~t{rid}")));
        assert!(cx
            .unify(&Type::Optional(Box::new(t(Prim::Z))), &rigid)
            .is_err());
        cx.unify(&rigid, &fresh).unwrap();
        assert_eq!(cx.resolve(&fresh), rigid);
        assert!(!cx.rigid_escapes(&[rid], &[Scheme::mono(outer.clone())]));
        cx.unify(&outer, &rigid).unwrap();
        assert!(cx.rigid_escapes(&[rid], &[Scheme::mono(outer)]));
        let scheme = cx.generalize(&rigid, &[], &[]);
        assert_eq!(scheme.vars, vec![rid]);
        assert_ne!(cx.instantiate(&scheme).0, rigid);
        cx.unmark_rigid(rid);
        cx.unify(&rigid, &t(Prim::Z)).unwrap();
    }

    #[test]
    fn rigid_row_tails_can_be_used_but_not_closed_or_strengthened() {
        let mut cx = InferCtx::new();
        let skolem = cx.fresh_row();
        let observed = row(&[("k", t(Prim::S)), ("v", t(Prim::Z))], Some(skolem));
        cx.register(&observed).unwrap();
        cx.mark_rigid(skolem);
        let flex = cx.fresh_row();
        let wanted = row(&[("k", t(Prim::S))], Some(flex));
        cx.unify(&observed, &wanted).unwrap();
        assert_eq!(cx.resolve(&wanted), observed);
        let closed = row(&[("k", t(Prim::S)), ("v", t(Prim::Z))], None);
        assert!(cx
            .unify(&observed, &closed)
            .unwrap_err()
            .contains(&format!("annotated row variable ~r{skolem}")));
        let extra = row(&[("new", t(Prim::Z))], Some(cx.fresh_row()));
        assert!(cx.unify(&observed, &extra).is_err());
        let strengthened = row(&[("extra", t(Prim::Z))], Some(skolem));
        assert!(cx
            .register(&strengthened)
            .unwrap_err()
            .contains("not known to lack field extra"));
    }

    #[test]
    fn rows_and_value_variables_cannot_reuse_an_identifier() {
        for row_first in [false, true] {
            let mut cx = InferCtx::new();
            let ordinary = Type::Var(32);
            let record = row(&[], Some(32));
            let (first, other) = if row_first {
                (&record, &ordinary)
            } else {
                (&ordinary, &record)
            };
            cx.register(first).unwrap();
            assert!(cx
                .register(other)
                .unwrap_err()
                .contains("both type and row kind"));
        }
    }

    #[test]
    fn a_scheme_replays_hidden_lacks_in_a_different_context() {
        let mut declaration = InferCtx::new();
        let id = declaration.fresh_row();
        let hidden_requirement = row(&[("reserved", t(Prim::B))], Some(id));
        declaration.register(&hidden_requirement).unwrap();
        let visible = row(&[("value", t(Prim::Q))], Some(id));
        let scheme = declaration.generalize(&visible, &[], &[]);
        assert_eq!(
            scheme.row_lacks[&id],
            BTreeSet::from(["reserved".into(), "value".into()])
        );

        let mut uses = InferCtx::new();
        let (bad, _) = uses.instantiate(&scheme);
        let forbidden = row(&[("value", t(Prim::Q)), ("reserved", t(Prim::B))], None);
        assert!(uses
            .unify(&bad, &forbidden)
            .unwrap_err()
            .contains("duplicate row field reserved"));
        let (good, _) = uses.instantiate(&scheme);
        let allowed = row(&[("value", t(Prim::Q)), ("label", t(Prim::S))], None);
        uses.unify(&good, &allowed).unwrap();
    }

    #[test]
    fn nominal_identity_is_not_shared_by_shadowed_spelling() {
        let mut cx = InferCtx::new();
        let nominal = |id| {
            Type::Nominal(NominalId {
                id,
                name: "row".into(),
            })
        };
        assert_eq!(nominal(1).to_string(), nominal(2).to_string());
        assert!(cx.unify(&nominal(1), &nominal(2)).is_err());
        cx.unify(&nominal(1), &nominal(1)).unwrap();
    }

    #[test]
    fn nominal_applications_preserve_identity_and_instantiate_arguments() {
        let mut cx = InferCtx::new();
        let nominal = |id, arg| {
            Type::Applied(
                NominalId {
                    id,
                    name: "tree".into(),
                },
                vec![arg],
            )
        };
        let arg = cx.fresh();
        let body = nominal(1, arg.clone());
        let scheme = cx.generalize(&body, &[], &[]);
        assert_eq!(scheme.vars.len(), 1);
        assert!(cx.unify(&arg, &body).is_err());
        let (first, _) = cx.instantiate(&scheme);
        let (second, _) = cx.instantiate(&scheme);
        cx.unify(&first, &nominal(1, t(Prim::N))).unwrap();
        cx.unify(&second, &nominal(1, t(Prim::S))).unwrap();
        assert_eq!(cx.resolve(&first).to_string(), "tree[n]");
        assert_eq!(cx.resolve(&second).to_string(), "tree[s]");
        assert!(cx.unify(&first, &nominal(2, t(Prim::N))).is_err());
        assert!(cx.require(Class::Eq, &first).is_err());
    }

    #[test]
    fn registering_public_unallocated_ids_reserves_them_and_rolls_back_on_failure() {
        let mut cx = InferCtx::new();
        cx.unify(&Type::Var(101), &t(Prim::Z)).unwrap();
        assert_eq!(cx.fresh(), Type::Var(102));
        assert!(cx.unify(&Type::Var(u32::MAX), &t(Prim::Z)).is_err());
        assert_eq!(cx.fresh(), Type::Var(103));
        let tail = 141;
        let open = row(&[("k", Type::Var(140))], Some(tail));
        let alias = Type::Var(131);
        cx.unify(&alias, &open).unwrap();
        let tailless = row(&[("k", t(Prim::Z)), ("v", t(Prim::B))], None);
        cx.unify(&alias, &tailless).unwrap();
        assert_eq!(cx.resolve(&alias), tailless);
        assert_eq!(cx.fresh(), Type::Var(142));
    }

    #[test]
    fn formatting_is_ordered_and_parenthesizes_function_operands() {
        let fields = row(&[("z", t(Prim::R)), ("a", t(Prim::D))], Some(7));
        assert_eq!(fields.to_string(), ".{z:r; a:d; | ~r7}");
        let fn_ty = Type::Fun(Box::new(t(Prim::N)), Box::new(t(Prim::Q)));
        assert_eq!(Type::Action(Box::new(fn_ty)).to_string(), "!(n->q)");
    }
    #[test]
    fn result_types_substitute_and_preserve_precedence() {
        let mut cx = InferCtx::new();
        let ok = cx.fresh();
        let err = cx.fresh();
        let ty = Type::Result(Box::new(ok.clone()), Box::new(err.clone()));
        assert_eq!(cx.free_vars(&ty).len(), 2);
        cx.unify(&ok, &t(Prim::Q)).unwrap();
        cx.unify(&err, &t(Prim::S)).unwrap();
        assert_eq!(cx.resolve(&ty).to_string(), "q^s");
        assert_eq!(
            Type::Action(Box::new(cx.resolve(&ty))).to_string(),
            "!(q^s)"
        );
        let inverse = Type::Result(
            Box::new(Type::Action(Box::new(t(Prim::Q)))),
            Box::new(t(Prim::S)),
        );
        assert_eq!(inverse.to_string(), "!q^s");
        let chained = Type::Fun(Box::new(cx.resolve(&ty)), Box::new(inverse));
        assert_eq!(chained.to_string(), "q^s->!q^s");
    }
}
