//! Numeric literal defaulting, domain promotion, and comparisons.
use super::{p, s, Engine, InferError, InferErrorKind, R};
use crate::types::{Class, Prim, Type};

pub(super) fn exact(t: Prim) -> bool {
    matches!(t, Prim::N | Prim::Z | Prim::Q | Prim::D)
}
pub(super) fn num(t: Prim) -> bool {
    exact(t) || t == Prim::R
}
fn numeric_kind(x: Prim) -> Option<crate::builtins::Numeric> {
    use crate::builtins::Numeric as K;
    Some(match x {
        Prim::N => K::N,
        Prim::Z => K::Z,
        Prim::Q => K::Q,
        Prim::D => K::D,
        Prim::R => K::R,
        _ => return None,
    })
}
fn numeric_prim(x: crate::builtins::Numeric) -> Prim {
    use crate::builtins::Numeric as K;
    match x {
        K::N => Prim::N,
        K::Z => Prim::Z,
        K::Q => Prim::Q,
        K::D => Prim::D,
        K::R => Prim::R,
    }
}
fn solve_numeric(a: Prim, b: Prim) -> Option<Prim> {
    crate::builtins::promote(numeric_kind(a)?, numeric_kind(b)?).map(numeric_prim)
}

impl Engine<'_> {
    pub(super) fn int(&mut self, n: i128, want: Option<&Type>) -> R<Type> {
        self.int_signed(n < 0, want)
    }

    pub(super) fn int_signed(&mut self, negative: bool, want: Option<&Type>) -> R<Type> {
        match want.map(|w| self.cx.resolve(w)) {
            Some(Type::Prim(Prim::N)) if negative => {
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
                self.literals.push((t.clone(), negative, self.start, end));
                Ok(t)
            }
        }
    }

    pub(super) fn default_literals(&mut self, from: usize) -> R<()> {
        let entries = self.literals[from..].to_vec();
        for (t, negative, start, end) in entries {
            match self.cx.resolve(&t) {
                Type::Var(_) => {
                    self.unify(&t, &p(Prim::Z)).map_err(|mut failure| {
                        failure.start = start;
                        failure.end = end;
                        failure
                    })?;
                }
                Type::Prim(Prim::N) if negative => {
                    return Err(InferError {
                        kind: InferErrorKind::TypeError,
                        start,
                        end,
                        expected: None,
                        actual: None,
                        constraint: None,
                        message: "negative integer literal cannot have type n".into(),
                    })
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(super) fn require_exact_numeric(&mut self, t: &Type, n: &str) -> R<()> {
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

    fn numeric_mixture(&self, op: &str, a: Prim, b: Prim) -> InferError {
        let mut failure = self.err("r does not implicitly mix with exact numerical types");
        failure.actual = Some(format!("{}, {}", p(a), p(b)));
        failure.expected = Some("two exact operands or two approximate operands".into());
        failure.constraint = Some(format!("compatible numerical domains for {op}"));
        failure
    }

    pub(super) fn numeric_binary(
        &mut self,
        n: &str,
        ta: &Type,
        tb: &Type,
        out: Option<&Type>,
    ) -> R<Type> {
        use crate::builtins::{lookup, BinaryDomain};
        use Prim::*;
        let rule = lookup(n)
            .and_then(|entry| BinaryDomain::from_rule(entry.rule))
            .ok_or_else(|| self.err(format!("missing numerical domain rule for {n}")))?;
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
        if matches!(n, "+" | "-" | "*" | "/") {
            let complex = p(Cx);
            if ta == complex || tb == complex || out.as_ref() == Some(&complex) {
                self.unify(&ta, &complex)?;
                self.unify(&tb, &complex)?;
                return Ok(complex);
            }
        }
        if rule == BinaryDomain::Power {
            self.unify(&tb, &p(N))?;
            self.req(Class::Num, &ta)?;
            return Ok(ta);
        }
        self.req(Class::Num, &ta)?;
        self.req(Class::Num, &tb)?;
        let k = match (&ta, &tb) {
            (Type::Prim(a), Type::Prim(b)) => {
                p(solve_numeric(*a, *b).ok_or_else(|| self.numeric_mixture(n, *a, *b))?)
            }
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
                    if rule != BinaryDomain::Division || t == &p(R) {
                        self.unify(&ta, t)?;
                    }
                }
                ta.clone()
            }
            _ => return Err(self.err("expected numerical operands")),
        };
        let mut k = self.cx.resolve(&k);
        if rule == BinaryDomain::Integral && matches!(k, Type::Var(_)) {
            self.unify(&k, &p(Z))?;
            k = p(Z);
        }
        if rule == BinaryDomain::Division && matches!(k, Type::Var(_)) {
            self.require_exact_numeric(&k, "division")?;
            k = self.cx.resolve(&k);
        }
        if let Type::Prim(t) = k {
            let domain = numeric_kind(t).ok_or_else(|| self.err("expected numerical operands"))?;
            Ok(p(numeric_prim(
                rule.result(n, domain).map_err(|e| self.err(e))?,
            )))
        } else {
            Ok(k)
        }
    }

    pub(super) fn binary_rel(&mut self, n: &str, a: &Type, b: &Type) -> R<Type> {
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
                p(solve_numeric(*a, *b).ok_or_else(|| self.numeric_mixture(n, *a, *b))?)
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
}
