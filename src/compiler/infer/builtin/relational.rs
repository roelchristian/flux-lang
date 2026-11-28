//! Relational builtin signatures and ordered column schema inference.
use super::super::{b, f, o, s, Engine, Env, R};
use crate::parser::Expr;
use crate::types::{Class, Row, Type};

impl Engine<'_> {
    fn table_row(&mut self, t: &Type, op: &str) -> R<Type> {
        match self.cx.resolve(t) {
            Type::Table(row) => Ok(*row),
            v @ Type::Var(_) => {
                let row = self.cx.fresh();
                self.unify(&v, &Type::Table(b(row.clone())))?;
                Ok(row)
            }
            x => Err(self.err(format!("{op} expects a table, found {x}"))),
        }
    }

    fn selectable_row(&self, t: &Type) -> R<()> {
        match self.cx.resolve(t) {
            Type::Tuple(_) => Ok(()),
            Type::Record(_) => Ok(()),
            Type::Nominal(id) if self.schemas.contains_key(&id) => Ok(()),
            x => Err(self.err(format!(
                "df.select callback must return a record or tuple row, found {x}"
            ))),
        }
    }

    fn join_output(&self, op: &str, left: Type, right: Type) -> Type {
        Type::Table(b(match op {
            "*=" => Type::Tuple(vec![left, right]),
            "*?" => Type::Tuple(vec![left, o(right)]),
            _ => left,
        }))
    }

    pub(super) fn relational_call(
        &mut self,
        env: &Env,
        op: &str,
        args: &[Expr],
        want: Option<&Type>,
        effect: bool,
    ) -> R<Type> {
        let wanted = want.map(|t| self.cx.resolve(t));
        match op {
            "*=" | "*?" | "df.semi" | "df.anti" => {
                let mut input_wants = [None, None];
                if let Some(Type::Table(result)) = wanted {
                    match (op, *result) {
                        ("*=" | "*?", Type::Tuple(mut ts)) if ts.len() == 2 => {
                            let right = ts.pop().unwrap();
                            let left = ts.pop().unwrap();
                            input_wants[0] = Some(Type::Table(b(left)));
                            match (op, right) {
                                ("*?", Type::Optional(t)) => input_wants[1] = Some(Type::Table(t)),
                                ("*=", t) => input_wants[1] = Some(Type::Table(b(t))),
                                _ => {}
                            }
                        }
                        ("df.semi" | "df.anti", t) => input_wants[0] = Some(Type::Table(b(t))),
                        _ => {}
                    }
                }
                let left = self.expr(env, &args[2], input_wants[0].as_ref(), effect)?;
                let right = self.expr(env, &args[3], input_wants[1].as_ref(), effect)?;
                let row_a = if left == Type::Never {
                    self.cx.fresh()
                } else {
                    self.table_row(&left, op)?
                };
                let row_b = if right == Type::Never {
                    self.cx.fresh()
                } else {
                    self.table_row(&right, op)?
                };
                // A shared key type requires identity, not numeric promotion.
                let key = self.cx.fresh();
                let l = self.expr(env, &args[0], Some(&f(row_a.clone(), key.clone())), effect)?;
                let r = self.expr(env, &args[1], Some(&f(row_b.clone(), key.clone())), effect)?;
                self.req(Class::Hash, &key)?;
                Ok(if [l, r, left, right].contains(&Type::Never) {
                    Type::Never
                } else {
                    self.join_output(op, row_a, row_b)
                })
            }
            "df.select" | "df.dropna" => {
                let input = self.expr(
                    env,
                    &args[1],
                    if op == "df.dropna" { want } else { None },
                    effect,
                )?;
                let row = if input == Type::Never {
                    self.cx.fresh()
                } else {
                    self.table_row(&input, op)?
                };
                let mapped = if op == "df.select" {
                    match wanted {
                        Some(Type::Table(t)) => *t,
                        _ => self.cx.fresh(),
                    }
                } else {
                    o(self.cx.fresh())
                };
                let selector = self.expr(env, &args[0], Some(&f(row, mapped.clone())), effect)?;
                if selector == Type::Never || input == Type::Never {
                    return Ok(Type::Never);
                }
                if op == "df.select" {
                    self.selectable_row(&mapped)?;
                    Ok(Type::Table(b(mapped)))
                } else {
                    Ok(input)
                }
            }
            "df.union" | "df.distinct" => {
                let row = self.cx.fresh();
                let result = Type::Table(b(row.clone()));
                if let Some(w) = want {
                    self.unify(&result, w)?;
                }
                let mut bottom = false;
                for x in args {
                    bottom |= self.expr(env, x, Some(&result), effect)? == Type::Never;
                }
                self.req(Class::Eq, &row)?;
                Ok(if bottom { Type::Never } else { result })
            }
            "df.fill" | "df.present" => {
                let item = match wanted {
                    Some(Type::Seq(t)) => *t,
                    _ => self.cx.fresh(),
                };
                let input =
                    self.expr(env, args.last().unwrap(), Some(&s(o(item.clone()))), effect)?;
                let fill = if op == "df.fill" {
                    self.expr(env, &args[0], Some(&item), effect)?
                } else {
                    item.clone()
                };
                Ok(if input == Type::Never || fill == Type::Never {
                    Type::Never
                } else {
                    s(item)
                })
            }
            _ => unreachable!(),
        }
    }

    pub(super) fn relational_types(
        &mut self,
        op: &str,
        xs: &[Type],
        want: Option<&Type>,
    ) -> R<Type> {
        match op {
            "*=" | "*?" | "df.semi" | "df.anti" => {
                let row_a = self.table_row(&xs[2], op)?;
                let row_b = self.table_row(&xs[3], op)?;
                let key = self.cx.fresh();
                self.unify(&xs[0], &f(row_a.clone(), key.clone()))?;
                self.unify(&xs[1], &f(row_b.clone(), key.clone()))?;
                self.req(Class::Hash, &key)?;
                Ok(self.join_output(op, row_a, row_b))
            }
            "df.select" | "df.dropna" => {
                let row = self.table_row(&xs[1], op)?;
                let mapped = if op == "df.select" {
                    match want.map(|w| self.cx.resolve(w)) {
                        Some(Type::Table(t)) => *t,
                        _ => self.cx.fresh(),
                    }
                } else {
                    o(self.cx.fresh())
                };
                self.unify(&xs[0], &f(row.clone(), mapped.clone()))?;
                if op == "df.select" {
                    self.selectable_row(&mapped)?;
                    Ok(Type::Table(b(mapped)))
                } else {
                    Ok(Type::Table(b(row)))
                }
            }
            "df.union" | "df.distinct" => {
                let row = self.table_row(&xs[0], op)?;
                let out = Type::Table(b(row.clone()));
                if op == "df.union" {
                    self.unify(&xs[1], &out)?;
                }
                self.req(Class::Eq, &row)?;
                Ok(out)
            }
            "df.fill" | "df.present" => {
                let item = self.cx.fresh();
                self.unify(xs.last().unwrap(), &s(o(item.clone())))?;
                if op == "df.fill" {
                    self.unify(&xs[0], &item)?;
                }
                Ok(s(item))
            }
            _ => unreachable!(),
        }
    }

    pub(super) fn extend_columns(&mut self, mut out: Row, source: &Type) -> R<Type> {
        let columns = self.row(source)?;
        if columns.tail.is_some() {
            return Err(self.err("df.extend requires a closed record of new columns"));
        }
        if columns.fields.is_empty() {
            return Err(self.err("df.extend requires at least one new column"));
        }
        for (name, t) in columns.fields {
            if out.fields.iter().any(|(k, _)| k == &name) {
                return Err(self.err(format!("df.extend cannot replace existing column {name}")));
            }
            out.fields.push((name, self.seq_element(&t)?));
        }
        let ty = Type::Table(b(Type::Record(out)));
        self.cx.register(&ty).map_err(|msg| self.err(msg))?;
        Ok(ty)
    }

    pub(super) fn column_record(&mut self, source: &Type) -> R<Type> {
        let columns = self.row(source)?;
        if columns.tail.is_some() {
            return Err(self.err("#: column records must have a closed schema; use a sequence of rows for an open row"));
        }
        if columns.fields.is_empty() {
            return Err(self.err("#: requires at least one column"));
        }
        let fields = columns
            .fields
            .into_iter()
            .map(|(name, t)| Ok((name, self.seq_element(&t)?)))
            .collect::<R<_>>()?;
        Ok(Type::Record(Row { fields, tail: None }))
    }
}
