# Draft 0.9: constraint inference and typed elaboration plan

Base: Draft 0.8, commit `3e36302`. Keep the current grammar, exactness,
rank-one inference, and typed interpreter. This change precedes the separate
specification, algebraic data type, row polymorphism, and array algebra changes.

1. Give each parsed expression occurrence a deterministic, program-local ID.
   Carry it through resolution, inference, and HIR provenance. Distinguish
   identical sibling expressions and include synthetic tacit nodes. Check
   equivalence after cloning or reparsing identical source.
2. Audit type-scheme boundaries and numeric/class constraints. Preserve value
   restriction, rigid checking and recursive group semantics. Specify the
   defaulting and numeric evidence rules and improve failing-constraint
   diagnostics with regression specimens.
3. Validate elaborated HIR before entering the reference evaluator. Verify
   expression references, symbol ownership, and required operation/schema
   witnesses without turning generic evaluation into runtime type inference.
4. Use independent review to stress invariants and address findings. Run the
   locked Rust checks, public example corpus, and existing oracles; document
   any unavailable verification. Only then open the implementation PR.
