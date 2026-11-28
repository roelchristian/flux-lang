//! Dispatch statically checked primitive operations and their operand evidence.
use super::{evidence::inner_type, Eval};
use crate::{
    hir::{CoreOp, LiftMode, Span, TableSchema},
    hir_eval::{
        array, array_sequences, as_bool, as_seq, collection, collections, column_rows, combinators,
        decimal, domain, effects, equal, error, exact_index, exact_unary, finite, invariant, joins,
        json, key_with_budget, layout, lift, lift_one, math, named, native, numeric_reduce, order,
        packed, range_with_fuel, real_of, reduction, relations, scalar_binary, stats, to_exact,
        wide, wrap, Action, Domain, Exact, KEY_WORK_LIMIT, R, V,
    },
    types::{Prim, Type},
};
use std::{cmp::Ordering, collections::HashMap, rc::Rc};

/// Builtins whose evaluator needs checked input types, not only the result
/// type and lift plan. Direct calls and saturated references share this path.
pub(super) enum WitnessedOp {
    Dense(CoreOp),
    Indexed(CoreOp),
    Join(CoreOp),
    Relation(CoreOp),
    Combinator(CoreOp),
    ArrayOverload(CoreOp, usize),
}

pub(super) fn witnessed_op(op: CoreOp) -> Option<WitnessedOp> {
    Some(if array::handles(op) {
        WitnessedOp::Dense(op)
    } else if array_sequences::handles(op) {
        WitnessedOp::Indexed(op)
    } else if joins::handles(op) {
        WitnessedOp::Join(op)
    } else if relations::handles(op) {
        WitnessedOp::Relation(op)
    } else if combinators::handles(op) {
        WitnessedOp::Combinator(op)
    } else {
        let index = array::overloaded_operand(op)?;
        WitnessedOp::ArrayOverload(op, index)
    })
}

pub(in crate::hir_eval) fn operation_arity(op: CoreOp, span: Span) -> R<usize> {
    crate::builtins::for_operation(op)
        .map(|m| m.arity)
        .ok_or_else(|| {
            invariant(
                span,
                format!("typed operation {op:?} has no registry identity"),
            )
        })
}

impl Eval<'_> {
    fn group(&mut self, f: V, xs: &[V], span: Span) -> R<Vec<(V, Vec<V>)>> {
        let mut groups: Vec<(V, Vec<V>)> = Vec::new();
        let mut index = HashMap::new();
        let mut budget = KEY_WORK_LIMIT;
        for row in xs {
            let k = self.apply(f.clone(), vec![row.clone()], span)?;
            let key = key_with_budget(&k, &mut budget, 0).ok_or_else(|| {
                error(
                    span,
                    "group key exceeds supported hash complexity or resource limit",
                )
            })?;
            let i = *index.entry(key).or_insert(groups.len());
            if let Some(g) = groups.get_mut(i) {
                g.1.push(row.clone())
            } else {
                groups.push((k, vec![row.clone()]))
            }
        }
        Ok(groups)
    }
    pub(super) fn witnessed_core(
        &mut self,
        op: WitnessedOp,
        args: Vec<V>,
        inputs: &[Type],
        output: &Type,
        schema: Option<&Rc<TableSchema>>,
        span: Span,
    ) -> R<V> {
        match op {
            WitnessedOp::Dense(core) | WitnessedOp::Indexed(core) => {
                self.tick(span)?;
                if args.len() != operation_arity(core, span)? || inputs.len() != args.len() {
                    return Err(invariant(span, "invalid checked array arity"));
                }
                if matches!(op, WitnessedOp::Dense(_)) {
                    array::apply(core, &args, inputs, output, self, span)
                } else {
                    array_sequences::apply(core, &args, inputs, self, span)
                }
            }
            WitnessedOp::Join(core) | WitnessedOp::Relation(core) => {
                if args.len() != operation_arity(core, span)? || inputs.len() != args.len() {
                    return Err(invariant(span, "invalid checked relation arity"));
                }
                if matches!(op, WitnessedOp::Join(_)) {
                    return joins::apply(core, args, inputs, output, schema, self, span);
                }
                self.tick(span)?;
                let concrete = (core == CoreOp::TableSelect)
                    .then(|| self.p.instance_schema(output))
                    .flatten();
                relations::apply(
                    core,
                    &args,
                    inputs,
                    concrete.as_ref().or(schema),
                    self,
                    span,
                )
            }
            WitnessedOp::Combinator(core) => {
                combinators::apply(core, args, inputs, output, self, span)
            }
            WitnessedOp::ArrayOverload(core, index) => {
                if inputs.get(index).and_then(Type::array_element).is_some() {
                    self.tick(span)?;
                    array::apply(core, &args, inputs, output, self, span)
                } else if core == CoreOp::Reduce {
                    reduction::apply(&args, inputs, output, self, span)
                } else {
                    self.core(core, args, output, schema, None, span)
                }
            }
        }
    }

    pub(in crate::hir_eval) fn core(
        &mut self,
        op: CoreOp,
        args: Vec<V>,
        ty: &Type,
        schema: Option<&Rc<TableSchema>>,
        mode: Option<LiftMode>,
        span: Span,
    ) -> R<V> {
        use CoreOp::*;
        self.tick(span)?;
        if args.len() != operation_arity(op, span)? {
            return Err(error(span, "invalid typed HIR operator arity"));
        }
        if let Some(work) =
            packed::dense_work(&args, mode).map_err(|message| error(span, message))?
        {
            self.charge_work(work, span)?;
            if let Some(result) = packed::binary(op, &args, ty, mode, span)
                .or_else(|| packed::unary(op, &args, ty, mode, span))
            {
                return result;
            }
        }
        if let Some(result) = effects::apply_core(op, &args, ty, span, self.p) {
            return result;
        }
        if let Some(result) = collections::apply(op, &args, ty, schema, self, span) {
            return result;
        }
        if let Some(result) = native::apply_core(op, &args, span, self.p) {
            return result;
        }
        if let Some(result) = json::apply(op, &args, ty, span, self.p) {
            return result;
        }
        let mut args = args.into_iter();
        let mut a = || args.next().expect("arity checked");
        let fail = |message| error(span, message);
        let result = match op {
            Add | Sub | Mul | Div | Quotient | Remainder | Pow | Equal | NotEqual | Lt | Le
            | Gt | Ge | And | Or => lift(a(), a(), mode, &|x, y| {
                scalar_binary(op, x, y, inner_type(ty))
            }),
            Sqrt => lift_one(a(), mode, &|v| {
                if matches!(v, V::Wide(_)) {
                    return finite(wide::sqrt(&wide::ratio(&v)?)?);
                }
                let x = real_of(&v)?;
                if x < 0.0 {
                    Err("square root of a negative number")
                } else {
                    finite(x.sqrt())
                }
            }),
            IoRead | IoWrite | IoExists | IoPrint | CheckedDiv => {
                return Err(invariant(span, "checked operation has no implementation"))
            }
            Concat | Distinct | Lookup | Zip | Cartesian | Take | Drop | Slice | Reverse
            | Flatten | Transpose | Variance | Percentile | Min | Max | All | Any | Fold | Scan => {
                return Err(invariant(
                    span,
                    "checked collection operation has no implementation",
                ))
            }
            Reduce => return Err(invariant(span, "checked reduce requires operand evidence")),
            ArrayReshape | ArrayForm | ArrayShape | ArrayRavel | ArrayAt | ArrayMap | ArrayZip
            | ArrayOuter | ArrayRankMap | ArrayScan | ArrayWindows | ArraySelect | ArrayGrade
            | ArrayPermute | ArrayAxes | ArraySection | ArrayReduce | ArrayFold | ArrayScanAxis
            | ArrayDot | ArrayMatMul | ArrayDiag => {
                return Err(invariant(
                    span,
                    "checked array operation requires operand evidence",
                ));
            }
            StructuralRows => match a() {
                value @ V::Table(_, _) => return Ok(value),
                _ => return Err(invariant(span, "checked structural view expects a table")),
            },
            InnerJoin | LeftJoin | SemiJoin | AntiJoin | TableUnion | TableDistinct
            | TableSelect | DropNa | Fill | PresentOnly => {
                return Err(invariant(
                    span,
                    "checked relation requires operand evidence",
                ));
            }
            OptMap | OptBind | OptResult | ResMap | ResMapErr | ResCollect | ResPartition
            | ResOk | ResOptional | ReadLine | FsList | FsMkdir | FsBase | FsJoin | SysArgs
            | SysEnv | SysPid | StrUpper | StrLower | StrTrim | StrSplit | StrJoin | RxFind
            | DtDays | DtDate | JsonEncode | JsonDecode => {
                return Err(invariant(
                    span,
                    "checked standard library operation has no implementation",
                ));
            }
            EqualWhole => Ok(V::Bool(equal(&a(), &a()))),
            Negate | Abs | Not => lift_one(a(), mode, &|v| {
                if op == Not {
                    return Ok(V::Bool(!as_bool(&v)?));
                }
                if *inner_type(ty) == Type::Prim(Prim::R) {
                    let V::Real(n) = v else {
                        return Err("expected finite real operand");
                    };
                    return finite(if op == Abs { n.abs() } else { -n });
                }
                let k = domain(inner_type(ty))
                    .ok_or("numeric domain is unresolved at typed execution")?;
                exact_unary(op, &v, k)
            }),
            Count => {
                let count = match a() {
                    V::Sequence(v) | V::Table(v, _) => v.len(),
                    V::Dict(v) => v.len(),
                    V::String(v) => v.chars().count(),
                    _ => {
                        return Err(error(
                            span,
                            "count expects a sequence or string in this backend",
                        ))
                    }
                };
                Ok(V::Exact(Exact::integer(count as i128, Domain::Natural)))
            }
            Range => {
                let d =
                    domain(ty).ok_or_else(|| error(span, "range has no checked numeric domain"))?;
                range_with_fuel(&a(), &a(), d, Some(&mut self.fuel))
            }
            Sum | Product => {
                let xs = a();
                let xs = as_seq(&xs).map_err(fail)?;
                numeric_reduce(op, xs.iter().cloned(), ty)
            }
            Apply => return self.apply(a(), vec![a()], span),
            ApplyTuple => {
                let mut f = a();
                let xs = a();
                let V::Tuple(xs) = xs else {
                    return Err(error(span, "@@ expects a tuple bundle"));
                };
                for operand in xs.iter() {
                    f = self.apply(f, vec![operand.clone()], span)?;
                }
                return Ok(f);
            }
            Map => {
                let (f, v) = (a(), a());
                if let V::Dict(entries) = v {
                    let mut out = Vec::with_capacity(entries.len());
                    for (k, v) in entries.iter() {
                        out.push((k.clone(), self.apply(f.clone(), vec![v.clone()], span)?))
                    }
                    return Ok(V::Dict(out.into()));
                }
                let (xs, _) = collection(v, span)?;
                let mut out = Vec::with_capacity(xs.len());
                for v in xs.iter() {
                    out.push(self.apply(f.clone(), vec![v.clone()], span)?)
                }
                Ok(V::Sequence(out.into()))
            }
            TableRows | TableFrom => {
                let v = a();
                let schema = layout(schema, span)?;
                if let V::Sequence(rows) = v {
                    return Ok(V::Table(rows, schema));
                }
                if op == TableRows {
                    return Err(error(span, "table rows require a sequence"));
                }
                let rows = column_rows(&v, span)?
                    .into_iter()
                    .map(|r| V::Record(r.into()))
                    .collect::<Vec<_>>();
                Ok(V::Table(rows.into(), schema))
            }
            Rows => match a() {
                V::Table(rows, _) => Ok(V::Sequence(rows)),
                _ => Err(":# expects a table"),
            },
            Filter => {
                let (predicate, v) = (a(), a());
                let (xs, shape) = collection(v, span)?;
                let mut out = Vec::new();
                if let V::Sequence(mask) = predicate {
                    if mask.len() != xs.len() {
                        return Err(error(span, "mask length differs from collection length"));
                    }
                    for (x, b) in xs.iter().zip(mask.iter()) {
                        if as_bool(b).map_err(fail)? {
                            out.push(x.clone())
                        }
                    }
                } else {
                    for x in xs.iter() {
                        if as_bool(&self.apply(predicate.clone(), vec![x.clone()], span)?)
                            .map_err(fail)?
                        {
                            out.push(x.clone())
                        }
                    }
                }
                Ok(wrap(out, shape))
            }
            Group | Aggregate => {
                let key = a();
                let summarizer = if op == Aggregate { Some(a()) } else { None };
                let (xs, in_schema) = collection(a(), span)?;
                if xs.len() > 8192 {
                    return Err(error(span, "prototype grouping limit (8192 rows) exceeded"));
                }
                let groups = self.group(key, &xs, span)?;
                if let Some(f) = summarizer {
                    let schema = layout(schema, span)?;
                    let mut rows = Vec::with_capacity(groups.len());
                    for (key, items) in groups {
                        let p = self.apply(f.clone(), vec![key], span)?;
                        let row = self.apply(p, vec![wrap(items, in_schema.clone())], span)?;
                        rows.push(row);
                    }
                    Ok(V::Table(rows.into(), schema))
                } else {
                    Ok(V::Dict(
                        groups
                            .into_iter()
                            .map(|(k, items)| (k, wrap(items, in_schema.clone())))
                            .collect::<Vec<_>>()
                            .into(),
                    ))
                }
            }
            Extend => {
                let (new, source) = (a(), a());
                let schema = layout(schema, span)?;
                let (rows, input) = collection(source, span)?;
                let (Some(TableSchema::Named(source_fields)), TableSchema::Named(output_fields)) =
                    (input.as_deref(), schema.as_ref())
                else {
                    return Err(invariant(
                        span,
                        "df.extend needs checked named table layouts",
                    ));
                };
                let schema = if source_fields
                    .iter()
                    .zip(output_fields)
                    .all(|(a, b)| a.0 == b.0)
                {
                    schema
                } else {
                    // Column type evidence remains the checked output layout.
                    // Existing labels follow the checked source table's order.
                    let position = source_fields
                        .iter()
                        .enumerate()
                        .map(|(i, (name, _))| (name, i))
                        .collect::<HashMap<_, _>>();
                    let mut fields = output_fields.clone();
                    fields
                        .sort_by_key(|(name, _)| position.get(name).copied().unwrap_or(usize::MAX));
                    Rc::new(TableSchema::Named(fields))
                };
                let extra = column_rows(&new, span)?;
                if extra.len() != rows.len() {
                    return Err(error(
                        span,
                        "extension column length does not match table height",
                    ));
                }
                let mut out = Vec::with_capacity(rows.len());
                for (row, new) in rows.iter().zip(extra) {
                    let mut fs = named(row, span)?.to_vec();
                    for (k, v) in new {
                        if fs.iter().any(|(f, _)| *f == k) {
                            return Err(error(
                                span,
                                format!("df.extend cannot duplicate existing field {k}"),
                            ));
                        }
                        fs.push((k, v));
                    }
                    out.push(V::Record(fs.into()));
                }
                Ok(V::Table(out.into(), schema))
            }
            SortAsc | SortDesc => {
                let (f, source) = (a(), a());
                let (values, schema) = collection(source, span)?;
                let mut pairs = Vec::with_capacity(values.len());
                for v in values.iter() {
                    pairs.push((self.apply(f.clone(), vec![v.clone()], span)?, v.clone()))
                }
                if let Some((first, _)) = pairs.first() {
                    for (k, _) in &pairs {
                        order(first, k).map_err(fail)?;
                    }
                }
                pairs.sort_by(|(a, _), (b, _)| {
                    let cmp = order(a, b).unwrap_or(Ordering::Equal);
                    if op == SortAsc {
                        cmp
                    } else {
                        cmp.reverse()
                    }
                });
                Ok(wrap(pairs.into_iter().map(|(_, v)| v).collect(), schema))
            }
            Mean | Median | StdDev => return stats(op, a(), span),
            CsvRead => {
                let V::String(path) = a() else {
                    return Err(error(span, "csv.read requires a string path"));
                };
                Ok(V::Action(Rc::new(Action::CsvRead(path, schema.cloned()))))
            }
            CsvWrite => {
                let (path, t) = (a(), a());
                let V::String(path) = path else {
                    return Err(error(span, "csv.write requires a string path"));
                };
                Ok(V::Action(Rc::new(Action::CsvWrite(path, t))))
            }
            Execute => {
                if self.effects == 0 {
                    return Err(error(
                        span,
                        "execution is only permitted inside an effect block",
                    ));
                }
                return self.execute(a(), span);
            }
            Present => Ok(V::Optional(Some(Rc::new(a())))),
            Absent => Ok(V::Optional(None)),
            Has => {
                if let V::Optional(x) = a() {
                    Ok(V::Bool(x.is_some()))
                } else {
                    Err("presence test requires an optional")
                }
            }
            Unwrap => match a() {
                V::Optional(Some(v)) => Ok(v.as_ref().clone()),
                V::Optional(None) => Err("cannot unwrap ?0"),
                _ => Err("unwrap requires an optional"),
            },
            Default => match (a(), a()) {
                (_, V::Optional(Some(v))) => Ok(v.as_ref().clone()),
                (fallback, V::Optional(None)) => Ok(fallback),
                _ => Err("fallback requires an optional"),
            },
            Quantize => {
                let scale = exact_index(&a()).map_err(fail)?;
                decimal::quantize(&a(), scale)
            }
            MathFloor | MathCeil | MathTrunc | MathRound | MathSign | MathFrac | MathRecip
            | MathExp | MathExpm1 | MathLn | MathLog10 | MathLog1p | MathSin | MathCos
            | MathTan | MathAsin | MathAcos | MathAtan | MathSinh | MathCosh | MathTanh
            | MathAsinh | MathAcosh | MathAtanh | MathRad | MathDeg | MathLog | MathPow
            | MathHypot | MathAtan2 | MathRoot | MathPi | MathE | MathTau | MathGcd | MathLcm
            | MathFact | MathChoose | MathPerm | MathModPow | MathIsqrt | MathPrime
            | MathFactor | MathDivisors | MathFib | MathGamma | MathLgamma | MathErf | MathErfc
            | MathBeta | PolyEval | PolyAdd | PolySub | PolyMul | PolyDeriv | PolyInteg
            | PolyDegree | NumBisect | NumNewton | NumIntegrate | NumInterp | NumDiff => {
                return math::apply(op, args.as_slice(), ty, mode, self, span)
            }
            ToNatural | ToInteger | ToRational | ToDecimal | ToReal => lift_one(a(), mode, &|v| {
                if op == ToReal {
                    return finite(real_of(&v)?);
                }
                let v = to_exact(&v)?;
                let d = match op {
                    ToNatural => Domain::Natural,
                    ToInteger => Domain::Integer,
                    ToDecimal => Domain::Decimal,
                    _ => Domain::Rational,
                };
                if d == Domain::Decimal {
                    return decimal::decimal_from_exact(&v);
                }
                if let V::Exact(n) = v {
                    return Ok(V::Exact(n.as_domain(d)?));
                }
                wide::ratio_value(wide::ratio(&v)?, d)
            }),
        };
        result.map_err(fail)
    }
}
