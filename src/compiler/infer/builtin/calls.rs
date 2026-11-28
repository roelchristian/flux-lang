//! Builtin call and reference inference, including contextual operand ordering.
use super::super::{a, b, f, function, o, p, result, s, Engine, Env, Operation, R};
use crate::parser::Expr;
use crate::types::{Class, Prim, Row, Type};

fn fallible_builtin(spec: &crate::builtins::Builtin) -> bool {
    spec.math_signature()
        .is_some_and(|sig| sig.output == crate::builtins::MathType::SolverResult)
        || matches!(
            spec.canonical,
            "io.print"
                | "io.println"
                | "io.read"
                | "io.write"
                | "io.exists"
                | "csv.read"
                | "csv.write"
                | "math.div"
                | "io.readln"
                | "fs.ls"
                | "fs.mkdir"
                | "sys.env"
                | "rx.find"
                | "dt.days"
                | "dt.date"
                | "json.encode"
                | "json.decode"
                | "opt.result"
                | "res.map"
                | "res.maperr"
                | "res.collect"
                | "res.partition"
                | "res.ok"
                | "res.optional"
        )
}

impl Engine<'_> {
    pub(in crate::infer) fn builtin_ref(&mut self, n: &str, want: Option<&Type>) -> R<Type> {
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
        if fallible_builtin(spec) {
            self.saw_result = true;
        }
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

    pub(super) fn known_callee(&self, env: &Env, e: &Expr) -> bool {
        match e {
            Expr::Name(name) | Expr::Ref(name) => {
                env.contains_key(name)
                    || self
                        .resolver
                        .reference(e)
                        .is_some_and(|id| self.result.constructors.contains_key(&id))
            }
            _ => false,
        }
    }

    /// Known functions supply argument context, whereas overloaded references
    /// need operand information first. This order affects inference only.
    fn check_application_pair(
        &mut self,
        env: &Env,
        args: &[Expr],
        function: &Type,
        operand: &Type,
        effect: bool,
    ) -> R<bool> {
        let operands = [(&args[0], function), (&args[1], operand)];
        let order = if self.known_callee(env, &args[0]) {
            [0, 1]
        } else {
            [1, 0]
        };
        let mut diverges = false;
        for i in order {
            let (expr, expected) = operands[i];
            diverges |= self.expr(env, expr, Some(expected), effect)? == Type::Never;
        }
        Ok(diverges)
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

    pub(in crate::infer) fn builtin_call(
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
        if spec.is_some_and(fallible_builtin) {
            self.saw_result = true;
        }
        let result = match n {
            _ if spec.is_some_and(|b| b.math_signature().is_some()) => {
                self.mathesis_call(env, spec.unwrap(), args, want, effect)?
            }
            op if op == "form" || (op.starts_with("arr.") && spec.is_some()) => {
                self.array_call(env, op, args, want, effect)?
            }
            "*=" | "*?" | "df.semi" | "df.anti" | "df.union" | "df.distinct" | "df.select"
            | "df.dropna" | "df.fill" | "df.present" => {
                self.relational_call(env, n, args, want, effect)?
            }

            "opt.map" | "opt.bind" | "opt.result" | "res.map" | "res.maperr" | "res.collect"
            | "res.partition" | "res.ok" | "res.optional" => {
                self.native_combinator(env, n, args, want, effect)?
            }
            "@" => {
                let arg = self.cx.fresh();
                let out = want.cloned().unwrap_or_else(|| self.cx.fresh());
                let expected = f(arg.clone(), out.clone());
                if self.check_application_pair(env, args, &expected, &arg, effect)? {
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
                if let Some(elt) = res.array_element().filter(|_| n == "@:") {
                    let want = want.map(|t| self.cx.resolve(t));
                    let result = self.unary_fn(
                        env,
                        &args[0],
                        elt,
                        want.as_ref().and_then(Type::array_element),
                        effect,
                    )?;
                    Type::dense_array(result)
                } else {
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
                let out = self.row(&row)?;
                let columns = self.expr(env, &args[0], None, effect)?;
                self.extend_columns(out, &columns)?
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
                    .insert(self.syntax_id(&args[0]), s(row.clone()));
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
                    Type::Nominal(_) | Type::Record(_) => self.column_record(&src)?,
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
                    t if t.array_element().is_some() => {
                        self.expr(env, &args[0], Some(&s(p(N))), effect)?;
                        o(t.array_element().unwrap().clone())
                    }
                    x => return Err(self.err(format!("cannot index {x}"))),
                }
            }
            "@/" => {
                let wanted = want.map(|w| self.cx.resolve(w));
                let optional = wanted.as_ref().and_then(|t| t.array_element().or(Some(t)));
                let item = match optional {
                    Some(Type::Optional(t)) => t.as_ref().clone(),
                    _ => self.cx.fresh(),
                };
                let expected = f(item.clone(), f(item.clone(), item.clone()));
                let source_want = if wanted.as_ref().and_then(Type::array_element).is_some() {
                    Some(Type::dense_array(item.clone()))
                } else if matches!(wanted, Some(Type::Optional(_)))
                    || matches!(&args[1], Expr::Array(_))
                {
                    Some(s(item.clone()))
                } else {
                    None
                };
                let known = self.known_callee(env, &args[0]);
                let mut bottom = false;
                if known {
                    bottom |= self.expr(env, &args[0], Some(&expected), effect)? == Type::Never;
                }
                let source = self.expr(env, &args[1], source_want.as_ref(), effect)?;
                let dense = self.cx.resolve(&source).array_element().is_some();
                if source == Type::Never {
                    bottom = true;
                } else {
                    let shape = if dense {
                        Type::dense_array(item.clone())
                    } else {
                        s(item.clone())
                    };
                    self.unify(&source, &shape)?;
                }
                if !known {
                    bottom |= self.expr(env, &args[0], Some(&expected), effect)? == Type::Never;
                }
                if bottom {
                    Type::Never
                } else if dense {
                    Type::dense_array(o(item))
                } else {
                    o(item)
                }
            }
            "/:" | "\\:" => {
                let source = self.expr(env, &args[2], None, effect)?;
                let wanted = want.map(|w| self.cx.resolve(w));
                let dense = self.cx.resolve(&source).array_element().is_some();
                let acc_expect = if dense {
                    wanted.as_ref().and_then(Type::array_element)
                } else {
                    match (n, wanted.as_ref()) {
                        ("/:", w) => w,
                        (_, Some(Type::Seq(t))) => Some(t.as_ref()),
                        _ => None,
                    }
                };
                let acc = self.expr(env, &args[1], acc_expect, effect)?;
                let (item, dense) = self.sequence_or_dense_element(&source)?;
                self.expr(
                    env,
                    &args[0],
                    Some(&f(acc.clone(), f(item, acc.clone()))),
                    effect,
                )?;
                if dense {
                    Type::dense_array(acc)
                } else if n == "/:" {
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
                            ("^", 1) if matches!(x, Expr::Int(_) | Expr::Num(_)) => Some(p(N)),
                            ("[:" | ":]" | "math.quantize", 0) | ("sect", 0 | 1) => Some(p(N)),
                            ("[:" | ":]", 1) | ("sect", 2) | ("rev" | "=.", 0) | ("+:", 0 | 1) => {
                                want.map(|w| self.cx.resolve(w)).filter(|t| {
                                    matches!(t, Type::Seq(_) | Type::Table(_) | Type::Prim(Prim::S))
                                })
                            }
                            ("|:", 0) => want.map(|w| self.cx.resolve(w)).filter(|t| {
                                t.array_element().is_some()
                                    || matches!(
                                        t,
                                        Type::Seq(_) | Type::Table(_) | Type::Prim(Prim::S)
                                    )
                            }),
                            (",.", 0) if matches!(x, Expr::Array(_)) => {
                                match want.map(|w| self.cx.resolve(w)) {
                                    Some(t @ Type::Seq(_)) => Some(s(t)),
                                    _ => None,
                                }
                            }
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
            self.syntax_id(call),
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
}
