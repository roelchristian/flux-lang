# Flux

Flux is a statically typed, predominantly functional, array-oriented programming
language with strict Polish expressions and an ASCII source format. It is
designed for numerical work, tabular analysis, and scripting. This repository
contains the Draft 0.3 design and an early reference interpreter in Rust, not a
stable language release.

```flux
<< "io" : io;
sq(x:n) := * x x;

!{
    x := .. 1 10;
    ! io.print (@: &sq x);
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
cargo run -- check examples/observations.flx
cargo run -- fmt examples/observations.flx
cargo run --
```

`cargo install --path .` installs `flux`. The interactive shell accepts
single-line declarations or expressions and exits with `:q`. The early
formatter writes to stdout, normalizes line endings and removes trailing
whitespace, and checks that the token stream is unchanged.

## Specification

- [Flux Language Specification, Draft 0.3](docs/specification.md)
- [Rust implementation notes and roadmap](docs/implementation.md)
- [Draft 0.2 baseline, preserved verbatim](docs/draft-0.2.md)
- [Executable examples](examples/README.md)

The language design is broader than the prototype. Full type inference, row
polymorphism, typed error recovery, schema-directed adapters and optimized
columnar execution remain future work. See the implementation notes for the
prototype's support boundaries.
