//! Effect construction and typed recoverable failures in the standard library.
use super::{
    error, exact_binary, exact_i128, invariant, read_cell, repr, succeeded, wide, write_cell,
    Action, CoreOp, Domain, Eval, Failure, HirProgram, ResourcePolicy, Span, Table, TableSchema,
    Type, VariantValue, DIV_BY_ZERO, R, V,
};
use crate::{
    csv::{self, CsvFault, CsvFaultKind, RuntimeInput},
    host_fault::{IoFault, IoKind},
    standard_errors,
};
use std::{
    io::{ErrorKind, Write},
    rc::Rc,
};

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
    let descriptor = standard_errors::lookup(owner, name)
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
    let grid = csv::checked_typed_grid(&data, &header).map_err(CsvFailure::Format)?;
    let decode = |row, slot: usize| {
        let (name, ty) = &names[slot];
        let raw = grid.cell(row, slot);
        read_cell(raw, ty).map_err(|detail| CsvFailure::Cell {
            row: row + 2,
            column: name.clone(),
            raw: raw.to_owned(),
            detail: detail.to_owned(),
        })
    };
    if let Some(result) = Table::from_checked_cells(schema.clone(), grid.len(), decode) {
        return result;
    }
    // Physical layout admission may fail (or a wide exact value may need
    // more storage). Decode from the already validated input without
    // reopening it, preserving the original row-backed observable behavior.
    let rows = (0..grid.len())
        .map(|i| {
            let row = names
                .iter()
                .enumerate()
                .map(|(slot, (name, _))| Ok((name.as_str().into(), decode(i, slot)?)))
                .collect::<Result<Vec<_>, CsvFailure>>()?;
            Ok(V::Record(row.into()))
        })
        .collect::<Result<Vec<_>, CsvFailure>>()?;
    Ok(Table::from_rows(rows.into(), schema.clone()))
}

fn csv_write(path: &str, value: &V) -> Result<V, CsvFailure> {
    let V::Table(table) = value else {
        return Err(CsvFailure::Shape("CSV write expects a table"));
    };
    let TableSchema::Named(fields) = table.schema().as_ref() else {
        return Err(CsvFailure::Shape(
            "CSV write requires a table with named columns",
        ));
    };
    let mut cells = Vec::with_capacity(table.len() + 1);
    cells.push(fields.iter().map(|(name, _)| name.clone()).collect());
    for i in 0..table.len() {
        let row = if table.columnar() {
            None
        } else {
            match table.get(i) {
                Some(V::Record(row)) => Some(row),
                _ => return Err(CsvFailure::Shape("CSV write requires record rows")),
            }
        };
        let cells_in_row = fields
            .iter()
            .enumerate()
            .map(|(slot, (name, _))| {
                let missing = || CsvFailure::Cell {
                    row: i + 2,
                    column: name.clone(),
                    raw: String::new(),
                    detail: "record does not contain the schema field".to_owned(),
                };
                let projected;
                let cell = if let Some(row) = &row {
                    row.iter()
                        .find(|(key, _)| key.as_ref() == name)
                        .map(|(_, v)| v)
                        .ok_or_else(missing)?
                } else {
                    projected = table.cell(i, slot).ok_or_else(missing)?;
                    &projected
                };
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
    execute(action, eval.p, eval.input, eval.output, span)
}

fn execute(
    action: &Action,
    program: &HirProgram,
    input: &mut dyn RuntimeInput,
    output: &mut dyn Write,
    span: Span,
) -> Option<R<V>> {
    Some(match action {
        Action::IoRead(path) => on_io(
            program,
            csv::read_text_with_input(input, path, "text", csv::TEXT_MAX_BYTES).map(string),
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
            let result = output
                .write_all(line.as_bytes())
                .and_then(|()| output.write_all(b"\n"))
                .and_then(|()| output.flush())
                .map(|()| V::Unit)
                .map_err(|e| IoFault::details(IoKind::Write, "stdout", format!("stdout: {e}")));
            on_io(program, result, span)
        }
        Action::CsvRead(path, schema) => {
            on_csv(program, csv_read(input, path, schema.as_ref()), span)
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
            Err(wide::RESOURCE) if wide::max_bits() != ResourcePolicy::default().max_exact_bits => {
                Err(error(span, wide::RESOURCE))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        hir_eval::{resources, OpenedInput, RuntimeOptions},
        types::Prim,
    };

    struct Fixture<'a>(&'a str, usize);

    impl RuntimeInput for Fixture<'_> {
        fn open(&mut self, path: &str) -> std::io::Result<OpenedInput> {
            assert_eq!(path, "fixture.csv");
            self.1 += 1;
            Ok(OpenedInput {
                reader: Box::new(std::io::Cursor::new(self.0.as_bytes().to_vec())),
                byte_len: Some(self.0.len() as u64),
            })
        }
    }

    fn schema(fields: &[(&str, Type)]) -> Rc<TableSchema> {
        Rc::new(TableSchema::Named(
            fields
                .iter()
                .map(|(name, ty)| ((*name).into(), ty.clone()))
                .collect(),
        ))
    }

    #[test]
    fn typed_csv_builds_columns_directly_with_the_same_exact_values() {
        let input = concat!(
            "\u{feff}wide,enabled,offset,station,temp\r\n",
            "0,0b,1.20,\"North, Annex\",-0\r\n",
            "1361129467683753853853498429727072845824,,0.00,ridge,0\r\n",
        );
        let schema = schema(&[
            ("station", Type::Prim(Prim::S)),
            ("offset", Type::Prim(Prim::D)),
            ("temp", Type::Prim(Prim::R)),
            ("enabled", Type::Optional(Box::new(Type::Prim(Prim::B)))),
            ("wide", Type::Prim(Prim::Z)),
        ]);
        for optimize in [true, false] {
            let _scope = resources::enter(RuntimeOptions {
                optimize,
                ..Default::default()
            })
            .unwrap();
            let mut fixture = Fixture(input, 0);
            let Ok(V::Table(table)) = csv_read(&mut fixture, "fixture.csv", Some(&schema)) else {
                panic!("typed CSV must decode");
            };
            assert_eq!(fixture.1, 1);
            assert_eq!(table.columnar(), optimize);
            for (row, slot, expected) in [
                (0, 0, "North, Annex"),
                (0, 1, "1.20"),
                (1, 1, "0.00"),
                (0, 2, "-0"),
                (1, 2, "0"),
                (0, 3, "?1 0b"),
                (1, 3, "?0"),
                (1, 4, "1361129467683753853853498429727072845824"),
            ] {
                assert_eq!(repr(&table.cell(row, slot).unwrap()), expected);
            }
        }
    }

    #[test]
    fn typed_csv_finishes_structural_validation_before_any_scalar_decoding() {
        let schema = schema(&[
            ("name", Type::Prim(Prim::C)),
            ("value", Type::Prim(Prim::Z)),
        ]);
        for optimize in [true, false] {
            let _scope = resources::enter(RuntimeOptions {
                optimize,
                ..Default::default()
            })
            .unwrap();
            let mut fixture = Fixture("value,name\nbad,too-long\nmissing\n", 0);
            assert!(matches!(
                csv_read(&mut fixture, "fixture.csv", Some(&schema)),
                Err(CsvFailure::Format(CsvFault {
                    kind: CsvFaultKind::Row(3),
                    ..
                }))
            ));
            assert_eq!(fixture.1, 1);

            let mut fixture = Fixture("value,name\nbad,too-long\n", 0);
            assert!(matches!(
                csv_read(&mut fixture, "fixture.csv", Some(&schema)),
                Err(CsvFailure::Cell { row: 2, column, raw, .. })
                    if column == "name" && raw == "too-long"
            ));
            assert_eq!(fixture.1, 1);
        }
    }

    #[test]
    fn widening_under_a_tight_layout_budget_does_not_reopen_or_change_cell_fault() {
        let _scope = resources::enter(RuntimeOptions {
            resources: ResourcePolicy {
                max_memory: 80,
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();
        let ty = Type::Prim(Prim::Z);
        let schema = schema(&[("a", ty.clone()), ("b", ty)]);
        // Compact admission fits, but the temporary overlap required to
        // widen column a cannot fit; the row-backed decoder still sees b.
        let mut fixture = Fixture("a,b\n1,2\n9223372036854775808,nope\n", 0);
        assert!(matches!(
            csv_read(&mut fixture, "fixture.csv", Some(&schema)),
            Err(CsvFailure::Cell { row: 3, column, raw, .. })
                if column == "b" && raw == "nope"
        ));
        assert_eq!(fixture.1, 1);
        assert_eq!(resources::available_memory(), 80);
    }
}
