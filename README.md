# Flux

Flux is a statically typed, array-oriented language for exact arithmetic, data
transformation and scripting. It uses strict prefix notation: `+ 2 3` means
“add 2 and 3”. Formulas compose as functions, while file and terminal operations
run only when explicitly requested.

This repository contains the **Draft 0.10** reference interpreter in Rust.
The language is still experimental.

## Quickstart

Use Rust 1.85 or newer. From the repository root:

```sh
cargo run --locked -- examples/basics/hello.flx
```

Expected output: `Salve, munde.`

Install the CLI with `cargo install --locked --path .`. Run `flux FILE` to execute
a script, `flux check FILE` to check it without running it, or `flux fmt FILE` to
print formatted source (`flux fmt --check FILE` checks formatting without
writing it). Run `flux` without arguments for the REPL; try
`+/ [1 2 3];` or `:type / 1 3;`.

## A small program

Save this as `squares.flx` and run `flux squares.flx`:

```flux
sq := &{ @* * . . };

!{
    ^{
        ^? ! print (@: sq (.. 1z 5z));
        ()
    }
};
```

It prints `[1 4 9 16 25]`. In the tacit block `&{ ... }`, each `.` supplies the
same input, and `@*` combines the two results with multiplication. The resulting
function is applied as `@ sq 5z` or mapped over a range with `@:`. The
calculation is pure; `!` executes the print action, and `^?` propagates a typed
failure.

## Explore by example

| Start here | What it shows |
| --- | --- |
| [Functions](examples/basics/functions.flx) and [tacit composition](examples/tacit/composition.flx) | Named calls, inferred types and reusable function values |
| [Exact arithmetic](examples/mathematics/exact.flx) and [rounding](examples/mathematics/rounding.flx) | Wide integers, exact fractions and explicit decimal rounding |
| [Table joins](examples/tables/joins.flx) and [calibration audit](examples/applications/calibration/README.md) | Checked rows, missing measurements and exact comparisons |
| [Source modules](examples/modules/transformations/demo.flx) and [typed results](examples/library/results.flx) | Reusable libraries and recoverable failures |

The [example tour](examples/README.md) lists all 53 runnable examples, including
concordance, meteorology, astronomy and demography. Start there for application
commands, sample inputs and expected outputs.

## Reference and development

- [Flux language reference](docs/specification.md): source, types, evaluation, tables, effects and modules.
- [Implementation guide](docs/implementation.md) and [performance](docs/performance.md).
- [Changelog](CHANGELOG.md).

Run `cargo test --locked` for the full suite or
`cargo test --locked --test example_corpus` for the public examples.
