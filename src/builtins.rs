//! Shared identities, visibility, arity and behavior classification for Flux primitives.
//! Availability describes implemented runtime signatures, not just recognized spellings.
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Builtin {
    pub canonical: &'static str,
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

macro_rules! pure {
    ($name:literal, $arity:literal, $rule:ident, $shape:expr) => {
        pure!($name, $arity, $rule, $shape, NotRequired, Runtime, &[])
    };
    ($name:literal, $arity:literal, $rule:ident, $shape:expr, $eq:expr, $avail:expr, $aliases:expr) => {
        Builtin {
            canonical: $name,
            aliases: $aliases,
            arity: $arity,
            effect: Pure,
            rule: Rule::$rule,
            shape: $shape,
            equality: $eq,
            availability: $avail,
            prelude: true,
        }
    };
}

macro_rules! internal {
    ($name:literal, $arity:literal, $rule:ident, $shape:expr, $effect:expr, $avail:expr, $aliases:expr) => {
        Builtin {
            canonical: $name,
            aliases: $aliases,
            arity: $arity,
            effect: $effect,
            rule: Rule::$rule,
            shape: $shape,
            equality: NotRequired,
            availability: $avail,
            prelude: false,
        }
    };
}

/// Registry of exact names; selectors such as .field are generated forms, not entries.
pub static REGISTRY: &[Builtin] = &[
    pure!("+", 2, Arithmetic, Elementwise),
    pure!("-", 2, Arithmetic, Elementwise),
    pure!("*", 2, Arithmetic, Elementwise),
    pure!("/", 2, Division, Elementwise),
    internal!("math.div", 2, Division, Scalar, Pure, Runtime, &[]),
    pure!("//", 2, IntegralArithmetic, Elementwise),
    pure!("%", 2, IntegralArithmetic, Elementwise),
    pure!("^", 2, Power, Elementwise),
    pure!("-:", 1, Negation, Elementwise),
    pure!("^/", 1, SquareRoot, Elementwise),
    pure!("|", 1, Arithmetic, Elementwise),
    pure!("<", 2, Order, Elementwise),
    pure!("<=", 2, Order, Elementwise),
    pure!(">", 2, Order, Elementwise),
    pure!(">=", 2, Order, Elementwise),
    pure!(
        "=",
        2,
        Equality,
        Elementwise,
        Equality::Elementwise,
        Runtime,
        &[]
    ),
    pure!(
        "/=",
        2,
        Equality,
        Elementwise,
        Equality::Elementwise,
        Runtime,
        &[]
    ),
    pure!(
        "==",
        2,
        StructuralEquality,
        Scalar,
        Equality::Structural,
        Runtime,
        &[]
    ),
    pure!(
        "~=",
        3,
        ApproxEquality,
        Dependent,
        Tolerance,
        Recognized,
        &[]
    ),
    pure!("~", 1, Boolean, Elementwise),
    pure!("&&", 2, Boolean, Elementwise),
    pure!("||", 2, Boolean, Elementwise),
    pure!("#", 1, Cardinality, Reduction),
    pure!("..", 2, Range, Collection),
    pure!("@", 2, Apply, Dependent),
    pure!("@@", 2, Apply, Dependent),
    pure!("@:", 2, Map, Collection),
    pure!("?:", 2, Filter, Collection),
    pure!(
        "=:",
        2,
        Group,
        Collection,
        Equality::Structural,
        Runtime,
        &[]
    ),
    pure!(">:", 2, Sort, Collection),
    pure!("<:", 2, Sort, Collection),
    pure!("+:", 2, Concat, Collection),
    pure!("&:", 2, Household, Collection),
    pure!("*:", 2, Household, Collection),
    pure!(
        "=.",
        1,
        Household,
        Collection,
        Equality::Structural,
        Runtime,
        &[]
    ),
    pure!(
        ".:",
        2,
        Lookup,
        Dependent,
        Equality::Structural,
        Runtime,
        &[]
    ),
    pure!("[:", 2, Household, Collection),
    pure!("/:", 3, Fold, Reduction),
    pure!("\\:", 3, Fold, Collection),
    pure!("+/", 1, Reduction, Reduction),
    pure!("*/", 1, Reduction, Reduction),
    pure!("&&/", 1, Reduction, Reduction),
    pure!("||/", 1, Reduction, Reduction),
    pure!(">/", 1, Reduction, Reduction),
    pure!("</", 1, Reduction, Reduction),
    pure!(
        "avg",
        1,
        Statistics,
        Reduction,
        NotRequired,
        Runtime,
        &[
            "statistics.avg",
            "stats.avg",
            "statistics.mean",
            "stats.mean"
        ]
    ),
    pure!(
        "pct",
        2,
        Statistics,
        Reduction,
        NotRequired,
        Runtime,
        &["statistics.pct", "stats.pct"]
    ),
    pure!(
        "sd",
        1,
        Statistics,
        Reduction,
        NotRequired,
        Runtime,
        &["statistics.sd", "stats.sd"]
    ),
    pure!(
        "cor",
        2,
        Statistics,
        Reduction,
        NotRequired,
        Recognized,
        &["statistics.cor", "stats.cor"]
    ),
    pure!("take", 2, Household, Collection),
    pure!("drop", 2, Household, Collection),
    pure!("slice", 3, Household, Collection),
    pure!("rev", 1, Household, Collection),
    pure!("flat", 1, Household, Collection),
    pure!("tr", 1, Household, Collection),
    pure!(
        "med",
        1,
        Statistics,
        Reduction,
        NotRequired,
        Runtime,
        &["statistics.med", "stats.med"]
    ),
    pure!(
        "var",
        1,
        Statistics,
        Reduction,
        NotRequired,
        Runtime,
        &["statistics.var", "stats.var"]
    ),
    pure!("?0", 0, Optional, Scalar),
    pure!("?1", 1, Optional, Dependent),
    pure!("?+", 1, Optional, Scalar),
    pure!("??", 2, Optional, Dependent),
    pure!("?!", 1, Optional, Dependent),
    pure!("#:", 1, Table, Collection),
    pure!(":#", 1, Table, Collection),
    pure!(
        "*=",
        4,
        Table,
        Collection,
        Equality::Structural,
        Recognized,
        &[]
    ),
    pure!(
        "*?",
        4,
        Table,
        Collection,
        Equality::Structural,
        Recognized,
        &[]
    ),
    internal!("!", 1, Effect, Dependent, ExecuteAction, Runtime, &[]),
    internal!("#[", 1, Table, Collection, Pure, Runtime, &[]),
    internal!(":n", 1, Conversion, Elementwise, Pure, Runtime, &[]),
    internal!(":z", 1, Conversion, Elementwise, Pure, Runtime, &[]),
    internal!(":q", 1, Conversion, Elementwise, Pure, Runtime, &[]),
    internal!(":d", 1, Conversion, Elementwise, Pure, Recognized, &[]),
    internal!(":r", 1, Conversion, Elementwise, Pure, Runtime, &[]),
    internal!(
        "io.print",
        1,
        Effect,
        Dependent,
        ConstructAction,
        Runtime,
        &["io.println"]
    ),
    internal!(
        "io.read",
        1,
        Effect,
        Dependent,
        ConstructAction,
        Runtime,
        &[]
    ),
    internal!(
        "io.write",
        2,
        Effect,
        Dependent,
        ConstructAction,
        Runtime,
        &[]
    ),
    internal!(
        "io.exists",
        1,
        Effect,
        Dependent,
        ConstructAction,
        Runtime,
        &[]
    ),
    internal!(
        "csv.read",
        1,
        Effect,
        Dependent,
        ConstructAction,
        Runtime,
        &["io.read_csv"]
    ),
    internal!(
        "csv.write",
        2,
        Effect,
        Dependent,
        ConstructAction,
        Runtime,
        &["io.write_csv"]
    ),
    internal!(
        "df.agg",
        3,
        Table,
        Collection,
        Pure,
        Runtime,
        &["table.agg"]
    ),
    internal!(
        "df.extend",
        2,
        Table,
        Collection,
        Pure,
        Runtime,
        &["table.extend"]
    ),
];

pub fn lookup(name: &str) -> Option<&'static Builtin> {
    REGISTRY
        .iter()
        .find(|b| b.canonical == name || b.aliases.contains(&name))
}

pub fn canonical(name: &str) -> &str {
    lookup(name).map_or(name, |b| b.canonical)
}

pub fn prelude_arity(name: &str) -> Option<usize> {
    lookup(name)
        .filter(|b| b.prelude && b.canonical == name)
        .map(|b| b.arity)
}

// Export visibility is separate from compatibility aliases, some of which may
// exist only for internal evaluator dispatch. Resolve arity from REGISTRY.
type Member = (&'static str, &'static str);
const IO: &[Member] = &[
    ("print", "io.print"),
    ("println", "io.println"),
    ("read", "io.read"),
    ("write", "io.write"),
    ("exists", "io.exists"),
    ("read_csv", "io.read_csv"),
    ("write_csv", "io.write_csv"),
];
const MATH: &[Member] = &[("div", "math.div")];
const CSV: &[Member] = &[("read", "csv.read"), ("write", "csv.write")];
const TABLE: &[Member] = &[("agg", "df.agg"), ("extend", "df.extend")];
const STATISTICS: &[Member] = &[
    ("avg", "avg"),
    ("mean", "avg"),
    ("pct", "pct"),
    ("sd", "sd"),
    ("cor", "cor"),
];

pub fn module_items(path: &str) -> Option<impl Iterator<Item = (&'static str, usize)>> {
    let members = match path {
        "io" => IO,
        "csv" => CSV,
        "math" => MATH,
        "df" | "table" => TABLE,
        "stats" | "statistics" => STATISTICS,
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
                    prelude_arity(a).is_none(),
                    "alias should not leak into prelude"
                );
            }
        }
    }
    #[test]
    fn module_exports_have_registry_arity_and_legacy_unsupported_spellings() {
        for module in ["io", "csv", "table", "df", "stats", "statistics", "math"] {
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
        assert_eq!(runtime_arity("*?"), None);
        assert_eq!(runtime_arity(".field"), Some(1));
        assert_eq!(runtime_arity("io.read"), Some(1));
        assert_eq!(runtime_arity(":d"), None);
        assert_eq!(runtime_arity(":q"), Some(1));
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
