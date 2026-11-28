//! Executable core for owned typed HIR. This backend never dispatches on surface AST.

use crate::{
    hir::{CoreOp, ExprId, ExprKind, HirProgram, LiftMode, Span, TableSchema, Target},
    types::{Prim, Type},
};
use num_integer::Integer;
use num_rational::BigRational;
use num_traits::{Signed, ToPrimitive, Zero};
use std::{cmp::Ordering, collections::HashMap, rc::Rc};
mod array;
#[path = "../stdlib/collections.rs"]
mod collections;
#[path = "../stdlib/combinators.rs"]
mod combinators;
#[path = "../stdlib/joins.rs"]
mod joins;
#[path = "../stdlib/json.rs"]
mod json;
#[path = "../stdlib/math/mod.rs"]
mod math;
#[path = "../stdlib/native/mod.rs"]
mod native;
mod numeric;
#[path = "../stdlib/reduce.rs"]
mod reduction;
#[path = "../stdlib/relations.rs"]
mod relations;
mod resources;
use array::{packed, sequence as array_sequences};
use numeric::{arithmetic, decimal, plan as numerical_plan, wide};
use value::display::{inspect, repr};
use wide::Wide;
#[path = "../stdlib/effects.rs"]
mod effects;
pub use crate::csv::{FsInput, OpenedInput, RuntimeInput};
pub use resources::{ResourcePolicy, RuntimeOptions};

// The existing runtime primitives use this facade; their policy and value
// types live in the implementation modules rather than in the dispatch layer.
mod engine;
mod value;
use arithmetic::{
    as_ratio, domain, exact_binary, exact_i128, exact_index, exact_unary, finite, lift, lift_one,
    numeric_identity, numeric_reduce, parse_exact, parse_exact_value, range_with_fuel, rational,
    real_of, scalar_binary, stats, subtract_work, to_exact, Domain, Exact, NumericResult,
    DIV_BY_ZERO, OVERFLOW,
};
use engine::{
    error, fun_arg, inner_type, invariant, operation_arity, succeeded, Control, Eval, Failure, R,
};
pub use engine::{
    run, run_with_host, run_with_host_options, run_with_io, run_with_options, run_with_output,
    view_entry, view_with_host, view_with_host_options, view_with_io, view_with_options,
    view_with_output,
};
pub(crate) use engine::{
    run_options_prevalidated, run_with_host_options_prevalidated, view_options_prevalidated,
    view_with_host_options_prevalidated,
};
use value::{
    as_bool, as_seq, collection, column_rows, equal, function, key, key_with_budget, layout, named,
    order, projection, read_cell, wrap, write_cell, Action, Function, Key, VariantValue,
    KEY_WORK_LIMIT, V,
};

const COLLECTION_LIMIT: usize = 100_000;

#[cfg(test)]
use arithmetic::{range, wide_standard_deviation};
#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
