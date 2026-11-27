use std::io::{Read, Write};

type Rt<T> = Result<T, String>;

const CSV_MAX_BYTES: u64 = 1_048_576;
const CSV_MAX_CELLS: usize = 100_000;
pub fn read(path: &str) -> Rt<String> {
    let err = |e: std::io::Error| format!("cannot read CSV {path:?}: {e}");
    let m = std::fs::metadata(path).map_err(err)?;
    if !m.is_file() {
        return Err(format!("CSV path {path:?} is not a regular file"));
    }
    if m.len() > CSV_MAX_BYTES {
        return Err(format!("CSV input exceeds {CSV_MAX_BYTES} bytes"));
    }
    let f = std::fs::File::open(path).map_err(err)?;
    if !f.metadata().map_err(err)?.is_file() {
        return Err(format!("CSV path {path:?} is not a regular file"));
    }
    let mut bytes = Vec::new();
    f.take(CSV_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(err)?;
    if bytes.len() as u64 > CSV_MAX_BYTES {
        return Err(format!("CSV input exceeds {CSV_MAX_BYTES} bytes"));
    }
    String::from_utf8(bytes).map_err(|_| format!("CSV {path:?} is not valid UTF-8"))
}
pub fn decode(s: &str) -> Rt<Vec<Vec<String>>> {
    #[derive(PartialEq, Eq)]
    enum State {
        Start,
        Text,
        Quoted,
        Closed,
    }
    let s = s.strip_prefix('\u{feff}').unwrap_or(s);
    let mut state = State::Start;
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut open = false;
    let mut count = 0usize;
    let mut chars = s.chars().peekable();
    let mut add_field = |row: &mut Vec<String>, field: &mut String| -> Rt<()> {
        count += 1;
        if count > CSV_MAX_CELLS {
            return Err(format!("CSV input exceeds {CSV_MAX_CELLS} cells"));
        }
        row.push(std::mem::take(field));
        Ok(())
    };
    fn add_row(rows: &mut Vec<Vec<String>>, row: &mut Vec<String>) -> Rt<()> {
        if let Some(first) = rows.first() {
            if row.len() != first.len() {
                return Err(format!(
                    "record {} has {} fields; expected {} (first mismatched column {})",
                    rows.len() + 1,
                    row.len(),
                    first.len(),
                    row.len().min(first.len()) + 1
                ));
            }
        }
        rows.push(std::mem::take(row));
        Ok(())
    }
    while let Some(c) = chars.next() {
        open = true;
        if state == State::Quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    state = State::Closed;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            ',' => {
                add_field(&mut row, &mut field)?;
                state = State::Start;
            }
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                add_field(&mut row, &mut field)?;
                add_row(&mut rows, &mut row)?;
                state = State::Start;
                open = false;
            }
            '"' if state == State::Start => state = State::Quoted,
            '"' => {
                return Err(format!(
                    "unexpected quote in record {}, field {}",
                    rows.len() + 1,
                    row.len() + 1
                ))
            }
            _ if state == State::Closed => {
                return Err(format!(
                    "unexpected content after closing quote in record {}, field {}",
                    rows.len() + 1,
                    row.len() + 1
                ))
            }
            _ => {
                field.push(c);
                state = State::Text;
            }
        }
    }
    if state == State::Quoted {
        return Err(format!(
            "unclosed quoted field in record {}, field {}",
            rows.len() + 1,
            row.len() + 1
        ));
    }
    if open {
        add_field(&mut row, &mut field)?;
        add_row(&mut rows, &mut row)?;
    }
    Ok(rows)
}

/// Parse records and select columns in schema order, without returning the header.
pub fn typed_rows(s: &str, ordered_header: &[String]) -> Rt<Vec<Vec<String>>> {
    use std::collections::{HashMap, HashSet};
    let mut rows = decode(s)?.into_iter();
    let header = rows
        .next()
        .ok_or("CSV row 1: expected header, found empty input")?;
    let mut columns = HashMap::new();
    for (i, name) in header.iter().enumerate() {
        if let Some(first) = columns.insert(name.as_str(), i) {
            return Err(format!(
                "CSV row 1, column {}: duplicate header {name:?} (first seen in column {})",
                i + 1,
                first + 1
            ));
        }
    }
    let mut fields = HashSet::new();
    let mut order = Vec::with_capacity(ordered_header.len());
    for (i, name) in ordered_header.iter().enumerate() {
        if !fields.insert(name.as_str()) {
            return Err(format!(
                "CSV schema column {}: duplicate name {name:?}",
                i + 1
            ));
        }
        order.push(*columns.get(name.as_str()).ok_or_else(|| {
            format!(
                "CSV row 1, schema column {}: missing header {name:?}",
                i + 1
            )
        })?);
    }
    for (i, name) in header.iter().enumerate() {
        if !fields.contains(name.as_str()) {
            return Err(format!(
                "CSV row 1, column {}: unexpected header {name:?}",
                i + 1
            ));
        }
    }
    Ok(rows
        .map(|row| {
            let mut row = row.into_iter().map(Some).collect::<Vec<_>>();
            order.iter().map(|&i| row[i].take().unwrap()).collect()
        })
        .collect())
}

/// Encode a rectangular grid. CRLF separates records; embedded line endings are preserved.
pub fn encode(rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    for (i, row) in rows.iter().enumerate() {
        for (j, field) in row.iter().enumerate() {
            if j != 0 {
                out.push(',');
            }
            let quote = field.contains([',', '"', '\r', '\n'])
                || (i == 0 && j == 0 && field.starts_with('\u{feff}'));
            if quote {
                out.push('"');
                for ch in field.chars() {
                    if ch == '"' {
                        out.push('"');
                    }
                    out.push(ch);
                }
                out.push('"');
            } else {
                out.push_str(field);
            }
        }
        out.push_str("\r\n");
    }
    out
}

/// Write a new regular file; never truncate an existing path.
pub fn write(path: &str, rows: &[Vec<String>]) -> Rt<()> {
    let width = rows.first().map_or(0, Vec::len);
    let mut cells = 0;
    for (i, row) in rows.iter().enumerate() {
        if row.is_empty() || row.len() != width {
            return Err(format!(
                "CSV row {}: expected {} fields, found {}",
                i + 1,
                width.max(1),
                row.len()
            ));
        }
        cells += row.len();
        if cells > CSV_MAX_CELLS {
            return Err(format!("CSV output exceeds {CSV_MAX_CELLS} cells"));
        }
    }
    let out = encode(rows);
    if out.len() as u64 > CSV_MAX_BYTES {
        return Err(format!("CSV output exceeds {CSV_MAX_BYTES} bytes"));
    }
    let err = |e| format!("cannot write CSV {path:?}: {e}");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(err)?;
    file.write_all(out.as_bytes()).map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn typed_order_and_delimiters() {
        let input = "value,valid,station\n1.5,true,\"North, Annex\"\n0,true,South\n9,false,South\n2.5,true,\"North, Annex\"\n";
        let schema = row(&["station", "value", "valid"]);
        assert_eq!(
            typed_rows(input, &schema).unwrap(),
            vec![
                row(&["North, Annex", "1.5", "true"]),
                row(&["South", "0", "true"]),
                row(&["South", "9", "false"]),
                row(&["North, Annex", "2.5", "true"]),
            ]
        );
    }

    #[test]
    fn reject_bad_headers_and_widths() {
        let schema = row(&["x", "y"]);
        assert!(typed_rows("x,x\n", &schema)
            .unwrap_err()
            .contains("column 2: duplicate"));
        assert!(typed_rows("x,z\n", &schema)
            .unwrap_err()
            .contains("missing header \"y\""));
        assert!(typed_rows("x,y,z\n", &schema)
            .unwrap_err()
            .contains("column 3: unexpected"));
        assert!(typed_rows("x,y\n0\n", &schema)
            .unwrap_err()
            .contains("record 2"));
        assert!(typed_rows("", &schema).unwrap_err().contains("row 1"));
    }

    #[test]
    fn raw_grid_and_output_roundtrip() {
        let grid = vec![row(&["\u{feff}header", "a,b"]), row(&["\"a\"\r\nb", ""])];
        assert_eq!(decode(&encode(&grid)).unwrap(), grid);
        assert_eq!(
            decode("\u{feff}a,\r\nb,c\r\n").unwrap(),
            vec![row(&["a", ""]), row(&["b", "c"])]
        );
        assert!(decode("\"x\"z\n").unwrap_err().contains("closing quote"));
        assert!(decode("\"x").unwrap_err().contains("unclosed"));
    }

    #[test]
    fn no_clobber() {
        let dir = std::env::temp_dir().join(format!(
            "flux-csv-test-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("file.csv");
        let name = path.to_str().unwrap();
        let grid = vec![row(&["a,b", "c"])];
        write(name, &grid).unwrap();
        assert!(write(name, &[row(&["replacement"])]).is_err());
        assert_eq!(decode(&read(name).unwrap()).unwrap(), grid);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
