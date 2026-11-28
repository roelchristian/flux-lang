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
}
