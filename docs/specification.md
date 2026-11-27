# Flux Language Specification

**Draft 0.3 • Surface language revision**

Flux is a statically typed, predominantly functional, array-oriented language for general-purpose scripting, exact numerical work and tabular analysis. Its source is ASCII and its expressions use strict Polish notation. This document changes the surface rules in [Draft 0.2](draft-0.2.md); sections of that baseline not superseded here still describe the intended language. The old spreadsheet specimens and old program-entry rule are superseded. A draft rule is a design commitment, not a claim that the Rust prototype implements it. See [Implementation Notes](implementation.md) for that distinction.

| Status | Scope |
|---|---|
| Adopted in this draft | Flux name; anonymous executable body; unit from a trailing-semicolon effect block; tuple application with `@@`; removal of dedicated I/O signs from the core |
| Working conventions | `.flx` extension and `flux` CLI spelling; command/API details can change before release |
| Provisional language/library proposals | Anonymous records `{. … }`, structural records and row polymorphism; table overload of `=.`; module-library signatures below |
| Carried forward | `[:` remains the fixed-arity take operator; a named `take` replacement is deferred |

## 1. Source, values and expressions

Identifiers match `[a-z][a-z0-9_]*`. Strings can contain escaped Unicode; source bytes are ASCII. `--` begins a line comment outside a literal. `:=` defines an immutable binding, `=` compares elementwise, and `==` compares whole structures. Whitespace is significant only for lexical boundaries. Use maximal munch for compound signs; there is no infix fallback. A qualified name such as `csv.read` accesses a module binding; `.value t` is prefix projection. Types and values occupy distinct namespaces. Application arity is a property of a name's lexical binding, not its inferred type.

```flux
sq(x:q) := * x x;
dp(x,y:[q]):q := +/ (* x y);
p(n,r:n):n := ? {
    > r n : 0;
    _     : */ (.. (+ (- n r) 1) n);
};
```

This retains nominal records, tuples, immutable sequences, dictionaries and ordered typed tables; exact `n`, `z`, `q` and `d`; approximate finite binary64 `r`; and the baseline's explicit conversions, constraints and scope rules. Private signatures may use inference; exported functions require fully specified signatures. Evaluation is strict and deterministic unless a form expressly specifies a lazy branch (e.g. conditional, guard, match or optional fallback). Do not infer a new grammar from a library example.

## 2. Program entry and effects

A source file used as a script consists of declarations followed by at most one anonymous effect block, terminated by a semicolon. It must be the last top-level item. There is no magic `main` binding; an ordinary binding called `main` does not run. An imported module contains declarations only. An entry block in an imported file is an error, not silently skipped or run.

```flux
<< "io" : io;
sq(x:q) := * x x;
!{
    x := .. 1 10;
    y := @: &sq x;
    ! io.print y;
};
```

Outside this distinguished module position, `!{ ... }` constructs a suspended action of type `!a`, without running it. `! e` executes an action in an effectful context or at the REPL, not in a pure expression. Within an effect block, definitions and semicolon-terminated expressions run in order; each expression statement discards its result. A final expression without a semicolon gives the resulting value. An empty effect block or one ending in a semicolon yields `()`; pure blocks still require a final expression. `!{ ... };` has two distinct boundaries: its inside statement semicolons say what is discarded; the final semicolon terminates the outer entry item.

Entry results are `()` for success or a natural for a process status. A portable entry should use 0–255; a larger value is a host-boundary diagnostic rather than silent truncation. A declaration-only file has no entry to dispatch. The REPL evaluates submitted expressions and reports their results; action values are not automatically executed merely by evaluating an expression.

Core `<@`, `@>` and `!>` are retired and not reserved as active operators. A frontend should recognize these obsolete spellings for targeted migration diagnostics rather than split them silently into different active tokens. Use a module action such as `! io.print x`, `! csv.read path` or `! csv.write path t`. Other formats are not special language forms. Host arguments and environment access are planned as explicitly effectful system-library operations.

## 3. First-class and tuple application

`&f` obtains a curried function value from a referenceable callable. Values that happen to be functions are not automatically applied by juxtaposition. `@ f x` applies one argument; `@@ f args` spreads a tuple value into successive unary applications of the curried function. Both operators have exactly two syntactic operands, independent of tuple length. The tuple expression can be grouped literal `(x,y)` or a variable whose static type is a tuple; unit is not a zero-argument tuple spread.

```flux
add(x,y:q):q := + x y;
f := &add;
a := @ (@ f 2) 3;
b := @@ f (2,3);
```

`@ f (x,y)` instead gives a single pair to `f`. In `@@`, evaluate the function expression and then the tuple expression once, left to right; subsequently apply tuple fields in order. This agrees with nested unary application when tuple fields and intermediate applications are total and pure, but need not give identical error ordering. Too many or ill-typed arguments are errors; supplying fewer than the curried function accepts returns a residual function.

### Structural grammar changes

```text
module       = { declaration } [ entry ] ;
entry        = effect_block ';' ;
effect_block = '!' '{' { def | expr ';' } [ expr ] '}' ;
application  = '@' expr expr | '@@' expr expr ;
```

`declaration` is the union of Draft 0.2 import, export, type declaration and definition. `application` records the fixed arity of these operators; it is not an additional overlapping parse where the generic callable rule already handles them. At the REPL, the context is submission rather than script entry. A second entry, declaration after entry, or a top-level bare expression in a file is rejected.

## 4. Tabular analysis independent of storage

A table has an ordered sequence of rows and a fixed schema. It retains duplicates and preserves row order under filter. `#:` converts a sequence of compatible rows into a table, and `:#` extracts the ordered rows. `@:` over a table returns a sequence; it does not infer a new table automatically. Named records remain the established explicit schema path. These rules apply whether input came from CSV, JSON, Parquet, a database or a manual literal.

| Purpose | Form |
|---|---|
| Project, filter, count | `.price t`, `?: (> .price t 100) t`, `# t` |
| Summarise | `+/ .price t`, `avg .price t`, `sd .price t`, `pct 95 .price t` |
| Group by key | `=: &.category t` gives a dictionary of same-kind groups, first-seen key order |
| Stable sort | `>: &.price t` ascending; `<: &.price t` descending |
| Join | `*= &.id &.id t u` inner; `*? &.id &.id t u` left |
| Concatenate | `+: t u`, requiring compatible schemas |
| Distinct | Stable `=. xs` on a sequence, so `#: (=. (:# t))` on an equality-capable row schema; direct `=. t` is provisional |

Join output contains paired rows, not automatically flattened fields. A left join wraps the right row in an optional. Duplicate matches produce all pairs in left-input then matching-right-input order. An optional absent key matches another absent key under structural equality; remove absent keys first for SQL-style null nonmatching. Shorter/longer Boolean masks are shape errors. Missing cells use declared `?a`; malformed nonempty input remains an error, not a zero or an absent value. Statistical optionals represent undefined results rather than I/O errors. An optional numeric column must be explicitly handled before a numeric aggregation.

### Exploratory measurements (proposed CSV adapter; REPL)

```flux
<< "csv" : csv;
obs :: { station:s; value:q; valid:b; };
t:#obs := ! csv.read "observations.csv";
u := ?: (&& (.valid t) (> .value t 0)) t;
# u;
avg .value u;
sd .value u;
pct 95 .value u;
g := =: &.station u;
@: (\(x)-> avg (.value x)) g;
```

Here the adapter uses the binding annotation to determine input schema; the last expression yields a keyed dictionary, not a table. A cleaned, grouped report can instead use an explicit output schema:

```flux
<< "csv" : csv;
<< "table" : df;
obs :: { station:s; value:q; valid:b; };
stat :: { station:s; count:n; mean:?q; };
summ(k:s,t:#obs):stat := stat {
    station := k;
    count   := # t;
    mean    := avg .value t;
};
!{
    t:#obs := ! csv.read "observations.csv";
    u := ?: (.valid t) t;
    a := df.agg &.station &summ u;
    ! csv.write "summary.csv" a;
};
```

Proposed adapter arities: `csv.read` takes a path and constructs a schema-directed action; `csv.write` takes path and table and constructs a unit action. Provisional `df.agg` is pure and takes key function, curried key/group summarizer, and table; it returns a table of summary records in first-seen group order. These names and signatures are design examples, not required built-ins. A financial reconciliation can use `*?` to retain unreconciled ledger rows; a time series can be sorted by validated timestamps before windowing; and a batch job can explicitly run read, transform, then write actions. Timestamp/window APIs and write/overwrite policy belong to a library draft.

### In-memory reconciliation and cleaning (language-level specimen)

```flux
ledger :: { id:s; amount:q; };
receipt :: { id:s; received:q; };
book:#ledger := #[ledger { id := "a1"; amount := 12q; }
                  ledger { id := "b2"; amount := 9q; }];
paid:#receipt := #[receipt { id := "a1"; received := 12q; }];
paired := *? &.id &.id book paid;
unpaid := ?: (~ (@: &?+ (.2 paired))) paired;
```

The left join keeps both ledger rows; the mask retains rows for which the optional receipt is absent. Matching receipt presence is not evidence that its amount balances: a production report compares amounts separately. To deduplicate a same-schema import without a proposed table overload, use `#: (=. (:# t))`; to inspect the first ten rows after a stable ascending sort by a declared temporal key, use `[: 10 (>: &.timestamp t)`. The latter orders already-validated keys, not raw timestamps in arbitrary string formats. In a batch, repeat the same classification for each input and concatenate compatible reports before running one write action.

### Structural extension under investigation

```flux
a := df.map (\(x)-> {.
    station := .station x;
    squared := ^ .value x 2;
}) t;
```

Neither `{.` nor `df.map` is normative yet. The example illustrates a pure schema-changing transformation; before adoption specify duplicate/missing fields, row field ordering, equality, inference, and whether anonymous schemas can escape module boundaries. A concrete `#obs` currently does not mean “any row with a `value` field.” Do not promise such polymorphism from the example.

## 5. Priorities after 0.3

Draft 0.4 should settle typed result/propagation (distinct from `?a` and `!a`), numeric overload and conversion inference, structural records/row polymorphism, nullable operations and module interfaces. Draft 0.5 should specify libraries for data adapters, dimensional/temporal semantics, statistics, strings and filesystem operations. Iteration, efficient collection folds, foreign/Arrow interoperability, controlled mutation and tensors follow as separate designs; concurrency is later. Keep strict deterministic semantics even where the implementation introduces lazy range or table representations.
