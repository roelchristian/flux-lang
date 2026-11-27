# Draft 0.8 layout: independent verification

The parser exposes source-side operand ranges only on request. The
formatter consumes those boundaries together with the original tokens, so
it does not infer call arity from whitespace or reconstruct a source
program from a desugared tree. It preserves the adjacency required by
identity references and compact table-type spellings.

Independent probes covered local modules, comments next to closing
delimiters, literals that look like comments, semicolon-free collections,
multiline signatures and exports, and 60 deterministic whitespace variants.
Review caught missing margins for existing multiline collections and a
comment aligned with the closing delimiter rather than the block body.
The revised formatter also keeps simple tacit blocks compact and lays out
the branches of longer forks beneath the combiner.

The application tests check the original 52 expected outputs and generated
files in ordinary and strict-HIR execution. The same harness checks
canonical formatting of all 57 public source files, including module-only
files. Separate regressions check the no-write flag, LF and CRLF input,
idempotence, tokens, comment anchors and unchanged sample output. Explicit
parentheses are retained by the formatter. Any removed parentheses in
curated examples were checked with their original output contract.

The 64-column application heuristic is a layout trigger, not a maximum
line length. Long literals and intentionally compact syntax may still
exceed it.
