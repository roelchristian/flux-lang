use crate::host_fault::{IoFault, IoKind};
use std::io::{Read, Write};

#[cfg(any(test, feature = "legacy-oracle"))]
type Rt<T> = Result<T, String>;
type FileResult<T> = Result<T, IoFault>;
type CsvResult<T> = Result<T, CsvFault>;

const CSV_MAX_BYTES: u64 = 1_048_576;
const CSV_MAX_CELLS: usize = 100_000;
pub(crate) const TEXT_MAX_BYTES: u64 = 1_048_576;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CsvFaultKind {
    Header(String),
    Duplicate(String),
    Absent(String),
    Row(usize),
}

#[derive(Debug)]
pub(crate) struct CsvFault {
    pub kind: CsvFaultKind,
    pub diagnostic: String,
}

impl CsvFault {
    fn row(index: usize, diagnostic: String) -> Self {
        Self {
            kind: CsvFaultKind::Row(index),
            diagnostic,
        }
    }

    fn header(diagnostic: String) -> Self {
        Self {
            kind: CsvFaultKind::Header(diagnostic.clone()),
            diagnostic,
        }
    }
}

/// A byte source used for executing read actions. It is not consulted when an
/// action is constructed or statically checked. The runtime still enforces
/// its independent length limit and UTF-8 decoding.
pub trait RuntimeInput {
    fn open(&mut self, path: &str) -> std::io::Result<OpenedInput>;
}

pub struct OpenedInput {
    pub reader: Box<dyn Read>,
    pub byte_len: Option<u64>,
}

/// Default input uses the platform filesystem and only opens regular files.
/// File writes and existence queries are not redirected by the input seam.
pub struct FsInput;

impl RuntimeInput for FsInput {
    fn open(&mut self, path: &str) -> std::io::Result<OpenedInput> {
        use std::io::{Error, ErrorKind};
        let irregular = || Error::new(ErrorKind::InvalidInput, "path is not a regular file");
        // Test before open so named pipes and other non-regular input are not
        // opened intentionally; recheck the opened descriptor as well.
        let before = std::fs::metadata(path)?;
        if !before.is_file() {
            return Err(irregular());
        }
        let file = std::fs::File::open(path)?;
        let opened = file.metadata()?;
        if !opened.is_file() {
            return Err(irregular());
        }
        Ok(OpenedInput {
            reader: Box::new(file),
            byte_len: Some(before.len().max(opened.len())),
        })
    }
}

pub(crate) fn read_text_with_input(
    input: &mut dyn RuntimeInput,
    path: &str,
    label: &str,
    max: u64,
) -> FileResult<String> {
    let io = |error| IoFault::system(path, error, false);
    let too_large = || {
        IoFault::details(
            IoKind::Read,
            path,
            format!("{label} input exceeds {max} bytes"),
        )
    };
    let opened = input.open(path).map_err(io)?;
    if opened.byte_len.is_some_and(|len| len > max) {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    opened
        .reader
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() as u64 > max {
        return Err(too_large());
    }
    String::from_utf8(bytes).map_err(|_| IoFault::encoding(path))
}

pub(crate) fn read041_with_input(input: &mut dyn RuntimeInput, path: &str) -> FileResult<String> {
    read_text_with_input(input, path, "CSV", CSV_MAX_BYTES)
}
#[cfg(any(test, feature = "legacy-oracle"))]
pub fn read(path: &str) -> Rt<String> {
    read041_with_input(&mut FsInput, path).map_err(|e| e.diagnostic)
}

#[cfg(any(test, feature = "legacy-oracle"))]
pub fn decode(s: &str) -> Rt<Vec<Vec<String>>> {
    decode041(s).map_err(|e| e.diagnostic)
}
pub(crate) fn decode041(s: &str) -> CsvResult<Vec<Vec<String>>> {
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
    let mut add_field =
        |row: &mut Vec<String>, field: &mut String, index: usize| -> CsvResult<()> {
            count += 1;
            if count > CSV_MAX_CELLS {
                return Err(CsvFault::row(
                    index,
                    format!("CSV input exceeds {CSV_MAX_CELLS} cells"),
                ));
            }
            row.push(std::mem::take(field));
            Ok(())
        };
    fn add_row(rows: &mut Vec<Vec<String>>, row: &mut Vec<String>) -> CsvResult<()> {
        if let Some(first) = rows.first() {
            if row.len() != first.len() {
                return Err(CsvFault::row(
                    rows.len() + 1,
                    format!(
                        "record {} has {} fields; expected {} (first mismatched column {})",
                        rows.len() + 1,
                        row.len(),
                        first.len(),
                        row.len().min(first.len()) + 1
                    ),
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
                add_field(&mut row, &mut field, rows.len() + 1)?;
                state = State::Start;
            }
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                add_field(&mut row, &mut field, rows.len() + 1)?;
                add_row(&mut rows, &mut row)?;
                state = State::Start;
                open = false;
            }
            '"' if state == State::Start => state = State::Quoted,
            '"' => {
                return Err(CsvFault::row(
                    rows.len() + 1,
                    format!(
                        "unexpected quote in record {}, field {}",
                        rows.len() + 1,
                        row.len() + 1
                    ),
                ))
            }
            _ if state == State::Closed => {
                return Err(CsvFault::row(
                    rows.len() + 1,
                    format!(
                        "unexpected content after closing quote in record {}, field {}",
                        rows.len() + 1,
                        row.len() + 1
                    ),
                ))
            }
            _ => {
                field.push(c);
                state = State::Text;
            }
        }
    }
    if state == State::Quoted {
        return Err(CsvFault::row(
            rows.len() + 1,
            format!(
                "unclosed quoted field in record {}, field {}",
                rows.len() + 1,
                row.len() + 1
            ),
        ));
    }
    if open {
        add_field(&mut row, &mut field, rows.len() + 1)?;
        add_row(&mut rows, &mut row)?;
    }
    Ok(rows)
}

/// Parse records and select columns in schema order, without returning the header.
#[cfg(any(test, feature = "legacy-oracle"))]
pub fn typed_rows(s: &str, ordered_header: &[String]) -> Rt<Vec<Vec<String>>> {
    typed_rows041(s, ordered_header).map_err(|e| e.diagnostic)
}
pub(crate) fn typed_rows041(s: &str, ordered_header: &[String]) -> CsvResult<Vec<Vec<String>>> {
    use std::collections::{HashMap, HashSet};
    let mut rows = decode041(s)
        .map_err(|e| match e.kind {
            CsvFaultKind::Row(1) => CsvFault::header(e.diagnostic),
            _ => e,
        })?
        .into_iter();
    let header = rows
        .next()
        .ok_or_else(|| CsvFault::header("CSV row 1: expected header, found empty input".into()))?;
    let mut columns = HashMap::new();
    for (i, name) in header.iter().enumerate() {
        if let Some(first) = columns.insert(name.as_str(), i) {
            return Err(CsvFault {
                kind: CsvFaultKind::Duplicate(name.clone()),
                diagnostic: format!(
                    "CSV row 1, column {}: duplicate header {name:?} (first seen in column {})",
                    i + 1,
                    first + 1
                ),
            });
        }
    }
    let mut fields = HashSet::new();
    let mut order = Vec::with_capacity(ordered_header.len());
    for (i, name) in ordered_header.iter().enumerate() {
        if !fields.insert(name.as_str()) {
            return Err(CsvFault::header(format!(
                "CSV schema column {}: duplicate name {name:?}",
                i + 1
            )));
        }
        order.push(*columns.get(name.as_str()).ok_or_else(|| CsvFault {
            kind: CsvFaultKind::Absent(name.clone()),
            diagnostic: format!(
                "CSV row 1, schema column {}: missing header {name:?}",
                i + 1
            ),
        })?);
    }
    for (i, name) in header.iter().enumerate() {
        if !fields.contains(name.as_str()) {
            return Err(CsvFault::header(format!(
                "CSV row 1, column {}: unexpected header {name:?}",
                i + 1
            )));
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
#[cfg(any(test, feature = "legacy-oracle"))]
pub fn write(path: &str, rows: &[Vec<String>]) -> Rt<()> {
    write041(path, rows).map_err(|e| e.diagnostic)
}

pub(crate) fn write041(path: &str, rows: &[Vec<String>]) -> FileResult<()> {
    let width = rows.first().map_or(0, Vec::len);
    let mut cells = 0;
    for (i, row) in rows.iter().enumerate() {
        if row.is_empty() || row.len() != width {
            return Err(IoFault::details(
                IoKind::Write,
                path,
                format!(
                    "CSV row {}: expected {} fields, found {}",
                    i + 1,
                    width.max(1),
                    row.len()
                ),
            ));
        }
        cells += row.len();
        if cells > CSV_MAX_CELLS {
            return Err(IoFault::details(
                IoKind::Write,
                path,
                format!("CSV output exceeds {CSV_MAX_CELLS} cells"),
            ));
        }
    }
    let out = encode(rows);
    write_text(path, &out, "CSV", CSV_MAX_BYTES)
}

pub(crate) fn write_text(path: &str, text: &str, label: &str, max: u64) -> FileResult<()> {
    if text.len() as u64 > max {
        return Err(IoFault::details(
            IoKind::Write,
            path,
            format!("{label} output exceeds {max} bytes"),
        ));
    }
    let err = |error| IoFault::system(path, error, true);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(err)?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.flush())
        .map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn structured_csv_failures_retain_header_and_record_identity() {
        let columns = row(&["station", "value"]);
        assert_eq!(
            typed_rows041("station,station\n", &columns)
                .unwrap_err()
                .kind,
            CsvFaultKind::Duplicate("station".to_owned()),
        );
        assert_eq!(
            typed_rows041("station\n", &columns).unwrap_err().kind,
            CsvFaultKind::Absent("value".to_owned()),
        );
        assert_eq!(
            typed_rows041(concat!("station,value\n", r#""bad,1"#), &columns)
                .unwrap_err()
                .kind,
            CsvFaultKind::Row(2),
        );
        assert!(matches!(
            typed_rows041("", &columns).unwrap_err().kind,
            CsvFaultKind::Header(_),
        ));
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
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("flux-csv-test-{}-{nonce}", std::process::id(),));
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
