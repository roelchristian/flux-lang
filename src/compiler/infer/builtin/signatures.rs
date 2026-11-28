//! Checked result types for builtin operands and callable references.
use super::super::{a, b, f, function, o, p, result, s, Engine, R};
use crate::types::{Class, Prim, Type};
use std::collections::HashSet;

impl Engine<'_> {
    pub(super) fn builtin_types(&mut self, n: &str, xs: &[Type], want: Option<&Type>) -> R<Type> {
        use Prim::*;
        let spec = crate::builtins::lookup(n);
        let arity = spec
            .map(|x| x.arity)
            .or_else(|| n.starts_with('.').then_some(1));
        if let Some(arity) = arity {
            if xs.len() != arity {
                return Err(self.err(format!("{n} expects {arity} operands")));
            }
        }
        if let Some(spec) = spec.filter(|spec| spec.math_signature().is_some()) {
            return self.mathesis_types(spec, xs, want);
        }
        if spec.is_some_and(|spec| spec.shape == crate::builtins::Shape::Elementwise) {
            if let Some(ty) = self.array_elementwise(n, xs, want)? {
                return Ok(ty);
            }
        }
        Ok(match n {
            op if op == "form" || (op.starts_with("arr.") && arity.is_some()) => {
                self.array_types(op, xs, want)?
            }
            "*=" | "*?" | "df.semi" | "df.anti" | "df.union" | "df.distinct" | "df.select"
            | "df.dropna" | "df.fill" | "df.present" => self.relational_types(n, xs, want)?,
            "df.structural" => {
                let Type::Table(row) = self.cx.resolve(&xs[0]) else {
                    return Err(self.err("df.structural expects a table of named rows"));
                };
                Type::Table(b(Type::Record(self.row(&row)?)))
            }

            "opt.map" | "opt.bind" => {
                let (a0, b0) = (self.cx.fresh(), self.cx.fresh());
                let out = if n == "opt.bind" {
                    o(b0.clone())
                } else {
                    b0.clone()
                };
                self.unify(&xs[1], &o(a0.clone()))?;
                self.unify(&xs[0], &f(a0, out))?;
                o(b0)
            }
            "opt.result" => {
                let item = self.cx.fresh();
                self.unify(&xs[1], &o(item.clone()))?;
                result(item, xs[0].clone())
            }
            "res.map" | "res.maperr" => {
                let (a0, e0, mapped) = (self.cx.fresh(), self.cx.fresh(), self.cx.fresh());
                self.unify(&xs[1], &result(a0.clone(), e0.clone()))?;
                if n == "res.map" {
                    self.unify(&xs[0], &f(a0, mapped.clone()))?;
                    result(mapped, e0)
                } else {
                    self.unify(&xs[0], &f(e0, mapped.clone()))?;
                    result(a0, mapped)
                }
            }
            "res.collect" | "res.partition" => {
                let (a0, e0) = (self.cx.fresh(), self.cx.fresh());
                self.unify(&xs[0], &s(result(a0.clone(), e0.clone())))?;
                if n == "res.collect" {
                    result(s(a0), e0)
                } else {
                    Type::Tuple(vec![s(a0), s(e0)])
                }
            }
            "res.ok" | "res.optional" => {
                let (a0, e0) = (self.cx.fresh(), self.cx.fresh());
                self.unify(&xs[0], &result(a0.clone(), e0))?;
                if n == "res.ok" {
                    p(B)
                } else {
                    o(a0)
                }
            }
            "io.readln" => a(result(o(p(S)), Self::standard_type("ioerr"))),
            "fs.ls" | "fs.mkdir" | "sys.env" => {
                self.unify(&xs[0], &p(S))?;
                let out = match n {
                    "fs.ls" => s(p(S)),
                    "fs.mkdir" => p(Unit),
                    _ => o(p(S)),
                };
                a(result(out, Self::standard_type("ioerr")))
            }
            "sys.args" => a(s(p(S))),
            "sys.pid" => a(p(N)),
            "str.upper" | "str.lower" | "str.trim" | "str.split" | "fs.base" | "fs.join" => {
                for x in xs {
                    self.unify(x, &p(S))?;
                }
                match n {
                    "str.split" => s(p(S)),
                    "fs.base" => o(p(S)),
                    _ => p(S),
                }
            }
            "str.join" => {
                self.unify(&xs[0], &p(S))?;
                self.unify(&xs[1], &s(p(S)))?;
                p(S)
            }
            "rx.find" => {
                for x in xs {
                    self.unify(x, &p(S))?;
                }
                result(o(p(S)), Self::standard_type("rxerr"))
            }
            "dt.days" => {
                self.unify(&xs[0], &p(S))?;
                result(p(Z), Self::standard_type("dterr"))
            }
            "dt.date" => {
                self.unify(&xs[0], &p(Z))?;
                result(p(S), Self::standard_type("dterr"))
            }
            "json.encode" => result(p(S), Self::standard_type("jsonerr")),
            "json.decode" => {
                self.unify(&xs[0], &p(S))?;
                let Some(Type::Result(ok, _)) = want.map(|t| self.cx.resolve(t)) else {
                    return Err(self.err("json.decode needs a contextual success type annotation"));
                };
                if !self.cx.free_vars(&ok).is_empty() {
                    return Err(self.err("json.decode needs a concrete success type annotation"));
                }
                if !self.json_decodable(&ok, &mut HashSet::new()) {
                    return Err(self.err(format!("unsupported json.decode schema {ok}")));
                }
                result(*ok, Self::standard_type("jsonerr"))
            }
            "@" => {
                let out = want.cloned().unwrap_or_else(|| self.cx.fresh());
                self.unify(&xs[0], &f(xs[1].clone(), out.clone()))?;
                out
            }
            "@@" => {
                let Type::Tuple(args) = self.cx.resolve(&xs[1]) else {
                    return Err(
                        self.err("tuple application requires a statically known tuple bundle")
                    );
                };
                let out = want.cloned().unwrap_or_else(|| self.cx.fresh());
                self.unify(&xs[0], &function(args.into_iter(), out.clone()))?;
                out
            }
            "@/" => {
                let (item, dense) = self.sequence_or_dense_element(&xs[1])?;
                self.unify(&xs[0], &f(item.clone(), f(item.clone(), item.clone())))?;
                if dense {
                    Type::dense_array(o(item))
                } else {
                    o(item)
                }
            }
            "/:" | "\\:" => {
                let (item, dense) = self.sequence_or_dense_element(&xs[2])?;
                self.unify(&xs[0], &f(xs[1].clone(), f(item, xs[1].clone())))?;
                if dense {
                    Type::dense_array(xs[1].clone())
                } else if n == "/:" {
                    xs[1].clone()
                } else {
                    s(xs[1].clone())
                }
            }
            ".:" => match self.cx.resolve(&xs[1]) {
                Type::Seq(t) | Type::Table(t) => {
                    self.unify(&xs[0], &p(N))?;
                    o(*t)
                }
                Type::Dict(k, v) => {
                    self.unify(&xs[0], &k)?;
                    o(*v)
                }
                t if t.array_element().is_some() => {
                    self.unify(&xs[0], &s(p(N)))?;
                    o(t.array_element().unwrap().clone())
                }
                other => {
                    return Err(self.err(format!(
                        "lookup needs a known sequence, table or dictionary, found {other}"
                    )))
                }
            },
            "@:" | "?:" | "=:" | ">:" | "<:" => {
                let src = self.cx.resolve(&xs[1]);
                if let Some(item) = src.array_element().filter(|_| n == "@:") {
                    let out = self.cx.fresh();
                    self.unify(&xs[0], &f(item.clone(), out.clone()))?;
                    return Ok(Type::dense_array(out));
                }
                let item = match &src {
                    Type::Seq(x) | Type::Table(x) => x.as_ref().clone(),
                    Type::Dict(_, v) if n == "@:" => v.as_ref().clone(),
                    _ => return Err(self.err(format!("{n} reference needs a known iterable type"))),
                };
                let key = self.cx.fresh();
                if n == "?:" {
                    if !matches!(self.cx.resolve(&xs[0]), Type::Seq(_)) {
                        self.unify(&xs[0], &f(item, p(B)))?;
                    } else {
                        self.unify(&xs[0], &s(p(B)))?;
                    }
                    src
                } else {
                    self.unify(&xs[0], &f(item, key.clone()))?;
                    match n {
                        "@:" => match src {
                            Type::Dict(k, _) => Type::Dict(k, b(key)),
                            _ => s(key),
                        },
                        "=:" => {
                            self.req(Class::Hash, &key)?;
                            Type::Dict(b(key), b(src))
                        }
                        _ => {
                            self.req(Class::Ord, &key)?;
                            src
                        }
                    }
                }
            }
            "?1" => o(xs[0].clone()),
            "??" => {
                self.unify(&xs[1], &o(xs[0].clone()))?;
                xs[0].clone()
            }
            "+" | "-" | "*" | "/" | "//" | "%" | "^" => {
                self.numeric_binary(n, &xs[0], &xs[1], want)?
            }
            ".." => {
                if xs
                    .iter()
                    .any(|t| matches!(self.cx.resolve(t), Type::Seq(_)))
                {
                    return Err(self.err("range bounds must be scalar exact integers"));
                }
                let w = match want.map(|t| self.cx.resolve(t)) {
                    Some(Type::Seq(t)) => Some(*t),
                    _ => None,
                };
                s(self.numeric_binary(n, &xs[0], &xs[1], w.as_ref())?)
            }
            "==" => {
                let a = self.cx.resolve(&xs[0]);
                let b = self.cx.resolve(&xs[1]);
                match (a.array_element(), b.array_element()) {
                    (Some(a), Some(b)) => self.binary_rel(n, a, b)?,
                    _ => self.binary_rel(n, &a, &b)?,
                }
            }
            "<" | "<=" | ">" | ">=" | "=" | "/=" => self.binary_rel(n, &xs[0], &xs[1])?,
            "~" => {
                self.unify(&xs[0], &p(B))?;
                p(B)
            }
            "&&" | "||" => {
                fn bool_shape(t: &Type) -> Type {
                    if let Type::Seq(t) = t {
                        s(bool_shape(t))
                    } else {
                        p(Prim::B)
                    }
                }
                let a = self.cx.resolve(&xs[0]);
                let b = self.cx.resolve(&xs[1]);
                if [&a, &b]
                    .iter()
                    .any(|t| matches!(t, Type::Seq(el) if matches!(el.as_ref(), Type::Seq(_))))
                {
                    return Err(self.err("logical lifting is limited to one sequence dimension"));
                }
                let ash = bool_shape(&a);
                let bsh = bool_shape(&b);
                self.unify(&a, &ash)?;
                self.unify(&b, &bsh)?;
                fn depth(t: &Type) -> usize {
                    if let Type::Seq(t) = t {
                        1 + depth(t)
                    } else {
                        0
                    }
                }
                if depth(&ash) >= depth(&bsh) {
                    ash
                } else {
                    bsh
                }
            }
            "-:" | "|" | "^/" => {
                self.req(Class::Num, &xs[0])?;
                match (n, self.cx.resolve(&xs[0])) {
                    ("^/", _) => p(R),
                    ("-:", Type::Prim(N)) => p(Z),
                    ("-:", t @ Type::Var(_)) => {
                        self.require_exact_numeric(&t, "negation")?;
                        self.cx.resolve(&t)
                    }
                    (_, t) => t,
                }
            }
            ":n" | ":z" | ":q" | ":d" | ":r" => {
                self.req(Class::Num, &xs[0])?;
                p(match n {
                    ":n" => N,
                    ":z" => Z,
                    ":q" => Q,
                    ":d" => D,
                    _ => R,
                })
            }
            "#" => match self.cx.resolve(&xs[0]) {
                Type::Seq(_) | Type::Table(_) | Type::Dict(..) | Type::Prim(S) => p(N),
                t if t.array_element().is_some() => p(N),
                Type::Var(_) => {
                    return Err(self.unsupported(
                        "cardinality on an unknown collection shape requires annotation",
                    ))
                }
                t => return Err(self.err(format!("cannot take cardinality of {t}"))),
            },
            "+/" | "*/" | "avg" | "med" | "var" | "sd" | ">/" | "</" | "&&/" | "||/" => {
                let (elt, dense) = if matches!(n, "avg" | "med" | "var" | "sd") {
                    (self.seq_element(&xs[0])?, false)
                } else {
                    self.sequence_or_dense_element(&xs[0])?
                };
                let ty = if matches!(n, "&&/" | "||/") {
                    self.unify(&elt, &p(B))?;
                    p(B)
                } else {
                    self.req(Class::Num, &elt)?;
                    match n {
                        "avg" | "med" | "var" => {
                            self.require_exact_numeric(&elt, n)?;
                            o(p(Q))
                        }
                        "sd" => o(p(R)),
                        ">/" | "</" => o(elt),
                        _ => elt,
                    }
                };
                if dense {
                    Type::dense_array(ty)
                } else {
                    ty
                }
            }
            "pct" => {
                self.require_exact_numeric(&xs[0], n)?;
                let t = self.seq_element(&xs[1])?;
                self.require_exact_numeric(&t, n)?;
                o(p(Q))
            }
            "?+" | "?!" => {
                let t = self.cx.fresh();
                self.unify(&xs[0], &o(t.clone()))?;
                if n == "?+" {
                    p(B)
                } else {
                    t
                }
            }
            "csv.read" => {
                self.unify(&xs[0], &p(S))?;
                self.csv_read_type(want)?
            }
            "csv.write" => {
                self.unify(&xs[0], &p(S))?;
                let Type::Table(row) = self.cx.resolve(&xs[1]) else {
                    return Err(self.err("csv.write expects a statically known table schema"));
                };
                self.csv_row(&row)?;
                a(result(p(Unit), Self::standard_type("csverr")))
            }
            "df.agg" => {
                let src = self.cx.resolve(&xs[2]);
                let Type::Table(row) = &src else {
                    return Err(self.err("df.agg expects a statically known table schema"));
                };
                let key = self.cx.fresh();
                self.unify(&xs[0], &f(row.as_ref().clone(), key.clone()))?;
                self.req(Class::Hash, &key)?;
                let output = match want.map(|w| self.cx.resolve(w)) {
                    Some(Type::Table(row)) => *row,
                    _ => self.cx.fresh(),
                };
                self.unify(&xs[1], &f(key, f(src, output.clone())))?;
                self.row(&output)?;
                Type::Table(b(output))
            }
            "df.extend" => {
                let Type::Table(row) = self.cx.resolve(&xs[1]) else {
                    return Err(self.err("df.extend expects a statically known table schema"));
                };
                let out = self.row(&row)?;
                self.extend_columns(out, &xs[0])?
            }
            "io.print" | "io.println" => a(result(p(Unit), Self::standard_type("ioerr"))),
            "io.read" | "io.exists" | "io.write" => {
                for path in xs {
                    self.unify(path, &p(S))?;
                }
                let ok = match n {
                    "io.read" => p(S),
                    "io.exists" => p(B),
                    _ => p(Unit),
                };
                a(result(ok, Self::standard_type("ioerr")))
            }
            "math.quantize" => {
                self.unify(&xs[0], &p(N))?;
                self.require_exact_numeric(&xs[1], n)?;
                p(D)
            }
            "math.div" => {
                for x in xs {
                    self.require_exact_numeric(x, "math.div")?;
                }
                result(p(Q), Self::standard_type("numerr"))
            }
            "[:" | ":]" | "sect" | "rev" => {
                let (ns, last) = xs.split_at(xs.len() - 1);
                for i in ns {
                    self.unify(i, &p(N))?;
                }
                match self.cx.resolve(&last[0]) {
                    t @ (Type::Seq(_) | Type::Table(_) | Type::Prim(S)) => t,
                    Type::Var(_) => {
                        return Err(self.unsupported(format!("{n} needs a known collection shape")))
                    }
                    t => {
                        return Err(
                            self.err(format!("{n} expects sequence, table, or string; found {t}"))
                        )
                    }
                }
            }
            ",." | "|:" => {
                let input = self.cx.resolve(&xs[0]);
                if let Some(elem) = input.array_element() {
                    return Ok(if n == ",." { s(elem.clone()) } else { input });
                }
                let el = self.cx.fresh();
                self.unify(&xs[0], &s(s(el.clone())))?;
                if n == ",." {
                    s(el)
                } else {
                    s(s(el))
                }
            }
            "=." => {
                let el = self.seq_element(&xs[0])?;
                self.req(Class::Eq, &el)?;
                s(el)
            }
            "#:" => {
                let source = self.cx.resolve(&xs[0]);
                let row =
                    match source {
                        Type::Seq(row)
                            if matches!(
                                row.as_ref(),
                                Type::Record(_) | Type::Nominal(_) | Type::Tuple(_)
                            ) =>
                        {
                            *row
                        }
                        Type::Record(_) | Type::Nominal(_) => self.column_record(&source)?,
                        _ => return Err(self.err(
                            "#: reference needs a known sequence of rows or record-of-columns type",
                        )),
                    };
                Type::Table(b(row))
            }
            ":#" => {
                let row = self.cx.fresh();
                self.unify(&xs[0], &Type::Table(b(row.clone())))?;
                s(row)
            }
            "+:" => {
                self.unify(&xs[0], &xs[1])?;
                self.collection(&xs[0])?;
                xs[0].clone()
            }
            "&:" => {
                let a = self.seq_element(&xs[0])?;
                let b = self.seq_element(&xs[1])?;
                s(Type::Tuple(vec![a, b]))
            }
            "*:" => match (self.cx.resolve(&xs[0]), self.cx.resolve(&xs[1])) {
                (Type::Table(a), Type::Table(b1)) => Type::Table(b(Type::Tuple(vec![*a, *b1]))),
                (Type::Seq(a), Type::Seq(b1)) => s(Type::Tuple(vec![*a, *b1])),
                _ => {
                    return Err(
                        self.err("*: expects two sequences or two tables of compatible kinds")
                    )
                }
            },
            n if n.starts_with('.') && n != ".:" => self.select(&n[1..], &xs[0])?,
            n => return Err(self.unsupported(format!("{n} is not covered by Draft 0.4 inference"))),
        })
    }
}
