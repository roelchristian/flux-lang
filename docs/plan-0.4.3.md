# Draft 0.4.3: local source modules and interfaces

Base: Draft 0.4.2 semantic consolidation (PR #4). Preserve strict Polish
arity and existing import/export surface syntax. Add synchronous, local
source imports only; no package registry, remote import or implicit execution.

1. Discover imports before expression parsing. Resolve explicit relative
   `.flx` paths from the importing file; canonicalize for identity and cache.
   DFS reports dependency cycles including the import chain, and rejects a
   library that has a script entry. Native imports still use the registry.
2. Parse modules independently from only their own lexical declarations,
   their registry imports, and interfaces of direct dependencies. Interfaces
   expose typed exported functions, annotated values, and explicit nominal
   type declarations. A function-valued exported binding stays a value.
   Constructors are available if their enclosing sum is exported. Nonexports
   remain inaccessible; transitive imports are not re-exported implicitly.
3. Bind declarations and nominal types to stable module-scoped identities.
   Resolve and check full signatures and canonical type schemes, then lower
   one typed HIR with dependency-ordered declarations and one root entry.
   Imported pure initializers run once for a canonical file; actions they
   construct remain suspended. No per-module or legacy evaluator.
4. Retain file/byte locations for frontend and runtime diagnostics; CLI and
   Rust path APIs use the loader. Expand tests from the public executable:
   finance example, private names, wrong operand/signature, aliasing and
   collisions, generic exports, diamond imports, nominal types, cycles,
   malformed library diagnostics, no execution on check, and no source-module
   fallback. Independently review correctness and revise before opening a
   stacked PR against the consolidation branch.
