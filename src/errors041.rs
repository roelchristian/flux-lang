//! Standard Flux error constructors shared by parser, checker and runtime.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StandardVariant {
    pub owner: &'static str,
    pub name: &'static str,
    pub payload: &'static [&'static str],
}

pub const OWNERS: &[&str] = &[
    "ioerr",
    "csverr",
    "numerr",
    "rxerr",
    "dterr",
    "jsonerr",
    "solvererr",
];

macro_rules! variants_for {
    ($owner:literal; $($name:literal => [$($payload:literal),*]),+ $(,)?) => {
        &[ $(StandardVariant { owner: $owner, name: $name, payload: &[$($payload),*] }),+ ]
    }
}

const IO: &[StandardVariant] = variants_for!("ioerr";
    "missing" => ["s"], "denied" => ["s"], "exists" => ["s"],
    "readerr" => ["s"], "writeerr" => ["s"], "encoding" => ["s"],
);
const CSV: &[StandardVariant] = variants_for!("csverr";
    "fileerr" => ["ioerr"], "header" => ["s"], "duplicate" => ["s"],
    "absent" => ["s"], "cellerr" => ["n", "s", "s"], "rowerr" => ["n"],
);
const NUM: &[StandardVariant] = variants_for!("numerr"; "divzero" => [], "range" => []);

const RX: &[StandardVariant] = variants_for!("rxerr"; "badpattern" => ["s"]);
const DT: &[StandardVariant] = variants_for!("dterr"; "baddate" => ["s"]);
const JSON: &[StandardVariant] =
    variants_for!("jsonerr"; "invalidjson" => ["s"], "jsonshape" => ["s"]);

const SOLVER: &[StandardVariant] = variants_for!("solvererr";
    "badparams" => [], "badbracket" => [], "singular" => [],
    "nonfinite" => [], "noconverge" => [], "budget" => [],
);

pub fn variants(owner: &str) -> &'static [StandardVariant] {
    match owner {
        "ioerr" => IO,
        "csverr" => CSV,
        "numerr" => NUM,
        "rxerr" => RX,
        "dterr" => DT,
        "jsonerr" => JSON,
        "solvererr" => SOLVER,
        _ => &[],
    }
}

pub fn lookup(owner: &str, name: &str) -> Option<&'static StandardVariant> {
    variants(owner).iter().find(|v| v.name == name)
}

/// Nominal identities reserved outside the ranges used for lexical declarations.
pub fn owner_id(owner: &str) -> Option<usize> {
    OWNERS
        .iter()
        .position(|x| *x == owner)
        .map(|i| usize::MAX - i)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IoKind {
    Missing,
    Denied,
    Exists,
    Read,
    Write,
    Encoding,
}

impl IoKind {
    pub(crate) fn constructor(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Denied => "denied",
            Self::Exists => "exists",
            Self::Read => "readerr",
            Self::Write => "writeerr",
            Self::Encoding => "encoding",
        }
    }
}

#[derive(Debug)]
pub(crate) struct IoFault {
    pub kind: IoKind,
    pub payload: String,
    pub diagnostic: String,
}

impl IoFault {
    pub(crate) fn system(path: &str, e: std::io::Error, writing: bool) -> Self {
        use std::io::ErrorKind;
        let kind = match e.kind() {
            ErrorKind::NotFound => IoKind::Missing,
            ErrorKind::PermissionDenied => IoKind::Denied,
            ErrorKind::AlreadyExists => IoKind::Exists,
            _ if writing => IoKind::Write,
            _ => IoKind::Read,
        };
        let verb = if writing { "write" } else { "read" };
        let diagnostic = format!("cannot {verb} {path:?}: {e}");
        let payload = path.to_owned();
        Self {
            kind,
            payload,
            diagnostic,
        }
    }

    pub(crate) fn details(kind: IoKind, path: &str, diagnostic: String) -> Self {
        Self {
            kind,
            diagnostic,
            payload: path.to_owned(),
        }
    }

    pub(crate) fn encoding(path: &str) -> Self {
        Self {
            kind: IoKind::Encoding,
            payload: path.to_owned(),
            diagnostic: format!("{path:?} is not valid UTF-8"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn standard_identities_and_payload_arities_are_unique() {
        let mut ids = HashSet::new();
        for &owner in OWNERS {
            assert!(ids.insert(owner_id(owner).unwrap()));
            let mut names = HashSet::new();
            for v in variants(owner) {
                assert_eq!(v.owner, owner);
                assert!(names.insert(v.name));
                assert_eq!(lookup(owner, v.name), Some(v));
            }
        }
        assert_eq!(
            lookup("csverr", "cellerr").unwrap().payload,
            ["n", "s", "s"]
        );
        assert!(lookup("numerr", "divzero").unwrap().payload.is_empty());
    }

    #[test]
    fn stable_io_names_and_do_not_parse_os_messages() {
        use std::io::{Error, ErrorKind};
        let e = IoFault::system(
            "sample.csv",
            Error::from(ErrorKind::PermissionDenied),
            false,
        );
        assert_eq!(e.kind.constructor(), "denied");
        assert_eq!(e.payload, "sample.csv");
        let e = IoFault::system("output.csv", Error::from(ErrorKind::AlreadyExists), true);
        assert_eq!(e.kind.constructor(), "exists");
        assert_eq!(e.payload, "output.csv");
    }
}
