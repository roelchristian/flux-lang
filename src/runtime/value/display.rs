//! Runtime value rendering for script output and REPL inspection.
use super::*;

#[derive(Clone, Copy)]
enum Style {
    Output,
    Inspect,
}

pub(super) fn repr(value: &V) -> String {
    render(value, Style::Output)
}

pub(super) fn inspect(value: &V) -> String {
    render(value, Style::Inspect)
}

fn render(value: &V, style: Style) -> String {
    let inspect = matches!(style, Style::Inspect);
    let show = |v: &V| render(v, style);
    let join = |xs: &[V], sep: &str| xs.iter().map(show).collect::<Vec<_>>().join(sep);
    match value {
        V::Exact(n) if n.d == 1 => n.n.to_string(),
        V::Exact(n) => format!("{}/{}", n.n, n.d),
        V::Wide(n) => format!("{n}"),
        V::Real(n) => n.to_string(),
        V::Bool(b) => if *b { "1b" } else { "0b" }.into(),
        V::String(s) if inspect => format!("{s:?}"),
        V::Char(c) if inspect => format!("{c:?}"),
        V::String(s) => s.to_string(),
        V::Char(c) => c.to_string(),
        V::Unit => "()".into(),
        V::Tuple(xs) => format!("({})", join(xs, ", ")),
        V::Dense(value) => value.render(show),
        V::Sequence(xs) => format!("[{}]", join(xs, " ")),
        V::Record(fs) => {
            let fields = fs
                .iter()
                .map(|(k, v)| format!("{k} := {}", show(v)))
                .collect::<Vec<_>>()
                .join("; ");
            if inspect {
                format!(".{{ {fields} }}")
            } else {
                format!("{{{fields}}}")
            }
        }
        V::Table(rows, schema) if inspect => table_view(rows, schema),
        V::Table(rows, schema) => format!(
            "#[{}]",
            rows.iter()
                .map(|row| repr_table_row(row, schema))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        V::Dict(entries) => {
            let fields = entries
                .iter()
                .map(|(k, v)| format!("{} : {}", show(k), show(v)))
                .collect::<Vec<_>>()
                .join("; ");
            if inspect {
                format!("#{{ {fields} }}")
            } else {
                format!("#{{{fields}}}")
            }
        }
        V::Optional(None) => "?0".into(),
        V::Optional(Some(x)) => format!("?1 {}", show(x)),
        V::Variant(variant) if variant.fields.is_empty() => variant.name.to_string(),
        V::Variant(variant) => format!("{} {}", variant.name, join(&variant.fields, " ")),
        V::Outcome(Ok(v)) => format!("ok {}", show(v)),
        V::Outcome(Err(f)) => format!("err {}", show(&f.payload)),
        V::Fun(_) => "<function>".into(),
        V::Action(_) => "<action>".into(),
    }
}

fn repr_table_row(row: &V, schema: &TableSchema) -> String {
    if let (V::Record(fields), TableSchema::Named(columns)) = (row, schema) {
        if fields
            .iter()
            .map(|(n, _)| n.as_ref())
            .ne(columns.iter().map(|(n, _)| n.as_str()))
        {
            let ordered = columns
                .iter()
                .map(|(name, _)| {
                    fields
                        .iter()
                        .find(|(key, _)| key.as_ref() == name)
                        .map(|(_, value)| format!("{name} := {}", repr(value)))
                })
                .collect::<Option<Vec<_>>>();
            if let Some(values) = ordered.filter(|_| fields.len() == columns.len()) {
                return format!("{{{}}}", values.join("; "));
            }
        }
    }
    repr(row)
}

fn table_view(rows: &[V], schema: &TableSchema) -> String {
    let headers = match schema {
        TableSchema::Named(fields) => fields.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>(),
        TableSchema::Positional(fields) => (1..=fields.len()).map(|i| format!(".{i}")).collect(),
    };
    if headers.is_empty() {
        return format!("#[ ]  ({} rows)", rows.len());
    }
    let safe = |text: String| {
        let snippet = text.chars().take(32).collect::<String>();
        let mut escaped = String::new();
        for c in snippet.chars() {
            if c.is_control() {
                escaped.extend(c.escape_default())
            } else {
                escaped.push(c)
            }
        }
        if snippet.len() < text.len() {
            escaped.push_str("...")
        }
        escaped
    };
    let mut display = vec![headers.iter().map(|h| safe(h.clone())).collect::<Vec<_>>()];
    for row in rows.iter().take(12) {
        let cells = match row {
            V::Record(fs) => headers
                .iter()
                .map(|key| {
                    fs.iter()
                        .find(|(k, _)| k.as_ref() == key)
                        .map(|(_, v)| safe(repr(v)))
                        .unwrap_or_else(|| "-".into())
                })
                .collect(),
            V::Tuple(xs) => xs.iter().map(|x| safe(repr(x))).collect(),
            _ => vec![safe(repr(row))],
        };
        display.push(cells);
    }
    let width = (0..headers.len())
        .map(|i| {
            display
                .iter()
                .filter_map(|r| r.get(i))
                .map(|x| x.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect::<Vec<_>>();
    let mut out = Vec::new();
    for (i, row) in display.into_iter().enumerate() {
        out.push(
            row.into_iter()
                .zip(&width)
                .map(|(s, w)| format!("{s:<width$}", width = *w))
                .collect::<Vec<_>>()
                .join(" | "),
        );
        if i == 0 {
            out.push(
                width
                    .iter()
                    .map(|w| "-".repeat(*w))
                    .collect::<Vec<_>>()
                    .join("-+-"),
            )
        }
    }
    if rows.len() > 12 {
        out.push(format!("... ({} rows total)", rows.len()))
    }
    out.join("\n")
}
