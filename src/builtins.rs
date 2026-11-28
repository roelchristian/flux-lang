//! Shared identities, visibility, arity and behavior classification for Flux primitives.
//! Availability describes implemented runtime signatures, not just recognized spellings.
use crate::hir::CoreOp;
use std::sync::LazyLock;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Availability {
    Runtime,
    Recognized,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Effect {
    Pure,
    ConstructAction,
    ExecuteAction,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Rule {
    Arithmetic,
    Division,
    IntegralArithmetic,
    Power,
    Negation,
    SquareRoot,
    Order,
    Equality,
    StructuralEquality,
    ApproxEquality,
    Boolean,
    Cardinality,
    Range,
    Apply,
    Map,
    Filter,
    Group,
    Sort,
    Concat,
    Household,
    Lookup,
    Fold,
    Reduction,
    Statistics,
    Optional,
    Result,
    Table,
    Effect,
    Conversion,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Shape {
    Scalar,
    Elementwise,
    Reduction,
    Collection,
    Dependent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Equality {
    NotRequired,
    Elementwise,
    Structural,
    Tolerance,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Numeric {
    N,
    Z,
    Q,
    D,
    R,
}

/// Ordinary implicit numeric joins. Approximation never silently consumes an exact operand.
pub const fn promote(a: Numeric, b: Numeric) -> Option<Numeric> {
    use Numeric::*;
    match (a, b) {
        (R, R) => Some(R),
        (R, _) | (_, R) => None,
        (Q, _) | (_, Q) => Some(Q),
        (D, _) | (_, D) => Some(D),
        (Z, _) | (_, Z) => Some(Z),
        (N, N) => Some(N),
    }
}

/// Domain rule after collection shape and operand constraints have been established.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BinaryDomain {
    Common,
    Integral,
    Division,
    Power,
}

impl BinaryDomain {
    pub(crate) const fn from_rule(rule: Rule) -> Option<Self> {
        match rule {
            Rule::Arithmetic | Rule::Order | Rule::Equality | Rule::StructuralEquality => {
                Some(Self::Common)
            }
            Rule::IntegralArithmetic | Rule::Range => Some(Self::Integral),
            Rule::Division => Some(Self::Division),
            Rule::Power => Some(Self::Power),
            _ => None,
        }
    }

    pub(crate) fn result(self, operator: &str, common: Numeric) -> Result<Numeric, String> {
        use Numeric::*;
        match (self, common) {
            (Self::Integral, N | Z) => Ok(common),
            (Self::Integral, _) => Err(format!("{operator} requires integral operands")),
            (Self::Division, R) => Ok(R),
            (Self::Division, _) => Ok(Q),
            (Self::Common | Self::Power, _) => Ok(common),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Builtin {
    pub canonical: &'static str,
    pub op: Option<CoreOp>,
    pub aliases: &'static [&'static str],
    pub arity: usize,
    pub effect: Effect,
    pub rule: Rule,
    pub shape: Shape,
    pub equality: Equality,
    pub availability: Availability,
    pub prelude: bool,
}

use Availability::{Recognized, Runtime};
use Effect::{ConstructAction, ExecuteAction, Pure};
use Equality::{NotRequired, Tolerance};
use Shape::{Collection, Dependent, Elementwise, Reduction, Scalar};

// Registry defaults keep the entries focused on what distinguishes each operation.
// Overrides are named so equalities, aliases, and unavailable forms stay visible.
macro_rules! builtin {
    ($prelude:literal, $op:expr, $name:literal, $arity:literal, $rule:ident, $shape:expr
        $(, $field:ident: $value:expr)* $(,)?) => {
        Builtin {
            $($field: $value,)*
            ..Builtin {
                canonical: $name,
                op: $op,
                aliases: &[],
                arity: $arity,
                effect: Pure,
                rule: Rule::$rule,
                shape: $shape,
                equality: NotRequired,
                availability: Runtime,
                prelude: $prelude,
            }
        }
    };
}
macro_rules! prelude {
    ($op:ident, $($spec:tt)*) => {
        builtin!(true, Some(CoreOp::$op), $($spec)*)
    };
}
macro_rules! internal {
    ($op:ident, $($spec:tt)*) => {
        builtin!(false, Some(CoreOp::$op), $($spec)*)
    };
}
macro_rules! action {
    ($op:ident, $name:literal, $arity:literal, $shape:expr $(, $field:ident: $value:expr)*) => {
        internal!($op, $name, $arity, Effect, $shape, effect: ConstructAction $(, $field: $value)*)
    };
}

/// Registry of exact names; selectors such as .field are generated forms, not entries.
pub static REGISTRY: &[Builtin] = &[
    internal!(ArrayReshape, "arr.reshape", 2, Household, Collection),
    internal!(ArrayShape, "arr.shape", 1, Household, Collection, aliases: &["dim"]),
    internal!(ArrayRavel, "arr.ravel", 1, Household, Collection),
    internal!(ArrayAt, "arr.at", 2, Household, Collection),
    internal!(ArrayMap, "arr.map", 2, Household, Collection),
    internal!(ArrayZip, "arr.zip", 3, Household, Collection),
    internal!(ArrayOuter, "arr.outer", 3, Household, Collection),
    internal!(ArrayRankMap, "arr.rankmap", 3, Household, Collection),
    internal!(ArrayScan, "arr.scan", 3, Household, Collection),
    internal!(ArrayWindows, "arr.windows", 2, Household, Collection),
    internal!(ArraySelect, "arr.select", 2, Household, Collection),
    internal!(ArrayGrade, "arr.grade", 2, Household, Collection),
    internal!(ArrayPermute, "arr.permute", 2, Household, Collection),
    prelude!(ArrayForm, "form", 2, Household, Collection),
    internal!(ArrayAxes, "arr.axes", 2, Household, Collection),
    internal!(ArraySection, "arr.sect", 3, Household, Collection),
    internal!(ArrayReduce, "arr.red", 3, Fold, Reduction),
    internal!(ArrayFold, "arr.fold", 4, Fold, Reduction),
    internal!(ArrayScanAxis, "arr.scana", 4, Fold, Collection),
    internal!(ArrayDot, "arr.dot", 2, Arithmetic, Reduction),
    internal!(ArrayMatMul, "arr.mul", 2, Arithmetic, Collection),
    internal!(ArrayDiag, "arr.diag", 1, Household, Collection),
    prelude!(Add, "+", 2, Arithmetic, Elementwise),
    prelude!(Sub, "-", 2, Arithmetic, Elementwise),
    prelude!(Mul, "*", 2, Arithmetic, Elementwise),
    prelude!(Div, "/", 2, Division, Elementwise),
    internal!(CheckedDiv, "math.div", 2, Division, Scalar),
    internal!(Quantize, "math.quantize", 2, Conversion, Scalar),
    prelude!(Quotient, "//", 2, IntegralArithmetic, Elementwise),
    prelude!(Remainder, "%", 2, IntegralArithmetic, Elementwise),
    prelude!(Pow, "^", 2, Power, Elementwise),
    prelude!(Negate, "-:", 1, Negation, Elementwise),
    prelude!(Sqrt, "^/", 1, SquareRoot, Elementwise),
    prelude!(Abs, "|", 1, Arithmetic, Elementwise),
    prelude!(Lt, "<", 2, Order, Elementwise),
    prelude!(Le, "<=", 2, Order, Elementwise),
    prelude!(Gt, ">", 2, Order, Elementwise),
    prelude!(Ge, ">=", 2, Order, Elementwise),
    prelude!(Equal, "=", 2, Equality, Elementwise, equality: Equality::Elementwise),
    prelude!(NotEqual, "/=", 2, Equality, Elementwise, equality: Equality::Elementwise),
    prelude!(EqualWhole, "==", 2, StructuralEquality, Scalar, equality: Equality::Structural),
    builtin!(
        true, None, "~=", 3, ApproxEquality, Dependent,
        equality: Tolerance, availability: Recognized
    ),
    prelude!(Not, "~", 1, Boolean, Elementwise),
    prelude!(And, "&&", 2, Boolean, Elementwise),
    prelude!(Or, "||", 2, Boolean, Elementwise),
    prelude!(Count, "#", 1, Cardinality, Reduction),
    prelude!(Range, "..", 2, Range, Collection),
    prelude!(Apply, "@", 2, Apply, Dependent),
    prelude!(ApplyTuple, "@@", 2, Apply, Dependent),
    prelude!(Map, "@:", 2, Map, Collection),
    prelude!(Filter, "?:", 2, Filter, Collection),
    prelude!(Group, "=:", 2, Group, Collection, equality: Equality::Structural),
    prelude!(SortAsc, ">:", 2, Sort, Collection),
    prelude!(SortDesc, "<:", 2, Sort, Collection),
    prelude!(Concat, "+:", 2, Concat, Collection),
    prelude!(Zip, "&:", 2, Household, Collection),
    prelude!(Cartesian, "*:", 2, Household, Collection),
    prelude!(Distinct, "=.", 1, Household, Collection, equality: Equality::Structural),
    prelude!(Lookup, ".:", 2, Lookup, Dependent, equality: Equality::Structural),
    prelude!(Take, "[:", 2, Household, Collection, aliases: &["take"]),
    prelude!(Fold, "/:", 3, Fold, Reduction),
    prelude!(Reduce, "@/", 2, Fold, Reduction),
    prelude!(Scan, "\\:", 3, Fold, Collection),
    prelude!(Sum, "+/", 1, Reduction, Reduction),
    prelude!(Product, "*/", 1, Reduction, Reduction),
    prelude!(All, "&&/", 1, Reduction, Reduction),
    prelude!(Any, "||/", 1, Reduction, Reduction),
    prelude!(Max, ">/", 1, Reduction, Reduction),
    prelude!(Min, "</", 1, Reduction, Reduction),
    prelude!(
        Mean, "avg", 1, Statistics, Reduction,
        aliases: &["statistics.avg", "stats.avg", "stat.avg",
                   "statistics.mean", "stats.mean", "stat.mean"]
    ),
    prelude!(
        Percentile, "pct", 2, Statistics, Reduction,
        aliases: &["statistics.pct", "stats.pct", "stat.pct"]
    ),
    prelude!(
        StdDev, "sd", 1, Statistics, Reduction,
        aliases: &["statistics.sd", "stats.sd", "stat.sd"]
    ),
    builtin!(
        true, None, "cor", 2, Statistics, Reduction,
        aliases: &["statistics.cor", "stats.cor", "stat.cor"],
        availability: Recognized
    ),
    prelude!(Drop, ":]", 2, Household, Collection, aliases: &["drop"]),
    prelude!(Slice, "sect", 3, Household, Collection, aliases: &["slice"]),
    prelude!(Reverse, "rev", 1, Household, Collection),
    prelude!(Flatten, ",.", 1, Household, Collection, aliases: &["flat"]),
    prelude!(Transpose, "|:", 1, Household, Collection, aliases: &["tr"]),
    prelude!(Median, "med", 1, Statistics, Reduction, aliases: &["statistics.med", "stats.med"]),
    prelude!(Variance, "var", 1, Statistics, Reduction, aliases: &["statistics.var", "stats.var"]),
    prelude!(Absent, "?0", 0, Optional, Scalar),
    prelude!(Present, "?1", 1, Optional, Dependent),
    prelude!(Has, "?+", 1, Optional, Scalar),
    prelude!(Default, "??", 2, Optional, Dependent),
    prelude!(Unwrap, "?!", 1, Optional, Dependent),
    prelude!(TableFrom, "#:", 1, Table, Collection),
    prelude!(Rows, ":#", 1, Table, Collection),
    prelude!(InnerJoin, "*=", 4, Table, Collection, equality: Equality::Structural),
    prelude!(LeftJoin, "*?", 4, Table, Collection, equality: Equality::Structural),
    internal!(Execute, "!", 1, Effect, Dependent, effect: ExecuteAction),
    internal!(TableRows, "#[", 1, Table, Collection),
    internal!(ToNatural, ":n", 1, Conversion, Elementwise),
    internal!(ToInteger, ":z", 1, Conversion, Elementwise),
    internal!(ToRational, ":q", 1, Conversion, Elementwise),
    internal!(ToDecimal, ":d", 1, Conversion, Elementwise),
    internal!(ToReal, ":r", 1, Conversion, Elementwise),
    action!(IoPrint, "io.print", 1, Dependent, aliases: &["io.println", "print"]),
    action!(IoRead, "io.read", 1, Dependent),
    action!(IoWrite, "io.write", 2, Dependent),
    action!(IoExists, "io.exists", 1, Dependent, aliases: &["fs.exists"]),
    action!(CsvRead, "csv.read", 1, Dependent, aliases: &["io.read_csv"]),
    action!(CsvWrite, "csv.write", 2, Dependent, aliases: &["io.write_csv"]),
    internal!(Aggregate, "df.agg", 3, Table, Collection, aliases: &["table.agg"]),
    internal!(Extend, "df.extend", 2, Table, Collection, aliases: &["table.extend"]),
    internal!(OptMap, "opt.map", 2, Optional, Dependent),
    internal!(OptBind, "opt.bind", 2, Optional, Dependent),
    internal!(OptResult, "opt.result", 2, Result, Dependent),
    internal!(ResMap, "res.map", 2, Result, Dependent),
    internal!(ResMapErr, "res.maperr", 2, Result, Dependent),
    internal!(ResCollect, "res.collect", 1, Result, Collection),
    internal!(ResPartition, "res.partition", 1, Result, Collection),
    internal!(ResOk, "res.ok", 1, Result, Dependent),
    internal!(ResOptional, "res.optional", 1, Result, Dependent),
    action!(ReadLine, "io.readln", 0, Scalar),
    action!(FsList, "fs.ls", 1, Scalar),
    action!(FsMkdir, "fs.mkdir", 1, Scalar),
    internal!(FsBase, "fs.base", 1, Household, Scalar),
    internal!(FsJoin, "fs.join", 2, Household, Scalar),
    action!(SysArgs, "sys.args", 0, Scalar),
    action!(SysEnv, "sys.env", 1, Scalar),
    action!(SysPid, "sys.pid", 0, Scalar),
    internal!(StrUpper, "str.upper", 1, Household, Scalar),
    internal!(StrLower, "str.lower", 1, Household, Scalar),
    internal!(StrTrim, "str.trim", 1, Household, Scalar),
    internal!(StrSplit, "str.split", 2, Household, Collection),
    internal!(StrJoin, "str.join", 2, Household, Collection),
    internal!(RxFind, "rx.find", 2, Result, Scalar),
    internal!(DtDays, "dt.days", 1, Result, Scalar),
    internal!(DtDate, "dt.date", 1, Result, Scalar),
    internal!(JsonEncode, "json.encode", 1, Result, Dependent),
    internal!(JsonDecode, "json.decode", 1, Result, Dependent),
    internal!(SemiJoin, "df.semi", 4, Table, Collection, aliases: &["table.semi"]),
    internal!(AntiJoin, "df.anti", 4, Table, Collection, aliases: &["table.anti"]),
    internal!(TableUnion, "df.union", 2, Table, Collection, aliases: &["table.union"]),
    internal!(TableDistinct, "df.distinct", 1, Table, Collection, aliases: &["table.distinct"]),
    internal!(
        StructuralRows, "df.structural", 1, Table, Collection, aliases: &["table.structural"]
    ),
    internal!(TableSelect, "df.select", 2, Table, Collection, aliases: &["table.select"]),
    internal!(DropNa, "df.dropna", 2, Table, Collection, aliases: &["table.dropna"]),
    internal!(Fill, "df.fill", 2, Table, Collection, aliases: &["table.fill"]),
    internal!(PresentOnly, "df.present", 1, Table, Collection, aliases: &["table.present"]),
];

// Indexed by the operation discriminant; aliases share the same execution contract.
static OPERATIONS: LazyLock<[Option<&'static Builtin>; 256]> = LazyLock::new(|| {
    let mut operations: [Option<&'static Builtin>; 256] = [None; 256];
    for meta in REGISTRY {
        if let Some(op) = meta.op {
            let slot = &mut operations[op as usize];
            if let Some(other) = slot {
                assert_eq!(meta.arity, other.arity, "arity differs for builtin alias");
            } else {
                *slot = Some(meta);
            }
        }
    }
    operations
});

pub fn for_operation(op: CoreOp) -> Option<&'static Builtin> {
    OPERATIONS[op as usize]
}

pub fn lookup(name: &str) -> Option<&'static Builtin> {
    REGISTRY
        .iter()
        .find(|b| b.canonical == name || b.aliases.contains(&name))
}

pub fn canonical(name: &str) -> &str {
    lookup(name).map_or(name, |b| b.canonical)
}

// Exposed unqualified aliases. Other aliases require a native import or are for dispatch.
const PRELUDE_ALIASES: &[&str] = &["print", "dim", "take", "drop", "slice", "flat", "tr"];

pub fn prelude_arity(name: &str) -> Option<usize> {
    lookup(name)
        .filter(|b| (b.prelude && b.canonical == name) || PRELUDE_ALIASES.contains(&name))
        .map(|b| b.arity)
}

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
const MATH: &[Member] = exports!("math": "div", "quantize");
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
    let members = match path {
        "io" => IO,
        "csv" => CSV,
        "math" => MATH,
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
    Some(members.iter().map(|&(local, key)| {
        (
            local,
            lookup(key).expect("module member has registry entry").arity,
        )
    }))
}

pub fn runtime_arity(name: &str) -> Option<usize> {
    if let Some(b) = lookup(name) {
        return (b.availability == Runtime).then_some(b.arity);
    }
    let bytes = name.as_bytes();
    if bytes.first() == Some(&b'.')
        && bytes.len() > 1
        && (bytes[1..].iter().all(u8::is_ascii_digit)
            || bytes[1].is_ascii_lowercase()
                && bytes[2..]
                    .iter()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_'))
    {
        return Some(1);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    #[test]
    fn canonical_names_and_aliases_are_unique() {
        let mut names = HashSet::new();
        for b in REGISTRY {
            assert!(names.insert(b.canonical), "duplicate {}", b.canonical);
            for a in b.aliases {
                assert!(names.insert(*a), "duplicate {a}");
                assert_eq!(lookup(a), Some(b));
                assert!(
                    PRELUDE_ALIASES.contains(a) || prelude_arity(a).is_none(),
                    "alias should not leak into prelude"
                );
            }
        }
    }
    #[test]
    fn module_exports_have_registry_arity_and_legacy_unsupported_spellings() {
        for module in [
            "io",
            "csv",
            "table",
            "df",
            "stats",
            "statistics",
            "math",
            "stat",
            "opt",
            "res",
            "fs",
            "sys",
            "str",
            "rx",
            "dt",
            "json",
            "arr",
        ] {
            let mut names = HashSet::new();
            for (name, arity) in module_items(module).unwrap() {
                assert!(names.insert(name));
                assert_eq!(lookup(&format!("{module}.{name}")).unwrap().arity, arity);
            }
        }
        assert_eq!(canonical("statistics.mean"), "avg");
        assert_eq!(canonical("table.agg"), "df.agg");
        assert_eq!(lookup("csv.write").unwrap().effect, ConstructAction);
        assert_eq!(lookup("!").unwrap().effect, ExecuteAction);
        assert_eq!(prelude_arity("*?"), Some(4));
        assert_eq!(runtime_arity("*?"), Some(4));
        assert_eq!(runtime_arity(".field"), Some(1));
        assert_eq!(runtime_arity("io.read"), Some(1));
        assert_eq!(runtime_arity(":d"), Some(1));
        assert_eq!(runtime_arity("math.quantize"), Some(2));
        assert_eq!(prelude_arity("print"), Some(1));
        assert_eq!(canonical("print"), "io.print");
        assert_eq!(runtime_arity(":q"), Some(1));
    }
    #[test]
    fn sequence_operators_canonicalize_only_the_listed_transition_spellings() {
        for (alias, primary, arity) in [
            ("take", "[:", 2),
            ("drop", ":]", 2),
            ("slice", "sect", 3),
            ("flat", ",.", 1),
            ("tr", "|:", 1),
        ] {
            let spec = lookup(primary).unwrap();
            assert_eq!(canonical(alias), primary);
            assert_eq!(lookup(alias), Some(spec));
            assert_eq!(for_operation(spec.op.unwrap()), Some(spec));
            assert_eq!(prelude_arity(alias), Some(arity));
            assert_eq!(prelude_arity(primary), Some(arity));
        }
        assert_eq!(canonical("rev"), "rev");
        assert_eq!(prelude_arity("@/"), Some(2));
        assert_eq!(for_operation(CoreOp::Reduce), lookup("@/"));
        assert_eq!(prelude_arity("statistics.mean"), None);
    }

    #[test]
    fn dense_operators_keep_named_aliases_and_native_visibility() {
        assert_eq!(lookup("dim"), lookup("arr.shape"));
        assert_eq!(prelude_arity("dim"), Some(1));
        assert_eq!(
            lookup("form").and_then(|entry| entry.op),
            Some(CoreOp::ArrayForm)
        );
        assert_eq!(prelude_arity("form"), Some(2));
        assert_ne!(lookup("form"), lookup("arr.reshape"));
        for (member, arity) in [
            ("axes", 2),
            ("sect", 3),
            ("red", 3),
            ("fold", 4),
            ("scana", 4),
            ("dot", 2),
            ("mul", 2),
            ("diag", 1),
        ] {
            let qualified = format!("arr.{member}");
            assert_eq!(prelude_arity(&qualified), None);
            assert_eq!(runtime_arity(&qualified), Some(arity));
            assert!(module_items("arr")
                .unwrap()
                .any(|item| item == (member, arity)));
        }
    }

    #[test]
    fn registered_numeric_rules_fix_result_domains() {
        use Numeric::*;
        let domain = |name| BinaryDomain::from_rule(lookup(name).unwrap().rule).unwrap();
        for op in ["+", "-", "*"] {
            for value in [N, Z, Q, D, R] {
                assert_eq!(domain(op).result(op, value), Ok(value));
            }
        }
        for op in ["//", "%", ".."] {
            assert_eq!(domain(op).result(op, N), Ok(N));
            assert_eq!(domain(op).result(op, Z), Ok(Z));
            for value in [Q, D, R] {
                assert!(domain(op).result(op, value).is_err());
            }
        }
        for value in [N, Z, Q, D] {
            assert_eq!(domain("/").result("/", value), Ok(Q));
        }
        assert_eq!(domain("/").result("/", R), Ok(R));
        assert_eq!(domain("^"), BinaryDomain::Power);
    }

    #[test]
    fn joins_are_symmetric_and_keep_approximate_separate() {
        use Numeric::*;
        for a in [N, Z, Q, D, R] {
            assert_eq!(promote(a, a), Some(a));
            for b in [N, Z, Q, D, R] {
                assert_eq!(promote(a, b), promote(b, a));
            }
        }
        assert_eq!(promote(N, Z), Some(Z));
        assert_eq!(promote(N, D), Some(D));
        assert_eq!(promote(D, Q), Some(Q));
        assert_eq!(promote(Q, R), None);
    }
}
