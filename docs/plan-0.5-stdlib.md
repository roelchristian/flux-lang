# P1: a practical standard library and typed combinators

Base: the completed exact-numerics branch (PR #6). Add ordinary named
operations without adding syntax; use the same native registry, canonical
type inference, and typed HIR as the core language.

- Introduce unqualified `print` as an alias of the existing fallible output
  action. Retain explicit `!` and explicit result handling. Keep the current
  fundamental collection/numeric prelude small.
- Provide first implementations of streams, filesystem paths and actions,
  process arguments/environment, Unicode text, regex, Gregorian date
  arithmetic, statistics aliases, and typed JSON. Retain bounded I/O,
  no-clobber writes, and stable recoverable error constructors.
- Add `opt` and `res` combinators with typed callbacks, explicit conversions,
  short-circuiting, first-failure sequencing, and provenance preservation.
  Build callback execution on checked HIR instead of inspecting runtime tags
  to infer types.
- Document signatures and failure semantics, then independently test each
  namespace and public CLI behavior. Open a separate PR on the numeric branch.
  Relational processing follows separately; process launching, general time
  zones, advanced statistics, and broad API coverage are not included here.
