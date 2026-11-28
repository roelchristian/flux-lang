//! Native namespace projections of the authoritative builtin catalogue.
use super::{lookup, REGISTRY};

// Native exports are explicit; some qualified aliases are only for dispatch.
// Form the registry key from the namespace and the member, and read its arity there.
type Member = (&'static str, &'static str);
macro_rules! exports {
    ($module:literal: $($member:literal),* $(,)?) => {
        &[$(($member, concat!($module, ".", $member))),*]
    };
}

const IO: &[Member] = exports!(
    "io": "print", "println", "readln", "read",
    "write", "exists", "read_csv", "write_csv"
);
const CSV: &[Member] = exports!("csv": "read", "write");
const TABLE: &[Member] = exports!(
    "df": "agg", "extend", "semi", "anti", "union",
    "distinct", "select", "structural", "dropna", "fill", "present"
);
const STATISTICS: &[Member] = exports!("stat": "avg", "mean", "pct", "sd", "cor");

const OPT: &[Member] = exports!("opt": "map", "bind", "result");
const RES: &[Member] = exports!("res": "map", "maperr", "collect", "partition", "ok", "optional");
const FS: &[Member] = exports!("fs": "ls", "mkdir", "base", "join", "exists");
const SYS: &[Member] = exports!("sys": "args", "env", "pid");
const STR: &[Member] = exports!("str": "upper", "lower", "trim", "split", "join");
const RX: &[Member] = exports!("rx": "find");
const DT: &[Member] = exports!("dt": "days", "date");
const JSON: &[Member] = exports!("json": "encode", "decode");
const ARR: &[Member] = exports!(
    "arr": "reshape", "shape", "ravel", "at", "map", "zip",
    "outer", "rankmap", "scan", "windows", "select", "grade", "permute",
    "axes", "sect", "red", "fold", "scana", "dot", "mul", "diag"
);

pub fn module_items(path: &str) -> Option<impl Iterator<Item = (&'static str, usize)>> {
    let math_prefix = match path {
        "math" => Some("math."),
        "poly" => Some("poly."),
        "num" => Some("num."),
        _ => None,
    };
    let members: &[Member] = match path {
        "io" => IO,
        "csv" => CSV,
        "math" | "poly" | "num" => &[],
        "df" | "table" => TABLE,
        "stat" | "stats" | "statistics" => STATISTICS,
        "opt" => OPT,
        "res" => RES,
        "fs" => FS,
        "sys" => SYS,
        "str" => STR,
        "rx" => RX,
        "dt" => DT,
        "json" => JSON,
        "arr" => ARR,

        _ => return None,
    };
    Some(
        members
            .iter()
            .map(|&(local, key)| {
                (
                    local,
                    lookup(key).expect("module member has registry entry").arity,
                )
            })
            .chain(REGISTRY.iter().filter_map(move |spec| {
                let local = spec.canonical.strip_prefix(math_prefix?)?;
                Some((local, spec.arity))
            })),
    )
}
