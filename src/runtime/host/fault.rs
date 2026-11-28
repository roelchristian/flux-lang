//! Host I/O failure classification before conversion to a checked error value.

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
    use super::IoFault;

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
