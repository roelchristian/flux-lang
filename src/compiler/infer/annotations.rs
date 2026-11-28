//! Canonical annotation conversion and callable signatures.
use super::{a, b, function, o, p, result, s, Engine, Signature, R};
use crate::parser::{TypeAnnotation, TypeAnnotationKind, TypePrimitive};
use crate::types::{Prim, Row, Type};
use std::collections::HashMap;

impl Engine<'_> {
    pub(super) fn annotation(
        &mut self,
        t: &TypeAnnotation,
        vars: &mut HashMap<String, Type>,
    ) -> R<Type> {
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
                TypePrimitive::Cx => Prim::Cx,
            }),
            K::Named(name) if name == "array" => {
                return Err(self.err("type array expects 1 type argument"))
            }
            K::Named(name) => {
                let id = self
                    .schema_id(name)
                    .ok_or_else(|| self.err(format!("unknown type {name}")))?
                    .clone();
                if let Some(arity @ 1..) = self.sum_arities.get(&id).copied() {
                    return Err(self.err(format!("type {name} expects {arity} type arguments")));
                }
                Type::Nominal(id)
            }
            K::NamedApply(name, args) if name == "array" => {
                if args.len() != 1 {
                    return Err(self.err(format!(
                        "type array expects 1 type argument, found {}",
                        args.len()
                    )));
                }
                Type::dense_array(self.annotation(&args[0], vars)?)
            }
            K::NamedApply(name, args) => {
                let id = self
                    .schema_id(name)
                    .ok_or_else(|| self.err(format!("unknown type {name}")))?
                    .clone();
                let arity = self.sum_arities.get(&id).copied().unwrap_or(0);
                if arity == 0 || arity != args.len() {
                    return Err(self.err(format!(
                        "type {name} expects {arity} type arguments, found {}",
                        args.len()
                    )));
                }
                Type::Applied(
                    id,
                    args.iter()
                        .map(|a| self.annotation(a, vars))
                        .collect::<R<_>>()?,
                )
            }
            K::Variable(name) => {
                let t = vars
                    .entry(name.clone())
                    .or_insert_with(|| self.cx.fresh())
                    .clone();
                if !matches!(t, Type::Var(_)) {
                    return Err(self.err(format!("{name} is a row variable, not an ordinary type")));
                }
                t
            }
            K::OpenRecord(fs, name) => {
                let marker = vars
                    .entry(name.clone())
                    .or_insert_with(|| {
                        Type::Record(Row {
                            fields: vec![],
                            tail: Some(self.cx.fresh_row()),
                        })
                    })
                    .clone();
                let Type::Record(Row { tail: Some(id), .. }) = marker else {
                    return Err(self.err(format!(
                        "{name} is an ordinary type variable, not a row variable"
                    )));
                };
                let ty = Type::Record(Row {
                    fields: fs
                        .iter()
                        .map(|(n, t)| Ok((n.clone(), self.annotation(t, vars)?)))
                        .collect::<R<_>>()?,
                    tail: Some(id),
                });
                self.cx.register(&ty).map_err(|msg| self.err(msg))?;
                ty
            }
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

    pub(super) fn signature(
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
}
