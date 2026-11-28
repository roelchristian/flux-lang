# Flux language reference

This is the normative reference for Draft 0.16. **Limited** features
work only as stated; **Proposed** features are unavailable and do not
reserve a design. See the [example corpus](../examples/README.md) for
complete programs, the [implementation guide](implementation.md) for Rust
internals, and the [changelog](../CHANGELOG.md) for revisions.

## Source, grammar and layout

Flux source is ASCII. Identifiers match `[a-z][a-z0-9_]*`; `_` is a
wildcard. Qualified names (`csv.read`) and selectors (`.reading`, `.1`)
are distinct tokens. Longest applicable token wins: `-2z` is a literal,
`- 2z 1z` is a call. Double quotes delimit strings, single quotes
characters. Escapes include `\n`, `\t` and `\u{HEX}` (a Unicode scalar);
unescaped controls are forbidden. Booleans are `0b` and `1b`.
Outside literals `--` starts a line comment. Standalone `---` lines are
a documentation convention. Comments have no runtime effect.

In this schematic grammar, `TYPE` refers to [types](#types-and-inference),
unquoted braces denote repetition, and a prefix call takes exactly
the operands declared by its lexical binding.

```text
file        = { import | export | declaration } [ action ";" ] ;
declaration = name [ "(" parameters ")" ] [ ":" TYPE ] [ qualifiers ]
              ":=" expression ";"
            | name "/" arity [ ":" TYPE ] [ qualifiers ] ":=" expression ";"
            | name "::" "{" { field ":" TYPE ";" } "}" ";"
            | name [ "[" type-parameters "]" ] "::"
              "{" variant { variant } "}" ";" ;
variant     = "|" name { payload-type } ";" ;
block       = "{" { declaration } expression "}" ;
action      = "!{" { declaration | expression ";" } [ expression ] "}" ;
fallible    = "^{" { declaration | expression ";" } expression "}" ;
parameters  = parameter { "," parameter } ;
parameter   = name [ ":" TYPE ] ;
```

An optional final anonymous action is the script entry. Libraries have
no entry; no name, including `main`, is special. Type and value
namespaces are distinct. Named parameters may share a type as in
`add(x, y: z): z`, or all omit types. With a declared result, all named
parameters need types. Qualifiers belong to complete callable signatures.

Indentation and newlines do not determine structure; delimiters and
semicolons do. Prefer four spaces per level, vertical operands for long
expressions and aligned guards and fields. `flux fmt FILE` prints one
file without editing it or its imports; `flux fmt --check FILE` reports
noncanonical or invalid input and is quiet on success. Formatting is
idempotent and preserves tokens, parentheses, literals and comment wording,
but may change spacing and strip trailing horizontal whitespace.

## Expressions and evaluation

Calls are strictly prefix, without infix fallback. Parentheses group
one expression, `()` is unit, `(a, b)` is a tuple of at least two elements, and `[a b]` is an
ordered sequence without commas. `:=` defines an immutable binding.

Operands evaluate left to right. Exceptions are control forms:
`? CONDITION YES NO` evaluates only the selected branch;
`?{ CONDITION : VALUE; ... _ : DEFAULT; }` tests guards in order,
evaluates only the chosen value and requires a final default.
Boolean `&&` and `||` are strict. `?? FALLBACK OPTIONAL` evaluates the
optional first and its fallback only on absence.

A pure block `{ ... }` contains declarations and a required final
expression. A pipeline `|{ SOURCE; STAGE; ... FINAL }` evaluates SOURCE
once, then at least one stage. In each stage, every `$` refers to the
previous value; the pipeline does not bind it in SOURCE. The last stage
has no semicolon and supplies the result. See [pipelines](../examples/basics/pipeline.flx).

## Named and first-class functions

A named callable `add(x, y: z): z := + x y;` consumes two operands in
`add 2z 3z`. The parser knows arity before inferring types. Headers are
visible throughout their lexical scope, allowing forward calls and checked
recursion. Local value initialization is sequential; eager expressions
cannot read uninitialized locals. Parameters and local names may shadow
outer bindings, including their callable status.

`&add` obtains the curried value of a referenceable callable. Operators
and selectors such as `&+` and `&.reading` may be referenced; some special
forms cannot. Lambdas use `\x -> EXPR` or `\(x: z, y: z) -> EXPR`.

- `@ F X` supplies one argument, even if X is a tuple. Ordinary
  function-valued bindings require explicit application.
- `@@ F ARGS` evaluates F and then a tuple (not unit) once, then applies
  its fields in order. Supplying fewer arguments leaves a curried function.
- `@: F XS` explicitly maps over a supported collection. User functions
  do not automatically lift over sequences or tables.

See [functions and inference](../examples/basics/functions.flx).

### Declared arity without named parameters

`name/N := EXPR;` declares a named callable with N syntactic operands
(1–64). The name, slash and unsuffixed decimal integer must be adjacent.
Elsewhere `/` is ordinary division. EXPR is a tacit block, explicit reference
or lambda, optionally grouped. The checker verifies N successive
applications. EXPR runs once per saturated call, after the operands
evaluate left to right. A returned action stays suspended.

The optional annotation gives the complete curried type, as in
`add/2 : q->q->q := &+;`. With `add/1 : q->q->q := &+;`, the
single-operand call returns a `q->q` function. Signature qualifiers
and explicit public signatures apply. Interfaces publish N separately
from the type. Reference the callable with `&name` or its bare name in a tacit block,
without `/N`.

See [callable tacit algebra](../examples/tacit/callable.flx) and
[an imported callable](../examples/modules/callables/demo.flx).

## Tacit function algebra

A tacit block builds a first-class function from one prefix expression:

```text
tacit-block = "&{" tacit-expr "}" ;
tacit-expr  = "." | callable-or-function-value
            | "@." tacit-expr tacit-expr
            | "@*" tacit-expr tacit-expr tacit-expr
            | "@?" tacit-expr tacit-expr
            | "(" tacit-expr ")" ;
```

An atom is a bare referenceable callable, operator, nonnullary constructor,
selector or function-valued binding, without `&`. Scalars, action execution
and pipeline `$` are not atoms. Outside a block, `&.` gives identity; bare
period and combinators are not ordinary expressions. Compact `@.field` and
`@.1` still mean application followed by a selector.

| Tacit form | Applied to `x` |
| --- | --- |
| `.` | `x` |
| `@. G H` | `@ G (@ H x)` |
| `@* F G H` | `@@ F ((@ G x), (@ H x))` |
| `@? F H` | `@@ F (x, (@ H x))` |

A fork evaluates both branches once, left then right, before invoking
the combiner. A hook is a fork with identity on the left. Tacit values
follow ordinary inference, lifting and application rules. Construction
does not run the body; application does not implicitly execute actions
or unwrap optionals or results. See [composition](../examples/tacit/composition.flx),
[forks](../examples/tacit/forks.flx) and [hooks](../examples/tacit/hooks.flx).

### Explicit tacit arity

`&{N: BODY }` constructs a curried function of N independent arguments.
N is an unsuffixed decimal integer from 1 to 64. In BODY, `$1` through
`$N` denote positional arguments; the numeral is adjacent, unsuffixed
and one-based. This does not change the ordinary pipeline placeholder,
or selectors such as `.1`. Bare `.` and the hook `@?` require a unary
input context. The original `&{ BODY }` retains its existing behavior.

A branch may select an argument, use a capture, or apply a callable
or function value to all of its input arguments in order. One tuple
argument is not implicitly spread. A **function operand** (the outer
function of a composition or the combiner of a fork or hook) instead
denotes a function: a bare callable or function-valued name is not
applied to the enclosing block's arguments. An implicit tacit
expression there uses unary input; use a nested `&{N: ... }` for
explicit arity. Parentheses group a tacit expression.

| Form | Behavior |
| --- | --- |
| `@. G H` | Evaluate branch H on all inputs, then apply G to its result. |
| `@* F G H` | Evaluate G and then H on all inputs before applying curried F to both results. |
| `@{ F; G1; ...; Gk }` | Evaluate 1–64 branches from left to right, exactly once each, then apply curried F to the results in that order. Semicolons separate the combiner and branches; no trailing separator. |
| `@= EXPR` | Capture a pure ordinary expression as a constant branch or function operand. |

A capture runs once, in source order, when the tacit block containing
it is constructed, not at each application or only when saturated.
Captures cannot read the tacit positional arguments. They may construct
an action, but do not execute or unwrap it. Blocks with expansive
captures are not generalized as values. `@{` and `@=`
require adjacent punctuation inside explicitly arity-marked blocks;
ordinary application outside such blocks is unchanged.

Applying fewer than N arguments returns a residual function without
running the body. A `name/M` declaration still consumes M operands
syntactically, independent of a tacit block's N. Its right-hand side,
including captures, is constructed only after those operands evaluate.
The remaining type may be a function. See [multiple arguments](../examples/tacit/nary.flx),
[partial application](../examples/tacit/partial.flx) and
[generalized forks](../examples/tacit/generalised-forks.flx).

## Types and inference

Annotations use `:`; omitted types are inferred where context suffices.
An annotation is not a conversion.

| Form | Meaning |
|---|---|
| `n`, `z`, `q`, `d`, `r` | Natural, signed integer, exact rational, exact decimal, approximate real |
| `b`, `c`, `s`, `()` | Boolean, Unicode scalar, string, unit |
| `[a]`, `(a,b)` | Homogeneous sequence and heterogeneous tuple |
| `array[a]` | Dense homogeneous array with checked dynamic shape |
| `.{ field:a; }`, a declared type name | Structural record and nominal type |
| `#row`, `#(a,b)`, `#.{ field:a; }` | Table of nominal, positional or structural rows |
| `{k:v}` | Dictionary keyed by `k`, with values of type `v` |
| `a->b` | Function taking one `a` and returning `b`; arrows associate to the right |
| `?a`, `a^e`, `!a` | Optional, typed result, suspended action |

Prefix type constructors bind more tightly than the result constructor:
`?q^e` means `(?q)^e`, and `!q^e` is a result whose success is an action.
Write `!(q^e)` for an action that returns a result. Delimiters separate
containers, so `[q^e]` is a sequence of results, not a result containing
a sequence. Parenthesize a nested result rather than chaining bare carets.

### Schemes, constraints and recursion

A named variable uses adjacent tilde and identifier (`~name`) in types
and qualifiers; repeated occurrences in a signature share one variable.
A row tail gives the
variable row kind. Use `_` for an inferred type. Old apostrophe variables
trigger a migration error; character literals and tilde operators are
unchanged.

Flux uses rank-one polymorphism. A reference to an eligible binding
freshens its quantified variables and their constraints. Type variables
free in surrounding captures are not generalized. A named function can be
generalized once its dependency group has been checked. Value bindings are
generalized only for nonexpansive expressions: literals, unit, names,
function references and lambdas, plus tuples, sequences and records
containing only such expressions. Tacit values qualify if their captures, if any, are nonexpansive.
Applications, applied constructors, pipelines, and fallible and action
blocks do not qualify. Any unresolved variables in an
expansive binding are shared monotypes; a universal annotation on such a
binding is rejected, although a concrete annotation is allowed.

A type variable such as `~a` in a named signature is rigid when its body is
checked. The body cannot specialize it, equate it with another distinct
quantified variable, or make it escape into an enclosing monomorphic
binding. Type variables in a lambda's parameter annotations are local
inference variables, not a separately quantified signature. They may be
generalized by an enclosing eligible binding. For an executable example,
see [inference, annotations and function values](../examples/basics/functions.flx).

Named signatures attach class constraints with a qualifier such as
`| num ~a`. The supported classes are `num`, `eq`, `ord` and `hash`.
They require a numeric operand, semantic equality, a supported order, or
hashable key equality, respectively. The checker enforces these constraints
before lowering; for example, functions and actions cannot be compared
by structural equality. Join and group keys admit primitives and
recursively hashable tuple and optional values; arbitrary records and
functions are not keys. Declared qualifiers must cover the operations
used by an explicitly quantified signature.

Recursive functions use monomorphic headers within their dependency
group. Eager value cycles are rejected, and polymorphic recursion is not
inferred. Separately quantified rigid variables in mutually recursive
signatures are not silently equated. **Current limitation:** local function
bodies are checked in declaration order. A forward callee remains
monomorphic until its body and reachable local callable dependencies have
been checked; put a generic local callee first if it must be used with
different types in the caller's body.

### Checking and elaboration

The checker resolves identities, unifies structural requirements and
selects the domain and scalar or lifted shape of numerical operations.
Typed HIR carries this evidence for direct, indirect and tacit calls;
execution does not select static overloads at runtime. Dynamic bounds,
sequence lengths and domain errors still require runtime checks. There
is no arbitrary overload search or user-defined class system.

## Exact and approximate numerics

An integral literal without a suffix uses its numerical context or
defaults to `z`; an unsuffixed fractional literal defaults to exact `q`.
A suffix fixes the domain. Scientific notation requires an explicit `r`
suffix. Naturals are nonnegative. Exact integers and reduced rationals
with positive denominators are not restricted by the inline machine
representation. Decimal literals preserve their written scale:
`1.20d` is the coefficient 120 at scale 2. Decimal has no separate
negative zero. Approximate reals are finite IEEE 754 binary64 values.

- Exact mixed-domain promotion widens in the order `n`, `z`, `d`, `q`.
  There is no implicit conversion from an exact domain to `r`.
- Addition and subtraction on decimals preserve the greater scale,
  multiplication adds scales, and exponentiation multiplies the base
  scale by its natural exponent. Negation and absolute value preserve
  scale. A promoted integer enters the decimal domain at scale zero.
- The exact `/` operation returns a rational, including with integer and
  decimal operands; real `/` takes two real operands. Integral `//` is
  floor division and `% a b` is `a - b * floor(a/b)`. A nonzero remainder
  has the sign of the divisor. Powers take a natural exponent and
  preserve the base domain; the zeroth power is the multiplicative
  identity.
- Natural subtraction underflow, unchecked division by zero and
  nonfinite approximate results are runtime diagnostics. The explicit
  operation `math.div` instead returns an exact quotient or a recoverable
  `numerr` such as `divzero`.
- `:n` and `:z` require an integral value, and `:n` also requires
  nonnegativity. `:d` requires an exact finite decimal expansion;
  `math.quantize SCALE VALUE` rounds an exact value to the requested
  natural decimal scale, nearest with ties to even. It is not implicit.
- `:q` on a finite real recovers that binary64 value exactly, including
  subnormals. `:d` on a real converts the stored binary64 value exactly,
  not its shortest displayed decimal. Explicit exact-to-real conversion
  rounds directly to binary64, nearest with ties to even; overflow
  produces a diagnostic rather than infinity.

Numbers compare numerically, so decimal display scale does not affect
equality or key identity. Signed real zeros compare equal. Where allowed,
pointwise operations preserve one-dimensional sequence shape and
broadcast a scalar operand. Two sequence operands must have matching
lengths. Structural equality `==` compares whole compatible values,
without lifting. The runtime has resource bounds, including limits on
exact-number sizes and decimal scale; these are diagnostics rather
than fixed-width wrapping or rounding.

See [wide arithmetic](../examples/mathematics/exact.flx),
[exact rational calculations](../examples/mathematics/rational.flx),
[rounding](../examples/mathematics/rounding.flx), and
[approximation](../examples/mathematics/approximation.flx).

## Algebraic types and matching

Declare a nominal sum with `name :: { | constructor PAYLOAD; ... };`.
Constructor arity is its number of payload types; a constructor with no
payloads is a value. Export the sum's name to expose its constructors
as namespace-qualified members. A separately declared sum is a distinct
type, irrespective of its constructor spellings.

Use adjacent brackets to declare and apply type parameters:
`tree[~a] :: { | leaf ~a; | node tree[~a] tree[~a]; };` declares one
parameter. `tree[z]` and `tree[s]` are different applications of the
same nominal type. Multiple parameters and arguments are comma-separated.
Constructors infer their type arguments at each use, retaining the
constraints of their payloads. A phantom parameter may require an expected
type. A recursive payload can refer to the nominal type without unrolling
the declaration. Parameterized records, higher-kinded types, GADTs and
structural equality on parameterized sums are not provided.

`?= VALUE { PATTERN : BODY; ... }` evaluates VALUE once. Arms are
checked against its type and one common output type. The first matching
arm runs, with fresh immutable bindings; none of the unselected bodies
runs. The checker rejects incomplete matches and unreachable arms,
including on nested finite alternatives.

Patterns are `_` (discard), unqualified binders, visible fixed-arity
constructors, `(P, Q)` tuples, `.{ field := P; }` record fields,
`?0`/ `?1 P` optionals, `0b`/ `1b` booleans, and `()`.
A constructor in scope is interpreted as a constructor, not a binder.
Record patterns can omit fields but cannot repeat them; binders cannot
repeat in the same arm. Ranges, arbitrary scalar constants and sequence
length patterns are not implemented. Coverage checks are resource-bounded;
if a very large match is rejected, split it into smaller matches.
The [recursive tree example](../examples/basics/trees.flx) uses nested
constructors and inferred polymorphism.

## Collections, records and tables

Sequences preserve order and duplicates; tuples may mix types. Records
have unique fields. `.{ field := value; }` constructs a structural record
whose type follows its field names and types. `row :: { field:a; };`
declares a nominal record, constructed with `row { field := value; }`.
Separate nominal declarations remain distinct even with identical
fields. Project with `.field` or a one-based tuple selector such as `.1`.
A table projection yields a column sequence. Projection can infer a
structural field requirement instead of a closed record type.

Tables are ordered collections of same-type record or tuple rows.
`#[…]` constructs a table; `#:` constructs one from a sequence of rows
or a nonempty record of equal-length columns, and `:#` extracts its rows.
Empty values need context when ambiguous. Tables retain their schema,
including field order for interchange such as CSV, even with zero rows.

`?:` filters sequences or tables with a predicate or an equal-length
Boolean mask, preserving input order. `+:` concatenates compatible
collections without removing duplicates. `&:` zips equal-length sequences;
`*:` forms their Cartesian product. `.:` looks up a one-based index in
a sequence or table, or a dictionary key. Missing keys and invalid
indices yield absence. For dictionaries use grouping (`=:`); dictionary
literals are not executable.

### Open structural rows

`.{ value:q; | ~r }` requires a rational `value` field and permits other
fields. Repeated tails describe the same remaining fields, but a name
cannot represent both a scalar and a row variable in one signature.
`#.{ value:q; | ~r }` is the corresponding table type. Returning that
type preserves extra columns. Use separate variables for independent
tails. Missing or conflicting required fields are static errors.

Nominal rows do not implicitly satisfy structural annotations.
`df.structural T` views a named table structurally, preserving field
names, order, types and values even when empty. It is an identity for
already structural named-field tables. A transformation can extend the
same tail, e.g. return `#.{ value:q; squared:q; | ~r }` from
`#.{ value:q; | ~r }`, only if `squared` is absent from the tail.
Checked calls and module interfaces carry this evidence; empty tables
do not need a row to determine their schema.

Column records passed to `#:` or `df.extend` require a closed schema
of sequences. Reconstruction from a row sequence can preserve an open
tail. See the [open-row specimen](../examples/tables/open-rows.flx).

### Sequence operators and reductions

`# X` measures cardinality (Unicode scalar count for a string, row count
for a table). `.. LOW HIGH` is an inclusive ascending integral range;
when LOW exceeds HIGH it is empty. `[: N X` retains a prefix and `:] N X`
omits a prefix; both clamp N to the input length. `sect START COUNT X`
takes up to COUNT items from a one-based START; zero is an error and an
out-of-range positive start gives an empty result. These operations and
`rev X` preserve sequence, table or string kind. String positions count
Unicode scalars. `,. XSS` flattens one sequence level; `|: XSS` transposes
equal-length sequence rows and rejects ragged input.
`>: KEY X` sorts ascending and `<: KEY X` descending; equal keys retain
their input order.

Sequence reductions take sequences, not tables; project a table column
first. The [dense forms](#reduction-scan-and-contraction) use an axis. `+/ X`,
`*/ X`, `&&/ X` and `||/ X` have empty identities zero, one, true and
false, respectively. Numeric reductions use the checked domain; `>/ X`
and `</ X` return optional numeric extrema, absent on empty input.
`/: F SEED XS` folds from a seed; `\: F SEED XS` scans, including the seed.

`@/ F XS` accepts a curried `a->a->a` callback and a sequence `[a]`,
evaluates both operands once in order, then folds left from the first
element. It calls F once per remaining element and returns `?a`. Empty
and singleton inputs make no calls and return `?0` and `?1 X`,
respectively. Optional, result and action elements are not unwrapped,
propagated or executed. `@/ &- [10 3 2]` returns `?1 5`.
See [sequence calculus](../examples/basics/sequence-calculus.flx).

Write a space before the operand of `|:` when its name could be a
conversion suffix: `|: q` transposes q, while `|:q x` retains the
compact conversion-and-absolute-value form. Use `@ /` if ordinary
application and the division token must remain separate; `@/` is
reduction. Tuple separators next to selectors retain their old meaning.

Migration: `take`, `drop`, `slice`, `flat` and `tr` are transitional
aliases for `[:`, `:]`, `sect`, `,.` and `|:`, respectively. The
formatter does not rewrite aliases.

`avg X`, `med X` and `var X` take exact numeric sequences and return
an optional exact rational mean, median or population variance. `pct P X`
uses an exact P between 0 and 100 and exact numeric inputs, with linear
interpolation at zero-based position `(length - 1) * P / 100` in sorted
order. `sd X` returns optional approximate population standard deviation.
These statistics return absence for empty input; none drops optional
items silently. See [percentiles](../examples/statistics/percentiles.flx)
and [descriptive statistics](../examples/statistics/descriptive.flx).

## Dense arrays and indexed sequences

An `array[a]` has homogeneous elements of static type `a` and a
checked dynamic shape. It is distinct from a sequence `[a]`; nested
sequences may be ragged and are never implicitly flattened into arrays.
The reserved type constructor `array` takes one type argument. Elements
are stored in row-major order: the last coordinate varies fastest.
Rank-zero shape `[]` contains one element; a zero dimension makes the
array empty without discarding any dimensions.

### Construction and indexing

| Operation | Contract |
| --- | --- |
| `form SHAPE ITEMS` | Shape has type `[n]`. Consume a sequence or ravel an array in row-major order. The element count must equal the product of the dimensions; no cycling, truncation or padding. |
| `dim A`, `# A`, `,. A` | Inspect dimensions as `[n]`, count elements as `n`, or ravel to `[a]`, respectively. |
| `.: COORDS A` | Return `?a` at one-based natural coordinates. Wrong coordinate count, zero or an out-of-range coordinate gives absence. For rank zero use `[]`. |
| `@: F A` | Map a checked unary function over elements in row-major order, preserving shape. |
| `\|: A` | Reverse the order of the axes. Rank-zero and rank-one arrays are unchanged. |
| `arr.axes PERM A` | Rearrange axes in the given order, using a full, one-based permutation. |
| `arr.sect STARTS COUNTS A` | Take a rectangular section. Both vectors of naturals must match the rank. Starts are one-based; zero is invalid and an out-of-range start gives zero width on that axis. Counts clamp to the available length; zero counts give zero width. No axes are removed. |

The unqualified forms need no import. Use `<< "arr";` for qualified
operations. The existing names `arr.reshape` (sequence input),
`arr.shape`, `arr.ravel`, `arr.at` and `arr.map` remain available.
An ambiguous first-class `&form` needs a checked input type to choose
sequence or array input. On sequences the shared operators retain their
[sequence semantics](#sequence-operators-and-reductions).

### Elementwise operations and broadcasting

Arithmetic (`+`, `-`, `*`, `/`, `//`, `%`, `^`, `-:`, absolute
value and square root), ordinary comparisons (`=`, `/=`, `<`, `<=`,
`>`, `>=`), Boolean operators (`~`, `&&`, `||`) and the numerical
conversions `:n`, `:z`, `:d`, `:q`, `:r` may operate on arrays.
Unary operations keep shape and ordinary comparisons return `array[b]`.
Each operation uses the same checked element domain, exact promotion,
decimal scale and approximation rules as the scalar operation.

A scalar broadcasts across an array. Two arrays align trailing axes;
missing axes have length one. Each pair of lengths must be equal or
include a singleton, which expands to the other length. Thus zero with
one yields zero, while zero with a length greater than one is
incompatible. Sequences and arrays do not broadcast together. Shape
incompatibility is diagnosed before element operations run. Use an
annotation when inference cannot distinguish array and sequence forms.
These rules also apply to checked first-class calls. Lifted `*` is
elementwise, not matrix multiplication.

Whole-array `==` returns one Boolean. Its operands must have compatible
element types that support equality. It compares shape and corresponding
elements without broadcasting, so unequal shapes compare false even
when both arrays are empty. Arrays are not hash keys.

### Reduction, scan and contraction

Reductions `+/`, `*/`, `>/`, `</`, `&&/`, `||/`, `@/` and `/:`
act on the final axis, remove it and return an array. Rank-one input
therefore yields a rank-zero array, not a scalar. Rank-zero input has no
axis to reduce or scan. The outer cells are visited in row-major order
and each contracted axis is traversed from left to right.

- Sum, product and Boolean reductions retain scalar semantics and their
  empty identities: zero, one, true and false, respectively.
- Extrema and unseeded `@/ F A` return arrays of optionals. An empty cell
  yields absence; for `@/` only elements after the first invoke F.
- `/: F SEED A` folds each cell starting with SEED. `\: F SEED A` scans
  and includes the seed, extending the final dimension by one. A zero
  axis length gives one seed per existing outer cell, not one seed for
  the whole array.

Use `arr.red AXIS F A`, `arr.fold AXIS F SEED A` and
`arr.scana AXIS F SEED A` to select a one-based axis for an unseeded
reduction, seeded fold or scan. Axes must exist. The older
`arr.scan F SEED A` scans the final axis. A zero outer frame yields no
callbacks; mapping or reducing a zero-width cell of a nonempty frame
follows the corresponding empty-cell rules. Callbacks never execute
suspended actions, propagate typed failures or unwrap optionals on
behalf of the caller.

| Operation | Contract |
| --- | --- |
| `arr.dot A B` | Multiply and sum equal-length rank-one arrays; return a scalar. |
| `arr.mul A B` | Multiply rank-two arrays of shapes `[m k]` and `[k n]` to obtain `[m n]`. |
| `arr.diag A` | Return the main diagonal of a rank-two array as a rank-one array of length `min(m,n)`. |
| `arr.zip F A B` | Apply a checked binary function to broadcast pairs. |
| `arr.outer F A B` | Apply F to pairs in A-major then B order; concatenate the input shapes. |
| `arr.rankmap K F A` | Pass each trailing rank-K cell as an array to F. Collect results in the leading frame shape; K cannot exceed the rank. |

Dot and matrix multiplication use a statically checked numeric domain
and accumulate left to right. An empty contraction yields the additive
identity of that domain. Unlike pairwise broadcasting, contracted axis
lengths must match exactly. Multiply-accumulate steps count against the
interpreter’s execution budget. An empty leading rankmap frame invokes no
callback; a zero-width trailing cell still supplies an empty array for
each existing frame position.

### Indexed sequence operations and limits

| Operation | Contract |
| --- | --- |
| `arr.windows W XS` | Return full, overlapping windows of positive natural width W in input order. A wide window yields no rows. |
| `arr.select IS XS` | Gather by one-based natural indices, allowing duplicates and reordering. Return absence if any index is invalid. |
| `arr.grade KEY XS` | Return a stable ascending permutation of one-based indices. Evaluate KEY once per item; keys must satisfy `ord`. |
| `arr.permute IS XS` | Reorder by a complete permutation. Reject missing, repeated or invalid positions. |

The reference limits rank to 16 and each dimension or materialized
collection to 100,000. Shapes, counts and permutations are validated
before output allocation and callbacks; singleton expansion does not
copy an expanded input. See [array calculus](../examples/mathematics/array-calculus.flx),
[matrix products](../examples/mathematics/matrix-products.flx),
[axis reductions](../examples/mathematics/axis-reduction.flx) and
[indexed sequences](../examples/mathematics/arrays.flx).

## Relational operations

Table joins take a left key function, a right key function, the left
table, and the right table. A key function receives a row and returns
the same hashable type on both sides. The operations do not coerce
different key types dynamically.

| Operation | Result |
|---|---|
| `*= keyL keyR left right` | Inner join; pairs `(a,b)` |
| `*? keyL keyR left right` | Left join; pairs `(a,?b)` |
| `df.semi keyL keyR left right` | Left rows with a right match |
| `df.anti keyL keyR left right` | Left rows with no right match |

A pair-emitting join preserves left input order and right match order
within each left row. Duplicates produce all matching combinations.
A left join emits one pair with an absent right row when no match exists;
the semi/anti variants preserve each retained left occurrence only once.
The right input is keyed before the left, with one call per row keyed.
Tuple keys compare component by component. Missing optional keys match
one another, unlike SQL NULL; a match with an absent field is not the
same as an absent right row. Empty outputs preserve the checked paired or
left schema. See [joins, duplicate matches and absent keys](../examples/tables/joins.flx).

`=:` forms a dictionary of groups; `df.agg` uses a key callback and a
summarizer accepting that key followed by the group's table. First key
appearance determines group order; rows retain their input order inside a
group. Empty output uses a checked schema without evaluating a sample
summarizer. See [compound-key aggregation](../examples/tables/aggregation.flx).

`df.distinct` retains the first occurrence of each comparable full row.
`df.union` is a stable set union: visit left rows and then right rows,
retaining only first occurrences and preserving the left schema's field
order. It is not concatenation. `df.select` constructs a new table from
a checked function that produces a closed row for each input row.
`df.extend` appends nonconflicting named columns of the table's height.
`df.dropna` retains rows whose chosen optional value is present;
`df.fill` substitutes a supplied default in an optional sequence; and
`df.present` removes absences without rearranging present payloads.
Present zero is never missing. See [set operations](../examples/tables/duplicates.flx),
[nullable transformations](../examples/tables/missing.flx) and the
[calibration reconciliation](../examples/applications/calibration/README.md).

## Actions and typed failure

An optional `?a` distinguishes presence from absence; a result `a^e`
distinguishes success from recoverable failure; an action `!a` suspends
computation. See [type precedence](#types-and-inference) for compositions.

- `!{ ... }` constructs an action; `! ACTION` executes it where effects
  are permitted. A call such as `print VALUE` constructs an action, not a
  write to the host. Named function and lambda bodies are not effect
  contexts; they may return independently suspended actions.
- `^{ ... }` evaluates its statements in order and wraps its final value
  in a success. It neither permits effects on its own nor flattens a
  result-valued tail.
- `^: ERROR` evaluates the error once and exits the nearest lexical
  fallible boundary. `^? RESULT` evaluates its operand once, unwraps one
  success layer, or exits with its failure. All exits to one boundary
  must agree on the error type. A function body, lambda, or independently
  suspended action starts a new propagation context; ordinary pure
  blocks, pipeline stages and recovery arms do not.
- `^= RESULT { PATTERN : BODY; ... }` returns the success unchanged or
  evaluates only the first matching arm. An exhaustive set of nominal
  alternatives, or a final catch-all, is required. A constructor
  pattern selects by identity and binds its immediate payloads in
  declaration order; `_` discards, and a catch-all identifier binds
  the whole error. Scope is the arm. Duplicate, wrong-arity and
  unreachable alternatives are errors. A non-nominal error requires a
  catch-all. The successful arm value must agree with the result's
  success type.
- Forwarding a failure retains its diagnostic origin; raising a new
  error introduces a new origin. A discarded result is a static error,
  including an unused result-valued pipeline stage. A fatal diagnostic,
  such as ordinary division by zero or forced extraction of an absent
  optional, is not a recoverable result. Completed effects are not
  rolled back.

The final anonymous action is executed and may complete with unit or a natural
exit status in 0–255, directly or inside a successful result. Unit exits
with status 0. An uncaught error exits with status 1 and a diagnostic on
stderr; recovered errors are not printed as terminal failures. See the
[stdin example](../examples/io/streams.flx) and the
[optional example](../examples/library/optionals.flx).

## Source modules and native namespaces

`<< "module" : alias;` imports into one namespace. The alias is optional
when the registry name or source basename is a valid identifier. Native
imports resolve through the library registry. Source imports name local
relative `.flx` files, relative to the **importing file's canonical
directory**, not the working directory. Dot and parent segments are
allowed; absolute paths and remote or package fetching are not. An
import alias must be unambiguous within its source file.

Export with `>> (NAME, ...);`. Public functions require explicit
parameter and result annotations; public values require an annotation;
public nominal types expose their declared schema or alternatives.
Public interfaces cannot expose a private nominal type. An exported
function-valued binding remains a value and is applied with `@` or
`@@`; it does not acquire the prefix arity of a named function. The parser discovers exported names and arities before parsing the
importer; typed interfaces are checked after linked inference. Use the
importer alias to qualify public members, including exported source
types and constructors. Private names and transitive members are not
implicitly re-exported.

The loader identifies files by canonical path and rejects dependency
cycles, reporting the import chain. A source dependency cannot have a
script entry. A diamond import shares its dependency's nominal
identities and initializers. The program executes with one typed HIR:
dependency initializers run in dependency order once per execution,
then the root entry runs. Constructing an action does not execute it;
checking the graph runs neither initializers nor actions. Diagnostics
use each original source file's coordinates. Native error constructors
are brought into lexical scope by their namespaces (the prelude
provides `ioerr`); for example a CSV recovery pattern uses
`fileerr e`, not `csv.fileerr e`. See the
[geometry](../examples/modules/geometry/demo.flx) and
[transformations](../examples/modules/transformations/demo.flx) examples.

## Standard library and host

Native library calls retain fixed prefix arity. Arguments in the
signatures below are shown in operand order; these tables are not new
annotation syntax. See the [registry](../src/builtins.rs) for the
implemented member and alias inventory. The `df` namespace is covered
under relational data; the `math` namespace provides exact
`math.div X Y : q^numerr` and `math.quantize SCALE VALUE : d`.
`numerr` has nullary alternatives `divzero` and `range`. The
`stat` aliases `avg`/`mean`, `pct` and `sd` use the corresponding
checked numeric operations. Correlation (`cor`) is recognized but
not implemented.

| Operation | Type |
|---|---|
| `print VALUE`, `io.print VALUE`, `io.println VALUE` | `a -> !(()^ioerr)` |
| `io.read PATH` | `s -> !(s^ioerr)` |
| `io.write PATH TEXT` | `s s -> !(()^ioerr)` |
| `io.exists PATH`, `fs.exists PATH` | `s -> !(b^ioerr)` |
| `io.readln` | `!((?s)^ioerr)` |
| `fs.ls PATH`, `fs.mkdir PATH` | `s -> !([s]^ioerr)`, `s -> !(()^ioerr)` respectively |
| `fs.base PATH`, `fs.join LEFT RIGHT` | `s -> ?s`, `s s -> s` respectively |
| `sys.args`, `sys.pid`, `sys.env NAME` | `![s]`, `!n`, `s -> !((?s)^ioerr)` respectively |

Print writes the value followed by a newline. Ordinary file operations
use the process working directory and honor host permissions. Reads
require bounded regular UTF-8 files. Writes create a new file and do not
clobber an existing path. Existence checks give successful false for a
missing path and do not guarantee later access. `fs.ls` returns sorted
basenames, rejecting non-Unicode names. `fs.mkdir` creates one level.
Base and join are native lexical path operations, not sandbox or
containment guarantees. `sys.args` returns the arguments after the
CLI's `--` without the executable or script name. `sys.env` distinguishes
missing variables from invalid host text; `sys.pid` reads the process ID.
`io.readln` strips a terminating LF and its preceding CR; a final bare CR
is data. EOF is absent, a blank line is present, and invalid UTF-8 or a
line over 1 MiB is an error. An oversized read need not drain the line.

`ioerr` alternatives `missing s`, `denied s`, `exists s`, `readerr s`,
`writeerr s` and `encoding s` carry the operation subject (for example a
path, stream label or environment name), not the OS diagnostic. See
[files](../examples/io/files.flx) and
[arguments](../examples/io/arguments.flx). The Rust host API can supply a
regular-file input implementation, buffered line input, script arguments
and an output sink; this does not virtualize filesystem writes, directory
operations, or process metadata.

### CSV

`csv.read PATH` constructs `!([[s]]^csverr)` without a typed table
context, or `!(#row^csverr)` for a supported closed named schema.
`csv.write PATH TABLE` constructs `!(()^csverr)` for a table with
supported named columns. `io.read_csv` and `io.write_csv` are aliases.
Raw reads do not validate a typed header. Typed reads validate record
structure and the header, then convert cells in source-record and schema
field order, allowing reordered input columns. A correctly headed empty
table is valid. Blank optional cells represent absence; malformed nonblank
cells are not absent. Exact wide integers and rationals stay exact, and
typed decimals retain their scale. Writes use schema field order and the
same no-clobber policy as text writes.

| `csverr` alternative | Payload |
|---|---|
| `fileerr ioerr` | File or encoding failure |
| `header s` | Header or schema validation detail |
| `duplicate s`, `absent s` | Duplicate or required missing column name |
| `cellerr n s s` | Logical record number, field name, decoded text before conversion |
| `rowerr n` | Malformed logical record number |

Record numbers start at 1 and include the header when present; quoted
newlines do not advance the record number. See
[typed CSV](../examples/io/csv-typed.flx).

### Optionals and results

`?0` constructs absence in a checked optional context. `?1 VALUE`
constructs presence, `?+ VALUE` tests presence and `?! VALUE` extracts
a present payload or raises a fatal diagnostic. These are scalar
operations; use explicit mapping for a nullable sequence.

| Operation | Type |
|---|---|
| `opt.map F VALUE` | `(a->b) ?a -> ?b` |
| `opt.bind F VALUE` | `(a->?b) ?a -> ?b` |
| `opt.result ERROR VALUE` | `e ?a -> a^e` |
| `res.map F RESULT` | `(a->b) (a^e) -> b^e` |
| `res.maperr F RESULT` | `(e->f) (a^e) -> a^f` |
| `res.collect RESULTS` | `[a^e] -> [a]^e` |
| `res.partition RESULTS` | `[a^e] -> ([a],[e])` |
| `res.ok RESULT`, `res.optional RESULT` | `(a^e) -> b`, `(a^e) -> ?a` respectively |

Optional callbacks run only on presence; bind adds no optional layer.
`opt.result` evaluates its error argument normally and uses it only for
absence. Result mapping applies only to the matching branch;
`res.maperr` changes the error payload while keeping provenance.
`res.collect` preserves success order or returns the first failure from
an already evaluated sequence. Partition preserves relative order on
both sides but returns plain payloads. `res.optional` discards failures
and their provenance. None executes actions implicitly.

### Text, regular expressions and dates

`str.upper`, `str.lower` and `str.trim` take and return text using Unicode
rather than locale-specific rules. `str.split DELIMITER TEXT : [s]`
preserves empty fields, including endpoint empties for an empty
delimiter. `str.join DELIMITER PARTS : s` inserts delimiters only between
parts. `rx.find PATTERN TEXT : (?s)^rxerr` returns the first leftmost whole
match or absence. Invalid or over-complex patterns yield `badpattern s`
carrying the pattern; the Unicode regex engine excludes backreferences
and lookaround. The reference limits include a 64 KiB pattern and an
8 MiB search string. `dt.days DATE : z^dterr` and
`dt.date DAYS : s^dterr` convert between proleptic Gregorian
`YYYY-MM-DD` (years 0001–9999) and days relative to 1970-01-01. Invalid
values yield `baddate s`. There are no time zones or time-of-day operations.

### Typed JSON

`json.encode VALUE : s^jsonerr` and `json.decode TEXT : a^jsonerr` are
pure. Decode requires a concrete target from checked context, not a
dynamic JSON value. Supported shapes are `n`, `z`, `q`, `d`, `b`,
`s`, `c`, unit, optionals, sequences, tuples and closed structural or
nominal records. Approximate real `r` is not a supported decode target.
Null represents unit or absence according to the checked shape; targets
such as `?()` or nested optionals that would conflate two values are
rejected. Encode also rejects a present value that encodes as null.

JSON integer tokens can exceed machine widths; numerals are read without
conversion through binary floating point. Exact decimals preserve scale.
A rational can be encoded as a JSON number only with a finite decimal
expansion. Objects reconstruct fields in schema order and reject
duplicate, missing and unexpected keys. Duplicate keys are a shape
failure even when the JSON syntax is otherwise valid. Malformed text
yields `invalidjson s`; shape, unsupported representation and resource
failures yield `jsonshape s`. The reference limits input and output to
1 MiB and also bounds nesting, nodes and numeric expansion, including a
1,000,000-digit decode budget. The
[JSON example](../examples/library/json.flx) round-trips
`170141183460469231731687303715884105728` and `101.2300` without losing
digits or scale, then recovers explicitly from a duplicate `id` field.

## Proposed and unimplemented

Static shape-indexed types, a package registry, a columnar execution
engine and native compilation are not provided.
