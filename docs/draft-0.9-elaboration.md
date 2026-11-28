# Flux Draft 0.9: Inference and typed elaboration

## Type schemes and the value restriction

Flux uses rank-one type schemes. Each reference to a generalized binding
freshens its quantified variables and their obligations. A type variable
shared with the surrounding environment is not generalized: closures can
capture monomorphic values, but an annotation cannot make such a capture
polymorphic.

A named function has a scheme once its recursive group is checked. For a
value binding, generalization is restricted to nonexpansive right-hand
sides: literals, unit, names, function references, and lambdas. Tuples,
sequences and records qualify when every contained expression qualifies.
Tacit function values qualify through their ordinary reference or lambda
lowering. Value-producing calls, applied constructors, pipelines and
fallible or action blocks do not qualify merely because their effects are
suspended or their results would be immutable. A type variable left
unresolved by an expansive binding is one shared monotype, not a fresh
variable at every use. An explicit quantified annotation on such a
binding is an error; a concrete annotation is allowed.

An explicit type variable in a named declaration's signature is rigid
while the declaration is checked. Its body cannot specialize it, identify
two distinct quantified variables, or let it escape into an enclosing
monomorphic binding. Class obligations on it must be declared in that
signature. Type variables in a lambda's parameter annotations are
inference variables within that lambda, not a separate universally
quantified signature. An enclosing eligible value binding can generalize
them in its ordinary scheme.

## Recursion and current limitations

Top-level dependency groups are checked in dependency order. During
checking, recursive references share monomorphic placeholders; completed
eligible groups can be instantiated polymorphically by later callers.
Polymorphic recursion is not inferred, and eager value cycles are rejected.
An explicitly quantified self-recursive function can call itself at the
same parameter types. Separately quantified annotations in one mutually
recursive group remain independently rigid; they are not silently equated.

Local functions can refer forward to other local functions, but the
current checker processes their declarations in source order. A forward
callee stays monomorphic until its own body and reachable local callable
dependencies have been checked. Consequently, using the same not-yet-checked
local callee at incompatible types in an earlier body is rejected; put the
callee declaration first when it does not depend on the caller. This is a
current restriction, not inferred polymorphic recursion.

## How constraints become execution evidence

The checker resolves an operator's collection shape before scalar domain
selection. Numeric operands first satisfy the numeric class obligation;
unknown compatible scalar operands may share an inference variable. Concrete
mixed exact operands use the registry's join (natural, signed, decimal, then
rational by increasing generality). Approximate reals join only with reals.
Division of exact values returns a rational, even when both are integers or
decimals. Exact ranges, quotient and remainder require integral operands.
Powers keep the base domain and require a natural exponent. An integer
literal uses its resolved numeric context; if it remains unconstrained at
defaulting, it takes type `z`. A failure during defaulting identifies the
literal's span. A comparison checks the appropriate equality or ordering
obligation and returns a boolean (lifted over a sequence where applicable).

A typed call names its selected builtin, constructor, projection or lexical
symbol. Numeric kernels use the checked result domain and recorded lift
shape; exact operand conversion is fused into those kernels to preserve
decimal scale and evaluation behavior. Indirect application uses checked
callable instances rather than inferring types from values. This revision
does not add independent coercion instructions or a general-purpose
overload search engine.

## Expression identity and HIR admission

Every expression occurrence has a program-local postorder `SyntaxId`,
including identical siblings and generated tacit expressions. Resolution,
inference and HIR provenance use the ID. An adapter for legacy borrowed
expression lookups maps addresses to IDs only within a live tree. The IDs
survive a clone or reparse with identical semantic structure, not arbitrary
source edits or changes to module linkage. The checked API rejects attempts
to reuse evidence with a different semantic tree; a matching reparse uses
its own source locations.

The production frontend validates HIR before storing an immutable prepared
program. Public entry points accepting caller-provided HIR validate before
execution too. The validator checks arena ownership and ordering, lexical
bindings, control boundaries, operation evidence and closed table layouts.
Generic layout evidence can be deferred to a checked concrete callable
instance. Admission checks consistency; it is not a second type checker.
