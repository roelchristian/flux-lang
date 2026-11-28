# Flux

Flux is a statically typed, array-oriented language for exact arithmetic,
data transformation and scripting. It uses strict prefix notation: `+ 2 3`
adds 2 and 3. This is the experimental **Draft 0.20** reference interpreter.
The Machina runtime uses checked dense-array storage and host-controlled
[execution budgets](docs/implementation.md#runtime-policy).

## Quickstart

Use Rust 1.85 or newer. From the repository root:

```sh
cargo run --locked -- examples/basics/hello.flx
cargo install --locked --path .
```

The first command prints `Salve, munde.`; the second installs the CLI.

| Command | Purpose |
| --- | --- |
| `flux FILE` | Run a script |
| `flux check FILE` | Check a script without running it |
| `flux fmt FILE` | Print formatted source; add `--check` to verify it |
| `flux` | Open the REPL; try `+/ [1 2 3];` or `:type / 1 3;` |

## A small program

```flux
sq := &{ @* * . . };

!^{
    ! print (@: sq (.. 1z 5z));
};
```

This prints `[1 4 9 16 25]`. The tacit block multiplies two copies of its
input. `@:` maps the resulting function over a range; `!` executes the print
action, and the combined block propagates a failed unit action.

## Learn and develop

- [Example tour](examples/README.md): start with expressions, then explore
  exact numerics, typed tables and runnable science applications.
- [Language reference](docs/specification.md): the normative description.
- [Implementation and verification](docs/implementation.md): Rust source map, pipeline,
  test commands and benchmarks.
- [Changelog](CHANGELOG.md): notable changes; older specifications live in Git.
