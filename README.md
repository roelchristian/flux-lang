# Flux

Flux is a statically typed, predominantly functional, array-oriented programming
language with strict Polish expressions and an ASCII source format. It is
designed for numerical work, tabular analysis, and scripting. This repository
contains the Draft 0.7 typed HIR interpreter in Rust, not a stable language release.

```flux
sq := &{ @* * . . };
!{ ^{
    ^? ! print (@: sq (.. 1z 5z));
    ()
} };
```

This prints `[1 4 9 16 25]`. The pure declaration binds a function value.
Supply one argument with prefix `@ sq 5z`, or map it with `@:`. A named
callable such as `square(x:z):z := * x x;` instead takes `square 5z`.
The anonymous entry executes the suspended print action with `!`; `^?`
propagates a typed failure.

## Try the examples

From the repository root:

```sh
cargo run -- examples/basics/hello.flx
cargo run -- examples/tacit/arithmetic.flx
cargo run -- examples/mathematics/exact.flx
cargo run -- check examples/tables/observations.flx
cargo run -- run-hir examples/tacit/composition.flx
cargo test --locked --test example_corpus
```

The [example index](examples/README.md) is a guided tour from expressions and
function algebra to local modules and complete scientific and text-processing
applications. `cargo install --path .` installs `flux`. Invoke it without a file
for the REPL; try `:type`, `:schema`, `:load`, or `:help`.

## Recoverable failure

Use `^{ ... }` to return a result, `^: error` to raise, `^? result` to
propagate, and `^= result { ... }` to recover. These operations are checked
statically. The ordinary `/` operator still produces a fatal diagnostic on a
zero divisor; use `math.div` for checked, exact division.

The prelude’s `print` returns an action of type `!(()^ioerr)`; executing that action yields a result, not bare unit.
File and CSV actions likewise return typed results. A script must handle or
propagate them explicitly.

## Tacit functions
```flux
sq:q->q := &{ @* * . . };
answer := @ sq 12;
```

The bare period is identity inside a tacit block. `@.` composes functions,
`@*` combines two transformations of one input, and `@?` is the hook (a
fork with identity on the left). No new execution or coercion rules apply.

## Specification

- [Draft 0.7 tacit function algebra](docs/draft-0.7-tacit.md) and [implementation plan](docs/plan-0.7-tacit.md)

- [Draft 0.6 relational and tabular computation](docs/draft-0.6-relational.md) and [independent review](docs/review-0.6-relational.md)
- [Draft 0.5 standard library and typed combinators](docs/draft-0.5-stdlib.md) and [independent review](docs/review-0.5-stdlib.md)
- [Draft 0.5 exact numerical foundations](docs/draft-0.5-numerics.md) and [independent review](docs/review-0.5-numerics.md)
- [Draft 0.4.3 local source modules](docs/draft-0.4.3.md) and [independent review](docs/review-0.4.3.md)
- [Draft 0.4.2 semantic consolidation](docs/plan-0.4.2.md) and [independent review](docs/review-0.4.2.md)
- [Draft 0.4.1 typed errors and migration](docs/draft-0.4.1.md)
- [Draft 0.4.1 implementation review and measurements](docs/review-0.4.1.md)
- [Flux Language Specification, Draft 0.4 semantic revision](docs/draft-0.4.md)
- [Draft 0.3 surface specification](docs/specification.md)
- [Draft 0.4 migration and performance notes](docs/review-0.4.md)
- [Rust implementation notes and roadmap](docs/implementation.md)
- [Draft 0.2 baseline, preserved verbatim](docs/draft-0.2.md)
- [Executable examples](examples/README.md)
- [Performance review and benchmarks](docs/performance.md)

The language design is broader than the prototype. The migration adds inferred
generics and open structural rows. The CLI, REPL and public execution API
use the same typed HIR backend. The Draft 0.3 interpreter is available only
as an opt-in Rust test oracle. A columnar engine remains outside this revision.
