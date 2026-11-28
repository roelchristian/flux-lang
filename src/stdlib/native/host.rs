//! Suspended input, filesystem and process operations.
use super::super::{
    effects, exact_i128, invariant, resources, Action, CoreOp, Domain, HirProgram, Span, R, V,
};
use super::{optional, sequence, string};
use crate::host_fault::{IoFault, IoKind};
use std::{
    io::{BufRead, Read},
    path::Path,
    rc::Rc,
};

const LINE_LIMIT: usize = 1024 * 1024;
const DIRECTORY_NAMES_LIMIT: usize = 8 * 1024 * 1024;

fn read_line(input: &mut dyn BufRead) -> Result<V, IoFault> {
    const SOURCE: &str = "stdin";
    let mut raw = Vec::new();
    // One extra byte for CRLF and one for the delimiter/detection. On
    // oversize input each call consumes at most this bounded prefix.
    (&mut *input)
        .take((LINE_LIMIT + 2) as u64)
        .read_until(b'\n', &mut raw)
        .map_err(|e| IoFault::system(SOURCE, e, false))?;
    if raw.is_empty() {
        return Ok(optional(None));
    }
    if raw.last() == Some(&b'\n') {
        raw.pop();
        if raw.last() == Some(&b'\r') {
            raw.pop();
        }
    }
    if raw.len() > LINE_LIMIT {
        return Err(IoFault::details(
            IoKind::Read,
            SOURCE,
            "input line exceeds the 1 MiB byte limit".into(),
        ));
    }
    let line = String::from_utf8(raw).map_err(|_| IoFault::encoding(SOURCE))?;
    Ok(optional(Some(string(line))))
}

fn directory(path: &str) -> Result<V, IoFault> {
    let entries = std::fs::read_dir(path).map_err(|e| IoFault::system(path, e, false))?;
    let mut names = Vec::new();
    let mut bytes = 0usize;
    for item in entries {
        let item = item.map_err(|e| IoFault::system(path, e, false))?;
        let name = item
            .file_name()
            .into_string()
            .map_err(|_| IoFault::encoding(path))?;
        bytes = bytes.saturating_add(name.len());
        if names.len() >= resources::limits().max_elements || bytes > DIRECTORY_NAMES_LIMIT {
            return Err(IoFault::details(
                IoKind::Read,
                path,
                "directory listing exceeds the resource limit".into(),
            ));
        }
        names.push(name);
    }
    names.sort_unstable();
    Ok(sequence(names.into_iter().map(string).collect()))
}

fn getenv(name: &str) -> Result<V, IoFault> {
    if name.is_empty() || name.contains(['=', '\0']) {
        return Err(IoFault::details(
            IoKind::Read,
            name,
            format!("invalid environment variable name {name:?}"),
        ));
    }
    match std::env::var(name) {
        Ok(value) => Ok(optional(Some(string(value)))),
        Err(std::env::VarError::NotPresent) => Ok(optional(None)),
        Err(std::env::VarError::NotUnicode(_)) => Err(IoFault::encoding(name)),
    }
}

pub(super) fn construct(op: CoreOp, args: &[V], span: Span) -> R<V> {
    use CoreOp::*;
    match (op, args) {
        (ReadLine | SysArgs | SysPid, []) | (FsList | FsMkdir | SysEnv, [V::String(_)]) => {
            Ok(V::Action(Rc::new(Action::Library(op, args.to_vec()))))
        }
        (FsBase, [V::String(path)]) => Ok(optional(
            Path::new(path.as_ref())
                .file_name()
                .and_then(|name| name.to_str())
                .map(string),
        )),
        (FsJoin, [V::String(path), V::String(part)]) => {
            let path = Path::new(path.as_ref()).join(part.as_ref());
            path.to_str()
                .map(string)
                .ok_or_else(|| invariant(span, "joined UTF-8 paths produced a non-UTF-8 path"))
        }
        _ => Err(invariant(span, "invalid standard-library operands")),
    }
}

pub(super) fn execute(
    op: CoreOp,
    args: &[V],
    program: &HirProgram,
    stdin: &mut dyn BufRead,
    arguments: &[String],
    span: Span,
) -> R<V> {
    use CoreOp::*;
    match (op, args) {
        (ReadLine, []) => effects::on_io(program, read_line(stdin), span),
        (FsList, [V::String(path)]) => effects::on_io(program, directory(path), span),
        (FsMkdir, [V::String(path)]) => effects::on_io(
            program,
            std::fs::create_dir(path.as_ref())
                .map(|()| V::Unit)
                .map_err(|e| IoFault::system(path, e, true)),
            span,
        ),
        (SysArgs, []) => Ok(sequence(arguments.iter().map(string).collect())),
        (SysPid, []) => Ok(exact_i128(std::process::id().into(), Domain::Natural)),
        (SysEnv, [V::String(name)]) => effects::on_io(program, getenv(name), span),
        _ => Err(invariant(
            span,
            "invalid suspended standard-library operation",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn line_eof_crlf_and_encoding_have_distinct_outcomes() {
        let mut input = Cursor::new(&b"one\r\n\n\xc3\xa9\nlast\r"[..]);
        for expected in ["one", "", "é", "last\r"] {
            let V::Optional(Some(value)) = read_line(&mut input).unwrap() else {
                panic!("expected a line");
            };
            assert!(matches!(&*value, V::String(s) if s.as_ref() == expected));
        }
        assert!(matches!(read_line(&mut input), Ok(V::Optional(None))));
        let err = read_line(&mut Cursor::new([0xff, b'\n']))
            .err()
            .expect("expected invalid UTF-8");
        assert_eq!(err.kind, IoKind::Encoding);
        assert_eq!(err.payload, "stdin");
    }

    #[test]
    fn line_resource_limit_is_byte_based_and_bounded() {
        let mut data = vec![b'x'; LINE_LIMIT];
        data.extend_from_slice(b"\r\n");
        let mut input = Cursor::new(&data);
        assert!(matches!(read_line(&mut input), Ok(V::Optional(Some(_)))));
        data.resize(LINE_LIMIT + 40, b'x');
        data[LINE_LIMIT..].fill(b'x');
        let mut input = Cursor::new(&data);
        assert_eq!(
            read_line(&mut input)
                .err()
                .expect("expected line limit")
                .kind,
            IoKind::Read
        );
        assert_eq!(input.position() as usize, LINE_LIMIT + 2);
    }
}
