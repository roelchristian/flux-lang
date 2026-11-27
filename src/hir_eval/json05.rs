//! Typed JSON without a dynamic Flux value or a binary64 conversion path.
use super::operations041::{failure, success, variant};
use super::*;
use serde::de::{self, Deserialize, Deserializer, MapAccess, Visitor};
use serde_json::value::RawValue;

const MAX_BYTES: usize = 1 << 20;
const MAX_NODES: usize = 100_000;
const MAX_DEPTH: usize = 64;
const MAX_DIGITS: usize = 100_000;
const MAX_TOTAL_NUMBER_DIGITS: usize = 1_000_000;

#[derive(Debug)]
enum JsonError {
    Invalid(String),
    Shape(String),
}
type J<T> = Result<T, JsonError>;
fn shape(s: impl Into<String>) -> JsonError {
    JsonError::Shape(s.into())
}
fn invalid(e: serde_json::Error) -> JsonError {
    JsonError::Invalid(e.to_string())
}

struct Budget {
    nodes: usize,
    number_digits: usize,
}
impl Default for Budget {
    fn default() -> Self {
        Self {
            nodes: MAX_NODES,
            number_digits: MAX_TOTAL_NUMBER_DIGITS,
        }
    }
}
impl Budget {
    fn enter(&mut self, depth: usize) -> J<()> {
        if depth > MAX_DEPTH || self.nodes == 0 {
            return Err(shape("JSON nesting or node limit exceeded"));
        }
        self.nodes -= 1;
        Ok(())
    }

    fn number(&mut self, digits: usize) -> J<()> {
        self.number_digits = self
            .number_digits
            .checked_sub(digits)
            .ok_or_else(|| shape("JSON total numeric expansion exceeds the resource limit"))?;
        Ok(())
    }
}

/// Convert a JSON number to plain decimal notation, retaining decimal scale
/// after application of its exponent. The JSON parser validates token syntax.
fn plain_number(raw: &str, budget: &mut Budget) -> J<String> {
    let (mantissa, exponent) = match raw.split_once(['e', 'E']) {
        Some((m, e)) => (
            m,
            e.parse::<i64>()
                .map_err(|_| shape("JSON numeric exponent exceeds the resource limit"))?,
        ),
        None => (raw, 0),
    };
    let (negative, m) = match mantissa.strip_prefix('-') {
        Some(m) => (true, m),
        None => (false, mantissa),
    };
    let (whole, fractional) = m.split_once('.').unwrap_or((m, ""));
    let input_digits = whole.len() + fractional.len();
    if input_digits > MAX_DIGITS {
        return Err(shape("JSON numeric digits exceed the resource limit"));
    }
    let scale = (fractional.len() as i64)
        .checked_sub(exponent)
        .ok_or_else(|| shape("JSON numeric exponent exceeds the resource limit"))?;
    if scale.unsigned_abs() > MAX_DIGITS as u64 {
        return Err(shape("JSON numeric scale exceeds the resource limit"));
    }
    // Charge both input digits and exponent-driven padding before allocating
    // the expanded string or parsing a wide coefficient/denominator.
    let significant_whole = whole.trim_start_matches('0').len();
    let significant_digits = if significant_whole == 0 {
        fractional.trim_start_matches('0').len()
    } else {
        significant_whole + fractional.len()
    };
    let expanded_digits = if scale > 0 {
        significant_digits.max(scale as usize + 1)
    } else if significant_digits == 0 {
        1
    } else {
        significant_digits + (-scale) as usize
    };
    budget.number(input_digits.max(expanded_digits))?;

    let digits = format!("{whole}{fractional}");
    let unpadded = digits.trim_start_matches('0');
    let digits = if unpadded.is_empty() { "0" } else { unpadded };
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    if scale <= 0 {
        out.push_str(digits);
        if digits != "0" {
            out.extend(std::iter::repeat_n('0', (-scale) as usize));
        }
    } else {
        let scale = scale as usize;
        if scale >= digits.len() {
            out.push_str("0.");
            out.extend(std::iter::repeat_n('0', scale - digits.len()));
            out.push_str(digits);
        } else {
            let (left, right) = digits.split_at(digits.len() - scale);
            out.push_str(left);
            out.push('.');
            out.push_str(right);
        }
    }
    Ok(out)
}

fn number(raw: &str, kind: Prim, budget: &mut Budget) -> J<V> {
    if !raw
        .as_bytes()
        .first()
        .is_some_and(|x| *x == b'-' || x.is_ascii_digit())
    {
        return Err(shape("expected a JSON number"));
    }
    let plain = plain_number(raw, budget)?;
    if kind == Prim::D {
        return decimal05::decimal_literal(&plain).map_err(shape);
    }
    if kind == Prim::Q {
        return parse_exact_value(&plain, Domain::Rational).map_err(shape);
    }
    let d = if kind == Prim::N {
        Domain::Natural
    } else {
        Domain::Integer
    };
    if !plain.contains('.') {
        return parse_exact_value(&plain, d).map_err(shape);
    }
    let exact = parse_exact_value(&plain, Domain::Rational).map_err(shape)?;
    let ratio = wide05::ratio(&exact).map_err(shape)?;
    wide05::ratio_value(ratio, d).map_err(shape)
}

struct Object<'a>(Vec<(String, &'a RawValue)>);
impl<'de> Deserialize<'de> for Object<'de> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Fields;
        impl<'de> Visitor<'de> for Fields {
            type Value = Object<'de>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("JSON object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut items = Vec::new();
                while let Some(entry) = map.next_entry::<String, &'de RawValue>()? {
                    if items.len() == MAX_NODES {
                        return Err(de::Error::custom(
                            "JSON field count exceeds the resource limit",
                        ));
                    }
                    items.push(entry);
                }
                Ok(Object(items))
            }
        }
        d.deserialize_map(Fields)
    }
}

fn fields(
    raw: &str,
    schema: &[(String, Type)],
    program: &HirProgram,
    budget: &mut Budget,
    depth: usize,
) -> J<V> {
    if !raw.starts_with('{') {
        return Err(shape("expected a JSON object"));
    }
    let Object(entries) = serde_json::from_str(raw).map_err(invalid)?;
    let mut by_name = HashMap::with_capacity(entries.len());
    for (name, value) in entries {
        if by_name.insert(name.clone(), value).is_some() {
            return Err(shape(format!("duplicate JSON field {name:?}")));
        }
    }
    let mut out = Vec::with_capacity(schema.len());
    for (name, ty) in schema {
        let value = by_name
            .remove(name)
            .ok_or_else(|| shape(format!("missing JSON field {name:?}")))?;
        out.push((
            name.as_str().into(),
            read(value, ty, program, budget, depth + 1)?,
        ));
    }
    if !by_name.is_empty() {
        // Hash iteration order must not make diagnostics nondeterministic.
        let name = by_name.keys().min().expect("unexpected field");
        return Err(shape(format!("unexpected JSON field {name:?}")));
    }
    Ok(V::Record(out.into()))
}

fn read(
    raw: &RawValue,
    ty: &Type,
    program: &HirProgram,
    budget: &mut Budget,
    depth: usize,
) -> J<V> {
    budget.enter(depth)?;
    let raw_value = raw;
    let raw = raw_value.get();
    match ty {
        Type::Prim(Prim::Unit) if raw == "null" => Ok(V::Unit),
        Type::Prim(Prim::B) if raw == "true" => Ok(V::Bool(true)),
        Type::Prim(Prim::B) if raw == "false" => Ok(V::Bool(false)),
        Type::Prim(k @ (Prim::N | Prim::Z | Prim::Q | Prim::D)) => number(raw, *k, budget),
        Type::Prim(Prim::C | Prim::S) if raw.starts_with('"') => {
            let value = serde_json::from_str::<String>(raw).map_err(invalid)?;
            if *ty == Type::Prim(Prim::S) {
                Ok(V::String(value.into()))
            } else {
                let mut chars = value.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => Ok(V::Char(c)),
                    _ => Err(shape(
                        "expected exactly one Unicode scalar for a Flux character",
                    )),
                }
            }
        }
        Type::Optional(_) if raw == "null" => Ok(V::Optional(None)),
        Type::Optional(inner) => Ok(V::Optional(Some(Rc::new(read(
            raw_value,
            inner,
            program,
            budget,
            depth + 1,
        )?)))),
        Type::Seq(inner) if raw.starts_with('[') => {
            let values = serde_json::from_str::<Vec<&RawValue>>(raw).map_err(invalid)?;
            let out = values
                .into_iter()
                .map(|v| read(v, inner, program, budget, depth + 1))
                .collect::<J<Vec<_>>>()?;
            Ok(V::Sequence(out.into()))
        }
        Type::Tuple(types) if raw.starts_with('[') => {
            let values = serde_json::from_str::<Vec<&RawValue>>(raw).map_err(invalid)?;
            if values.len() != types.len() {
                return Err(shape(format!(
                    "expected {} JSON tuple elements, got {}",
                    types.len(),
                    values.len()
                )));
            }
            let out = values
                .into_iter()
                .zip(types)
                .map(|(v, t)| read(v, t, program, budget, depth + 1))
                .collect::<J<Vec<_>>>()?;
            Ok(V::Tuple(out.into()))
        }
        Type::Record(row) if row.tail.is_none() => fields(raw, &row.fields, program, budget, depth),
        Type::Nominal(_) => {
            // Reuse the checked row metadata, not record contents or name-based guesses.
            let schema = program
                .instance_schema(&Type::Table(Box::new(ty.clone())))
                .ok_or_else(|| shape("JSON decode requires a closed nominal record schema"))?;
            match schema.as_ref() {
                TableSchema::Named(schema) => fields(raw, schema, program, budget, depth),
                _ => Err(shape("JSON decode requires named record fields")),
            }
        }
        _ => Err(shape(format!("JSON value does not match Flux type {ty}"))),
    }
}

struct Encoder {
    out: String,
    budget: Budget,
}
impl Encoder {
    fn push(&mut self, value: &str) -> J<()> {
        if value.len() > MAX_BYTES - self.out.len() {
            return Err(shape("JSON encoded text exceeds the resource limit"));
        }
        self.out.push_str(value);
        Ok(())
    }
    fn quoted(&mut self, text: &str) -> J<()> {
        if text.len() > MAX_BYTES - self.out.len() {
            return Err(shape("JSON encoded text exceeds the resource limit"));
        }
        self.push(&serde_json::to_string(text).map_err(invalid)?)
    }
    fn value(&mut self, v: &V, depth: usize) -> J<()> {
        self.budget.enter(depth)?;
        match v {
            V::Bool(b) => self.push(if *b { "true" } else { "false" }),
            V::String(s) => self.quoted(s),
            V::Char(c) => self.quoted(c.encode_utf8(&mut [0; 4])),
            V::Unit | V::Optional(None) => self.push("null"),
            V::Optional(Some(value)) => {
                let start = self.out.len();
                self.value(value, depth + 1)?;
                if &self.out[start..] == "null" {
                    return Err(shape(
                        "JSON cannot distinguish this present optional from null",
                    ));
                }
                Ok(())
            }
            V::Exact(_) | V::Wide(_) => {
                let decimal = if matches!(v, V::Wide(w) if matches!(w.as_ref(), Wide::Decimal(_))) {
                    v.clone()
                } else {
                    let ratio = wide05::ratio(v).map_err(shape)?;
                    if ratio.is_integer() {
                        return self.push(&ratio.numer().to_string());
                    }
                    decimal05::decimal_from_exact(v).map_err(shape)?
                };
                let V::Wide(wide) = decimal else {
                    return Err(shape("invalid decimal representation"));
                };
                let Wide::Decimal(decimal) = wide.as_ref() else {
                    return Err(shape("invalid decimal representation"));
                };
                self.push(&decimal.to_string())
            }
            V::Tuple(items) | V::Sequence(items) => {
                self.push("[")?;
                for (i, item) in items.iter().enumerate() {
                    if i != 0 {
                        self.push(",")?;
                    }
                    self.value(item, depth + 1)?;
                }
                self.push("]")
            }
            V::Record(fields) => {
                self.push("{")?;
                for (i, (name, value)) in fields.iter().enumerate() {
                    if i != 0 {
                        self.push(",")?;
                    }
                    self.quoted(name)?;
                    self.push(":")?;
                    self.value(value, depth + 1)?;
                }
                self.push("}")
            }
            _ => Err(shape("Flux value has no supported JSON representation")),
        }
    }
}

fn outcome(value: J<V>, span: Span, program: &HirProgram) -> R<V> {
    match value {
        Ok(v) => Ok(success(v)),
        Err(e) => {
            let (name, message) = match e {
                JsonError::Invalid(s) => ("invalidjson", s),
                JsonError::Shape(s) => ("jsonshape", s),
            };
            let payload = variant(
                program,
                "jsonerr",
                name,
                vec![V::String(message.clone().into())],
                span,
            )?;
            Ok(failure(payload, span, Some(message)))
        }
    }
}

pub(super) fn apply(
    op: CoreOp,
    args: &[V],
    ty: &Type,
    span: Span,
    program: &HirProgram,
) -> Option<R<V>> {
    Some(match (op, args) {
        (CoreOp::JsonEncode, [v]) => {
            let mut encoder = Encoder {
                out: String::new(),
                budget: Budget::default(),
            };
            let result = encoder.value(v, 0).map(|()| V::String(encoder.out.into()));
            outcome(result, span, program)
        }
        (CoreOp::JsonDecode, [V::String(input)]) => {
            let Type::Result(target, _) = ty else {
                return Some(Err(invariant(
                    span,
                    "JSON decode success type missing from checked HIR",
                )));
            };
            let result = if input.len() > MAX_BYTES {
                Err(shape("JSON input exceeds the resource limit"))
            } else {
                serde_json::from_str::<&RawValue>(input)
                    .map_err(invalid)
                    .and_then(|raw| read(raw, target, program, &mut Budget::default(), 0))
            };
            outcome(result, span, program)
        }
        (CoreOp::JsonDecode | CoreOp::JsonEncode, _) => {
            Err(invariant(span, "malformed typed JSON call"))
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn json_numeric_spellings_are_lossless() {
        for (a, b) in [
            ("1.2300e2", "123.00"),
            ("1.2300E-2", "0.012300"),
            ("-0.000", "-0.000"),
            ("20e+2", "2000"),
        ] {
            assert_eq!(plain_number(a, &mut Budget::default()).unwrap(), b);
        }
        let mut budget = Budget::default();
        let mut number = |s, kind| super::number(s, kind, &mut budget);
        assert_eq!(repr(&number("12.000", Prim::Z).unwrap()), "12");
        assert!(number("12.001", Prim::Z).is_err());
        assert!(number("-1", Prim::N).is_err());
        let d = number("9007199254740993.2000", Prim::D).unwrap();
        let mut enc = Encoder {
            out: String::new(),
            budget: Budget::default(),
        };
        enc.value(&d, 0).unwrap();
        assert_eq!(enc.out, "9007199254740993.2000");
    }
    #[test]
    fn nonterminating_rationals_and_present_null_are_not_silently_encoded() {
        let mut enc = Encoder {
            out: String::new(),
            budget: Budget::default(),
        };
        let v = rational(BigInt::from(1), BigInt::from(3)).unwrap();
        assert!(matches!(enc.value(&v, 0), Err(JsonError::Shape(_))));
        assert!(matches!(
            enc.value(&V::Optional(Some(Rc::new(V::Unit))), 0),
            Err(JsonError::Shape(_))
        ));
    }
}
