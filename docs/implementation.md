# Flux Implementation Notes

**Draft 0.3 • Rust reference prototype**

The [specification](specification.md) describes intended language behavior. The reference prototype is a subset designed to make key syntax, scope and evaluation decisions executable. Unless a checked specimen and a regression test exercise a feature, do not read its design example as a capability claim.

## Architecture

1. **Lexer:** validate ASCII source, tokenize fixed symbolic operators by longest match, retain byte spans and diagnose malformed escapes or numerals. Preserve retired spellings long enough to diagnose them. Adjacent sign/digit and the difference between `#[` and `# [` matter.
2. **Parser:** discover declaration headers and callable arities before parsing bodies. Consume exactly the declared number of operands in a direct call. Resolve names lexically: local values and parameters take zero operands even if the checker later gives them function types. A tuple-spread call always consumes two expressions. Imported callable arities require a module interface or an explicit built-in stub; never guess from juxtaposition.
3. **Checker:** use expected types for compatible literals and table schemas; do not permit type inference to reparse source. Check entry result, explicit action execution context, type-compatible operands and row projections separately from parsing. Reject or report unimplemented language features, rather than treating unknown types as dynamically typed.
4. **Evaluator:** evaluate pure values deterministically; model suspended computations as values storing their captured environment. Dispatch only the parsed anonymous entry in script mode. Pass process status across the host boundary explicitly. Keep output and file effects behind action implementations, so evaluating a declaration never performs I/O.
5. **Front end and conformance:** separate script execution, static checking and interactive/tooling commands. In conformance tests, check both positive outputs and negative diagnostics. Do not report formatting as successful if the frontend performs only syntax validation or echoes unformatted text.

## Prototype limits and next gates

| Area | Scope boundary |
|---|---|
| Arithmetic | Prototype uses checked bounded exact integer/rational arithmetic (i128); arbitrary precision, exact decimal `d` and finite binary64 `r` from the design are not implied. An overflow is an error, never wrapping. |
| Inference | Useful contextual inference is not a complete principal-type/overload solver. Carry explicit parameter/result annotations where the implementation requires them. |
| Data | Schema-aware row values and table literals are separable from connector support. The prototype CSV action reads a bounded UTF-8 regular file into `[[s]]` with the header retained; paths resolve relative to the process working directory. It does not infer or coerce a table schema and has no writer. JSON, Parquet, database adapters, `df.agg`, and columnar storage remain out of scope. |
| Modules | Module syntax and built-in aliases are distinct from separate compilation, source-module resolution, public interface validation and package/version management. |
| Effects and errors | Optional absence, delayed actions and runtime diagnostics are distinct. Typed result propagation/recovery is a subsequent design; Rust errors in the host do not expose a Flux result type. |
| REPL / formatter | Command spellings are working conventions. Use the shipped CLI and tests to determine actual command availability. |
| Future shapes | Structural/anonymous records, row polymorphism, nullable operations, tensor broadcasting and mutable buffers need semantic designs before production implementation. |

A minimum conformance ladder is: ASCII/error locations and compound signs; nested prefix expressions and lexical arity shadowing; `@` versus `@@` including tuple-as-single-argument, partial applications and invalid operand types; entry dispatch and terminal position; suspended actions versus execution; checked arithmetic and overflow; and positional and named tabular fixtures where implemented. Keep executable fixtures separate from design-only examples with provisional adapters.

## Execution roadmap

The reference interpreter comes before a production data engine. First make lexer, parser, checker, evaluator, REPL and formatter behavior testable; then specify file/module interfaces and adapters. Later, choose internal columnar representations and optimization only after a reference semantics is stable.

The target program `+/ (^ (.. 1 1000000) 2)` illustrates a future fusion opportunity: a range, numeric transform and reduction should be able to share one traversal and avoid large intermediate arrays. Laziness is an internal optimization, not an observable change in evaluation order, numerical results or failure behavior. For exact operations, define overflow as a prototype constraint rather than making it part of the eventual language; floating reductions must preserve the specified reference order unless an explicit alternative algorithm is requested.
