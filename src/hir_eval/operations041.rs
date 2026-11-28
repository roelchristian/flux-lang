//! Effect construction and typed recoverable failures for the 0.4.1 runtime.
use super::*;
use crate::{
    csv::{self, CsvFault, CsvFaultKind, RuntimeInput},
    errors041::{self, IoFault, IoKind},
};
use std::io::ErrorKind;

fn string(s: impl AsRef<str>) -> V {
    V::String(Rc::from(s.as_ref()))
}

fn natural(n: usize) -> V {
    exact_i128(n as i128, Domain::Natural)
}

pub(super) fn variant(
    program: &HirProgram,
    owner: &str,
    name: &str,
    fields: Vec<V>,
    span: Span,
) -> R<V> {
    let descriptor = errors041::lookup(owner, name)
        .ok_or_else(|| invariant(span, format!("unknown standard constructor {owner}.{name}")))?;
    if descriptor.payload.len() != fields.len() {
        return Err(invariant(
            span,
            format!("wrong field count for standard constructor {owner}.{name}"),
        ));
    }
    let constructor = program.standard_constructor(owner, name).ok_or_else(|| {
        error(
            span,
            format!("standard constructor {owner}.{name} missing from HIR"),
        )
    })?;
    Ok(V::Variant(Rc::new(VariantValue {
        constructor,
        name: Rc::from(name),
        fields: fields.into(),
    })))
}

pub(super) fn failure(payload: V, origin: Span, diagnostic: Option<String>) -> V {
    V::Outcome(Err(Failure::new(Rc::new(payload), origin, diagnostic)))
}

fn io_payload(program: &HirProgram, fault: &IoFault, span: Span) -> R<V> {
    variant(
        program,
        "ioerr",
        fault.kind.constructor(),
        vec![string(&fault.payload)],
        span,
    )
}

pub(super) fn on_io(program: &HirProgram, r: Result<V, IoFault>, span: Span) -> R<V> {
    match r {
        Ok(v) => Ok(succeeded(v)),
        Err(e) => Ok(failure(
            io_payload(program, &e, span)?,
            span,
            Some(e.diagnostic),
        )),
    }
}

enum CsvFailure {
    Io(IoFault),
    Format(CsvFault),
    Cell {
        row: usize,
        column: String,
        raw: String,
        detail: String,
    },
    Shape(&'static str),
}

fn on_csv(program: &HirProgram, result: Result<V, CsvFailure>, span: Span) -> R<V> {
    let (value, diagnostic) = match result {
        Ok(v) => return Ok(succeeded(v)),
        Err(CsvFailure::Io(fault)) => {
            let payload = io_payload(program, &fault, span)?;
            (
                variant(program, "csverr", "fileerr", vec![payload], span)?,
                fault.diagnostic,
            )
        }
        Err(CsvFailure::Format(fault)) => {
            let (name, value) = match fault.kind {
                CsvFaultKind::Header(s) => ("header", string(s)),
                CsvFaultKind::Duplicate(s) => ("duplicate", string(s)),
                CsvFaultKind::Absent(s) => ("absent", string(s)),
                CsvFaultKind::Row(n) => ("rowerr", natural(n)),
            };
            (
                variant(program, "csverr", name, vec![value], span)?,
                fault.diagnostic,
            )
        }
        Err(CsvFailure::Cell {
            row,
            column,
            raw,
            detail,
        }) => {
            let context = format!("CSV row {row}, column {column}: {detail}");
            (
                variant(
                    program,
                    "csverr",
                    "cellerr",
                    vec![natural(row), string(column), string(raw)],
                    span,
                )?,
                context,
            )
        }
        Err(CsvFailure::Shape(error)) => (
            variant(program, "csverr", "header", vec![string(error)], span)?,
            error.to_owned(),
        ),
    };
    Ok(failure(value, span, Some(diagnostic)))
}

fn csv_read(
    input: &mut dyn RuntimeInput,
    path: &str,
    schema: Option<&Rc<TableSchema>>,
) -> Result<V, CsvFailure> {
    let data = csv::read041_with_input(input, path).map_err(CsvFailure::Io)?;
    let Some(schema) = schema else {
        let cells = csv::decode041(&data).map_err(CsvFailure::Format)?;
        return Ok(V::Sequence(
            cells
                .into_iter()
                .map(|row| V::Sequence(row.into_iter().map(string).collect::<Vec<_>>().into()))
                .collect::<Vec<_>>()
                .into(),
        ));
    };
    let TableSchema::Named(names) = schema.as_ref() else {
        return Err(CsvFailure::Shape(
            "typed CSV read requires a named row schema",
        ));
    };
    let header: Vec<_> = names.iter().map(|(name, _)| name.clone()).collect();
    let cells = csv::typed_rows041(&data, &header).map_err(CsvFailure::Format)?;
    let rows = cells
        .into_iter()
        .enumerate()
        .map(|(i, cells)| {
            let row = names
                .iter()
                .zip(cells)
                .map(|((name, ty), cell)| {
                    let v = read_cell(&cell, ty).map_err(|detail| CsvFailure::Cell {
                        row: i + 2,
                        column: name.clone(),
                        raw: cell,
                        detail: detail.to_owned(),
                    })?;
                    Ok((name.as_str().into(), v))
                })
                .collect::<Result<Vec<_>, CsvFailure>>()?;
            Ok(V::Record(row.into()))
        })
        .collect::<Result<Vec<_>, CsvFailure>>()?;
    Ok(V::Table(rows.into(), schema.clone()))
}

fn csv_write(path: &str, value: &V) -> Result<V, CsvFailure> {
    let V::Table(rows, schema) = value else {
        return Err(CsvFailure::Shape("CSV write expects a table"));
    };
    let TableSchema::Named(fields) = schema.as_ref() else {
        return Err(CsvFailure::Shape(
            "CSV write requires a table with named columns",
        ));
    };
    let mut cells = Vec::with_capacity(rows.len() + 1);
    cells.push(fields.iter().map(|(name, _)| name.clone()).collect());
    for (i, row) in rows.iter().enumerate() {
        let V::Record(row) = row else {
            return Err(CsvFailure::Shape("CSV write requires record rows"));
        };
        let cells_in_row = fields
            .iter()
            .map(|(name, _)| {
                let cell = row
                    .iter()
                    .find(|(key, _)| key.as_ref() == name)
                    .map(|(_, v)| v)
                    .ok_or_else(|| CsvFailure::Cell {
                        row: i + 2,
                        column: name.clone(),
                        raw: String::new(),
                        detail: "record does not contain the schema field".to_owned(),
                    })?;
                write_cell(cell).map_err(|detail| CsvFailure::Cell {
                    row: i + 2,
                    column: name.clone(),
                    raw: match cell {
                        V::Optional(Some(v)) => repr(v),
                        v => repr(v),
                    },
                    detail: detail.to_owned(),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        cells.push(cells_in_row);
    }
    csv::write041(path, &cells).map_err(CsvFailure::Io)?;
    Ok(V::Unit)
}

pub(super) fn apply_action(action: &Action, eval: &mut Eval<'_>, span: Span) -> Option<R<V>> {
    let program = eval.p;
    Some(match action {
        Action::IoRead(path) => on_io(
            program,
            csv::read_text_with_input(eval.input, path, "text", csv::TEXT_MAX_BYTES).map(string),
            span,
        ),
        Action::IoWrite(path, content) => on_io(
            program,
            csv::write_text(path, content, "text", csv::TEXT_MAX_BYTES).map(|()| V::Unit),
            span,
        ),
        Action::IoExists(path) => on_io(
            program,
            match std::fs::metadata(path.as_ref()) {
                Ok(_) => Ok(V::Bool(true)),
                Err(e) if e.kind() == ErrorKind::NotFound => Ok(V::Bool(false)),
                Err(e) => Err(IoFault::system(path, e, false)),
            },
            span,
        ),
        Action::IoPrint(value) => {
            let line = repr(value);
            let result = eval
                .output
                .write_all(line.as_bytes())
                .and_then(|()| eval.output.write_all(b"\n"))
                .and_then(|()| eval.output.flush())
                .map(|()| V::Unit)
                .map_err(|e| IoFault::details(IoKind::Write, "stdout", format!("stdout: {e}")));
            on_io(program, result, span)
        }
        Action::CsvRead(path, schema) => {
            on_csv(program, csv_read(eval.input, path, schema.as_ref()), span)
        }
        Action::CsvWrite(path, value) => on_csv(program, csv_write(path, value), span),
        _ => return None,
    })
}

fn numeric_failure(program: &HirProgram, fault: &str, span: Span) -> R<V> {
    let name = if fault == DIV_BY_ZERO {
        "divzero"
    } else {
        "range"
    };
    Ok(failure(
        variant(program, "numerr", name, vec![], span)?,
        span,
        Some(fault.to_owned()),
    ))
}

pub(super) fn apply_core(
    op: CoreOp,
    args: &[V],
    _ty: &Type,
    span: Span,
    program: &HirProgram,
) -> Option<R<V>> {
    Some(match (op, args) {
        (CoreOp::IoRead, [V::String(path)]) => Ok(V::Action(Rc::new(Action::IoRead(path.clone())))),
        (CoreOp::IoWrite, [V::String(path), V::String(content)]) => Ok(V::Action(Rc::new(
            Action::IoWrite(path.clone(), content.clone()),
        ))),
        (CoreOp::IoExists, [V::String(path)]) => {
            Ok(V::Action(Rc::new(Action::IoExists(path.clone()))))
        }
        (CoreOp::IoPrint, [value]) => Ok(V::Action(Rc::new(Action::IoPrint(value.clone())))),
        (CoreOp::CheckedDiv, [a, b]) => match exact_binary(CoreOp::Div, a, b, Domain::Rational) {
            Ok(v) => Ok(succeeded(v)),
            // Keep the existing checked division range outcome at the
            // default precision. A host override is a runtime budget, not
            // a new recoverable source-level arithmetic result.
            Err(wide05::RESOURCE)
                if wide05::max_bits() != ResourcePolicy::default().max_exact_bits =>
            {
                Err(error(span, wide05::RESOURCE))
            }
            Err(e) => numeric_failure(program, e, span),
        },
        (
            CoreOp::IoRead
            | CoreOp::IoWrite
            | CoreOp::IoExists
            | CoreOp::IoPrint
            | CoreOp::CheckedDiv,
            _,
        ) => Err(invariant(span, "invalid operands for checked operation")),
        _ => return None,
    })
}
