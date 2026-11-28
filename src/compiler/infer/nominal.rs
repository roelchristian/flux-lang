//! Lexical schemas, nominal sums, constructors, and standard errors.
use super::{function, mono, Constructor, Engine, R};
use crate::parser::{Expr, Stmt, TypeAnnotation};
use crate::resolve::SymbolId;
use crate::types::{NominalId, Row, Type};
use std::collections::{HashMap, HashSet};

impl Engine<'_> {
    pub(super) fn standard_type(name: &str) -> Type {
        Type::Nominal(NominalId {
            id: crate::standard_errors::owner_id(name).expect("known standard error type"),
            name: name.to_owned(),
        })
    }

    pub(super) fn install_standard_errors(&mut self, imports: &[String]) -> R<()> {
        for &name in crate::standard_errors::OWNERS {
            if name != "ioerr"
                && !imports
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
        for &name in crate::standard_errors::OWNERS {
            let Type::Nominal(owner) = Self::standard_type(name) else {
                unreachable!()
            };
            let mut ids = Vec::new();
            for v in crate::standard_errors::variants(name) {
                if let Some(id) = self.resolver.standard_constructor(name, v.name) {
                    let mut payload = Vec::new();
                    for t in v.payload {
                        payload.push(if crate::standard_errors::owner_id(t).is_some() {
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

    pub(super) fn schema_id(&self, name: &str) -> Option<&NominalId> {
        self.schema_scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name))
    }

    pub(super) fn schema_headers(&mut self, defs: &[Stmt]) -> R<()> {
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
                let ty = if let Stmt::Sum { type_params, .. } = stmt {
                    self.sum_arities.insert(id.clone(), type_params.len());
                    let distinct = type_params.iter().collect::<HashSet<_>>();
                    if distinct.len() != type_params.len() {
                        return Err(self.err("duplicate sum type parameter"));
                    }
                    if type_params.is_empty() {
                        mono(Type::Nominal(id))
                    } else {
                        let args = type_params
                            .iter()
                            .map(|_| self.cx.fresh())
                            .collect::<Vec<_>>();
                        self.cx.generalize(&Type::Applied(id, args), &[], &[])
                    }
                } else {
                    mono(Type::Nominal(id))
                };
                self.result.bindings.insert(symbol, ty);
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
                    type_params,
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
                        let mut vars = type_params
                            .iter()
                            .map(|name| (name.clone(), self.cx.fresh()))
                            .collect::<HashMap<_, _>>();
                        let args = type_params
                            .iter()
                            .map(|name| vars[name].clone())
                            .collect::<Vec<_>>();
                        let declared = args
                            .iter()
                            .flat_map(|t| self.cx.free_vars(t))
                            .collect::<HashSet<_>>();
                        let mut payload = Vec::new();
                        for spelling in &variant.payload {
                            let t = self.annotation(spelling, &mut vars)?;
                            if self
                                .cx
                                .free_vars(&t)
                                .iter()
                                .any(|id| !declared.contains(id))
                            {
                                return Err(self.err(
                                    "undeclared type variable or hole in sum constructor payload",
                                ));
                            }
                            payload.push(t);
                        }
                        let result = if args.is_empty() {
                            Type::Nominal(owner.clone())
                        } else {
                            Type::Applied(owner.clone(), args)
                        };
                        let ty = function(payload.iter().cloned(), result);
                        self.result
                            .bindings
                            .insert(id, self.cx.generalize(&ty, &[], &[]));
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

    pub(super) fn constructor_type(&mut self, e: &Expr) -> Option<Type> {
        let id = self.resolver.reference(e)?;
        self.result.constructors.get(&id)?;
        let scheme = self.result.bindings.get(&id)?;
        Some(self.cx.instantiate(scheme).0)
    }

    pub(super) fn match_constructor_signature(&mut self, id: SymbolId) -> R<(Type, Vec<Type>)> {
        let arity = self
            .result
            .constructors
            .get(&id)
            .ok_or_else(|| self.err("unknown constructor"))?
            .payload
            .len();
        let scheme = self
            .result
            .bindings
            .get(&id)
            .ok_or_else(|| self.err("missing constructor scheme"))?;
        let (mut result, _) = self.cx.instantiate(scheme);
        let mut payload = Vec::with_capacity(arity);
        for _ in 0..arity {
            let Type::Fun(arg, rest) = result else {
                return Err(self.err("invalid checked constructor scheme"));
            };
            payload.push(*arg);
            result = *rest;
        }
        Ok((result, payload))
    }

    pub(super) fn match_constructor(&mut self, id: SymbolId, subject: &Type) -> R<Vec<Type>> {
        let (result, payload) = self.match_constructor_signature(id)?;
        self.unify(&result, subject)?;
        Ok(payload.iter().map(|p| self.cx.resolve(p)).collect())
    }

    pub(super) fn match_constructors(&mut self, t: &Type) -> R<Vec<(SymbolId, Vec<Type>)>> {
        let ty = self.cx.resolve(t);
        let owner = match &ty {
            Type::Nominal(owner) | Type::Applied(owner, _) => owner,
            _ => return Ok(vec![]),
        };
        let ids = self.sums.get(owner).cloned().unwrap_or_default();
        ids.into_iter()
            .map(|id| Ok((id, self.match_constructor(id, &ty)?)))
            .collect()
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
}
