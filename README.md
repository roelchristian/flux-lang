# Flux

Flux is a statically typed, predominantly functional, array-oriented programming
language with strict Polish expressions and an ASCII source format. It is
designed for numerical work, tabular analysis, and scripting. This repository
contains the Draft 0.4.1 typed error extension and a reference interpreter
in Rust, not a stable language release.

```flux
<< "io" : io;
sq(x:z) := * x x;

!{
    x := .. 1 10;
    ^{
        ^? ! io.print (@: &sq x);
        ()
    }
};
```

The single executable block runs when invoked as a script. Pure declarations
stay outside it; importing a library does not run an entry block.

## Run the prototype

```sh
cargo test --locked
cargo run -- examples/permutation.flx
cargo run -- examples/observations.flx
cargo run -- examples/csv-grid.flx
cargo run -- examples/pipeline.flx
cargo run -- examples/monitoring.flx
cargo run -- check examples/observations.flx
cargo run -- fmt examples/observations.flx
cargo run -- hir examples/permutation.flx
cargo run -- run-hir examples/permutation.flx
cargo run --
```

`cargo install --path .` installs `flux`. The interactive shell accepts
single-line declarations or expressions. Try `:type`, `:schema`, `:history`,
`:load`, or `:help`, and exit with `:quit`. The monitoring example refuses to
overwrite an existing `summary.csv`. The early
formatter writes to stdout, normalizes line endings and removes trailing
whitespace, and checks that the token stream is unchanged.

## Recoverable failure

Use `^{ ... }` to return a result, `^: error` to raise, `^? result` to
propagate, and `^= result { ... }` to recover. These operations are checked
statically. The ordinary `/` operator still produces a fatal diagnostic on a
zero divisor; use `math.div` for checked, exact division.

The I/O signatures changed in Draft 0.4.1. A print now returns an action
of type `!(()^ioerr)`; executing that action yields a result, not bare unit.
File and CSV actions likewise return typed results. A script must handle or
propagate them explicitly.

## Specification

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
generics and open structural rows. The ordinary CLI
uses typed execution where available and a checked Draft 0.3 path at explicit
migration boundaries. Use `run-hir` to require typed execution. Draft 0.4.1 error and I/O operations never use the compatibility evaluator. A
columnar engine remains outside this revision.
