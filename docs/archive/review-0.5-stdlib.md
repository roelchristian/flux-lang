# P1 standard library and error combinators: implementation review

Base: exact numerics (PR #6). One registry supplies fixed arity, type
contracts, effect classification and HIR operation identity. `print` is
a deliberately small unqualified addition; executing it still requires
`!` and produces a typed result. Native namespaces add bounded input,
filesystem and process operations, Unicode text, regex search, Gregorian
dates, exact JSON and result/optional combinators.

The runtime receives stdin and script argument data explicitly. CLI and
REPL calls do not hold a command-reading stdin lock while evaluating an
expression; embeddings can provide their own buffer. The standard tests
distinguish EOF, an empty line, malformed UTF-8 and no I/O at check time.

## Independent verification

- Direct and higher-order callback tests exercise selected branches, lazy
  callback invocation and ordered failures. `res.collect` preserves the
  first failure value and origin; `res.maperr` changes its payload while
  preserving provenance.
- Public CLI probes exercise exact/wide JSON, exponent parsing, decimal
  scale, reordered record fields, duplicate-key rejection, and type-directed
  decoding. A nonterminating rational reports a typed encoding error rather
  than silently rounding. JSON uses a checked host library with depth and
  size bounds, not a parallel type inference path.
- Python Gregorian date comparisons cover leap rules and 202 dates in both
  directions under strict and normal CLI. Integration tests cover every new
  registry operation and keep prior numerical tests enabled.
- Unqualified `print` remains suspendable and shadowable by lexical
  declarations, and source-module imports do not bypass canonical typing.

The [surface contracts](draft-0.5-stdlib.md) define behavior and limits.
This initial library does not add process launching, time zones, a dynamic
JSON type, general ADT matching, or a wider prelude.
