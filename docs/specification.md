# Flux language reference

This is the normative reference for the implemented Draft 0.9 language.
Unless marked **Limited** or **Proposed**, the rules describe the behavior
of the Rust reference interpreter. Limited features have the scope stated
here. Proposed features are not available and do not reserve a design.
For dated proposals, read the [archive](archive/README.md); for compiler
structure and validation, read the [implementation guide](implementation.md).

The linked runnable programs are part of [the checked example corpus](../examples/README.md).
The fragments in tables and grammar blocks illustrate individual rules;
they are not complete programs.

## Source, grammar and layout

Flux source is ASCII. An identifier matches `[a-z][a-z0-9_]*`; `_` is a separate wildcard. Qualified names such as `csv.read` and selectors such as `.reading` and `.1` are distinct tokens. The lexer takes the longest applicable token. Keep compound punctuation together, and use whitespace where adjacent characters would change the token: `-2z` is a literal, whereas `- 2z 1z` is a call. Double quotes enclose strings and single quotes enclose a single character; escapes include `\n`, `\t`, and `\u{HEX}` for a Unicode scalar. The Boolean literals are `0b` and `1b`. A type variable, for example `'a`, has no closing quote. Unescaped control characters are not allowed in literals. Outside literals, `--` begins a line comment; a standalone `---` line is also a documentation convention. Comments have no runtime effect.

The following grammar is schematic: `TYPE` is the type grammar below, and a direct call takes exactly the operands declared by its lexical binding. Curly braces in the grammar's quoted terminals are source characters; unquoted braces mean repetition.

```text
file        = { import | export | declaration } [ action ";" ] ;
declaration = name [ "(" parameters ")" ] [ ":" TYPE ] [ qualifiers ]
              ":=" expression ";"
            | name "::" "{" { field ":" TYPE ";" } "}" ";"
            | name "::" "{" variant { variant } "}" ";" ;
variant     = "|" name { payload-type } ";" ;
block       = "{" { declaration } expression "}" ;
action      = "!{" { declaration | expression ";" } [ expression ] "}" ;
fallible    = "^{" { declaration | expression ";" } expression "}" ;
parameters  = parameter { "," parameter } ;
parameter   = name [ ":" TYPE ] ;
```

An import, export, or declaration cannot follow the final anonymous entry action. There is no special `main` name. A library has no entry. Types and values have distinct namespaces. Consecutive parameter names can share the type written on the last name of their group: `add(x, y: z): z`. A parameter list can omit all types; giving the result type of a named function requires its parameter types as well. Signature qualifiers belong to complete named-function signatures. See the [function examples](../examples/basics/functions.flx) and [geometry module](../examples/modules/geometry/demo.flx).

Indentation and line endings are not statement terminators. Four spaces per structural level, vertical operands for substantial prefix expressions, and aligned related guard/field margins are the layout convention; explicit delimiters and semicolons still determine structure. `flux fmt FILE` writes formatted source to stdout without editing the file; `flux fmt --check FILE` is quiet for canonical source and reports noncanonical or invalid input. The formatter is idempotent and preserves the token sequence, explicit parentheses, literals, and comment wording; it may change spacing and remove trailing horizontal whitespace. It formats only the requested source file, not its imported files. See the [formatted pipeline](../examples/basics/pipeline.flx).

## Expressions and evaluation

Expressions include literals, values, strict prefix calls, function references and lambdas, tuples and sequences, record and table construction, grouping, guarded cases, blocks, pipelines, and the effect and typed-failure forms described below. There is no infix fallback. Parentheses around one expression group it; `()` is unit; `(a, b)` is a tuple with two or more fields, and `[a b]` is an ordered sequence with no commas. Named records use `schema { field := value; }`; structural records use `.{ field := value; }`. Both require unique field names. Use `.field value` for named projection, or `.1 pair` for the first positional field. `:=` defines an immutable binding; it is not equality or assignment to an existing cell.

Ordinary evaluation is strict and evaluates operands from left to right. The conditional `? CONDITION YES NO` selects one branch. In `?{ CONDITION : VALUE; ... _ : DEFAULT; }`, guards are tested in order, only the chosen value is evaluated, and the final default is required. Boolean `&&` and `||` are ordinary strict operations, not short-circuit control flow. Optional fallback `?? FALLBACK OPTIONAL` checks the optional first and evaluates the fallback only on absence. Typed recovery evaluates an arm only on the matching failure. See [guards](../examples/basics/guards.flx) and [optionals](../examples/library/optionals.flx).

A pure block `{ ... }` contains local declarations and a required final expression; use `()` for a unit result. A pipeline `|{ SOURCE; STAGE; ... FINAL }` evaluates its source once, then evaluates each later stage in order. Within a stage, each `$` refers to the same preceding value. The pipeline itself does not bind `$` in its source expression. It needs at least one stage after the source, and the final expression has no trailing semicolon. The pipeline returns that final value. It is immediate evaluation, not a way to define a reusable function. See [pipelines](../examples/basics/pipeline.flx).

## Named and first-class functions

A named callable such as `add(x, y: z): z := + x y;` consumes two expressions when written as `add 2z 3z`. The parser determines call arity before type inference. Named callable headers are visible throughout their lexical scope, allowing forward calls and checked recursion; values held in ordinary bindings are not implicitly called. Local value initialization is sequential: an eagerly evaluated expression cannot read a later, uninitialized local value. A parameter or local name can shadow an outer name, and the visible binding determines whether the token is a callable or a value.

`&add` obtains the curried function value of a referenceable callable. Builtin operators and selectors can also be referenceable, for example `&+` and `&.reading`; not all special forms can be referenced. Write a lambda as `\x -> EXPR` or `\(x: z, y: z) -> EXPR`; explicit action construction, not a lambda alone, is needed to suspend effects.

- `@ F X` applies a function value to one argument. If `X` is a tuple it stays one argument.
- `@@ F ARGS` spreads a tuple value into successive unary applications. It evaluates `F` and then the tuple once before applying its fields in order. It requires a tuple, not unit. Supplying fewer arguments than the curried function accepts leaves a function.
- `@: F XS` maps explicitly over a supported collection. A user-defined function does not automatically lift over a sequence or table.

See [functions and inference](../examples/basics/functions.flx) and [explicit mapping](../examples/basics/collections.flx).

## Tacit function algebra

A tacit block builds an ordinary first-class function. Its content is one strict prefix expression:

```text
tacit-block = "&{" tacit-expr "}" ;
tacit-expr  = "." | callable-or-function-value
            | "@." tacit-expr tacit-expr
            | "@*" tacit-expr tacit-expr tacit-expr
            | "@?" tacit-expr tacit-expr
            | "(" tacit-expr ")" ;
```

Use a bare referenceable callable, operator, nonnullary constructor, selector, or lexical function-valued binding as an atom. Do not prefix it with `&` inside the block. Scalars, action execution and pipeline `$` are not tacit atoms. Outside a block, the standalone adjacent spelling `&.` gives identity; bare period and the tacit combinators are not ordinary expressions. Compact ordinary application such as `@.field` or `@.1` still uses the existing selector token. Separate tacit operators and operands when needed to retain their token boundaries.

| Tacit form | Applied to `x` |
| --- | --- |
| `.` | `x` |
| `@. G H` | `@ G (@ H x)` |
| `@* F G H` | `@@ F ((@ G x), (@ H x))` |
| `@? F H` | `@@ F (x, (@ H x))` |

A fork evaluates the left branch and then the right exactly once each before applying the combiner. A hook uses identity for its left branch, not a condition. Tacit functions infer and generalize under the ordinary checker, including its restrictions on numeric lifting and open rows. Constructing a tacit value does not run its body, and applying a tacit function does not implicitly execute actions or unwrap optionals or results. A tacit declaration such as `square := &{ @* * . . };` binds a value: call it as `@ square 5z`, not `square 5z`. See [composition](../examples/tacit/composition.flx), [forks](../examples/tacit/forks.flx), and [hooks](../examples/tacit/hooks.flx).
## Types and inference

Flux is statically typed. An annotation uses `:` in a binding or parameter
declaration. Omitted types are inferred where there is sufficient context.
A type annotation is not a numeric conversion; use an explicit conversion
when the value has the wrong domain.

| Form | Meaning |
|---|---|
| `n`, `z`, `q`, `d`, `r` | Natural, signed integer, exact rational, exact decimal, approximate real |
| `b`, `c`, `s`, `()` | Boolean, Unicode scalar, string, unit |
| `[a]`, `(a,b)` | Homogeneous sequence and heterogeneous tuple |
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

Flux uses rank-one polymorphism. A reference to an eligible binding
freshens its quantified variables and their constraints. Type variables
free in surrounding captures are not generalized. A named function can be
generalized once its dependency group has been checked. Value bindings are
generalized only for nonexpansive expressions: literals, unit, names,
function references and lambdas, plus tuples, sequences and records
containing only such expressions. This includes tacit values that lower to
references or lambdas. Applications, applied constructors, pipelines, and
fallible and action blocks do not qualify. Any unresolved variables in an
expansive binding are shared monotypes; a universal annotation on such a
binding is rejected, although a concrete annotation is allowed.

A type variable such as `'a` in a named signature is rigid when its body is
checked. The body cannot specialize it, equate it with another distinct
quantified variable, or make it escape into an enclosing monomorphic
binding. Type variables in a lambda's parameter annotations are local
inference variables, not a separately quantified signature. They may be
generalized by an enclosing eligible binding. For an executable example,
see [inference, annotations and function values](../examples/basics/functions.flx).

Named signatures attach class constraints with a qualifier such as
`| num 'a`. The supported classes are `num`, `eq`, `ord` and `hash`.
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

Checking resolves names and constructor identities, unifies structural
requirements and infers types. For numerical operations it resolves
scalar or one-dimensional lift shape, checks applicable constraints and
selects the checked domain and operation. The evaluator uses typed HIR
and checked callable instances for direct, indirect and tacit applications;
it does not choose a different static overload from the particular value
it happens to see. The runtime still checks dynamic bounds, compatible
sequence lengths and domain failures. The reference compiler does not
provide arbitrary overload search or a user-defined class system.

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

## Collections, records and tables

Sequences preserve order and duplicates. Tuple components may have
different types. A record has uniquely named fields; its structural type
is determined by their names and types. `.{ field := value; }` constructs
a structural record. `row :: { field:a; };` declares a nominal record and
`row { field := value; }` constructs that type. Two separately declared
nominal types are distinct even if their field layouts match. Select a
field with `.field` or a tuple position with a one-based selector such as
`.1`. A table selector returns the named column or tuple component as a
sequence.

Projection can infer a structural requirement instead of fixing all
fields: a function that projects `.value` can be used with structural
records containing appropriately typed `value` fields and other fields.
**Current limitation:** explicit open-row source annotations, general
row-polymorphic table transformations, and replacing nominal type identity
with structural compatibility are not provided. See the
[tacit projection example](../examples/tacit/projection.flx).

Tables are ordered homogeneous collections of record or tuple rows.
`#[…]` constructs a table; `#:` reconstructs one from a row sequence
or a nonempty named record of equally sized columns (which can have zero rows), and `:#`
extracts its row sequence. Field order is retained for the checked schema
and for schema-driven interchange, including CSV. Supply context for
ambiguous empty sequences or tables; an empty table still carries
its row schema. See [projection and reconstruction](../examples/tables/projection.flx)
and [checked observations](../examples/tables/observations.flx).

`@:` maps function values, and `?:` filters sequences and tables with
a compatible boolean mask or predicate, retaining source order. A mask
must have precisely the input length.
`/:` folds with a seed and `\:` scans including the initial seed.
`+:` concatenates compatible collections, preserving duplicates.
`&:` zips equal-length sequences and `*:` forms a Cartesian product.
`.:` is one-based lookup for sequences and tables, or key lookup for
dictionaries; a missing key, zero index or out-of-range index returns
absence. Dictionaries can be obtained by grouping with `=:`; dictionary
literal syntax is not executable. Nested sequences are available, but
a separate multidimensional array type, rank and general broadcasting
are proposed rather than implemented. See
[functional collections](../examples/basics/collections.flx) and
[descriptive statistics](../examples/statistics/descriptive.flx).

### Sequence operators and reductions

`# X` measures cardinality (Unicode scalar count for a string, row count
for a table). `.. LOW HIGH` is an inclusive ascending integral range;
when LOW exceeds HIGH it is empty. `take N X` (also `[: N X`) and
`drop N X` retain a prefix or omit it, clamped to length. `slice START
COUNT X` takes up to COUNT items from a one-based START; zero is an error
and an out-of-range positive start gives an empty result. These operations
and `rev X` preserve sequence, table or string kind. String positions
count Unicode scalars. `flat XSS` flattens one sequence level; `tr XSS`
transposes equal-length sequence rows and rejects ragged input.
`>: KEY X` sorts ascending and `<: KEY X` descending; equal keys retain
their input order.

Reductions consume sequences rather than tables. Project a table column
first. `+/ X` and `*/ X` return the sum and product in the checked
numeric domain, with empty identities zero and one; `&&/ X` and `||/ X`
use the Boolean identities true and false. `>/ X` and `</ X` return
optional numeric extrema, absent on empty input.

`avg X`, `med X` and `var X` take exact numeric sequences and return
an optional exact rational mean, median or population variance. `pct P X`
uses an exact P between 0 and 100 and exact numeric inputs, with linear
interpolation at zero-based position `(length - 1) * P / 100` in sorted
order. `sd X` returns optional approximate population standard deviation.
These statistics return absence for empty input; none drops optional
items silently. See [percentiles](../examples/statistics/percentiles.flx)
and [descriptive statistics](../examples/statistics/descriptive.flx).

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

An optional `?a` represents presence or ordinary absence. A result `a^e`
contains a success of type `a` or a recoverable error of type `e`. An
action `!a` suspends a computation that produces `a`. These are different
constructors: `!q^e` means `(!q)^e`; an action returning a result is
`!(q^e)`. Parenthesize nested results rather than writing `a^e^f`.

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

A script entry runs its action and may complete with unit or a natural
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

The language has checked nominal sums, recovery patterns, inferred structural
field constraints, and rank-one sequence operations. This revision does not
introduce general algebraic pattern matching, parameterized nominal sums,
explicit row constraints, distinct ranked arrays, a package registry, a
columnar engine or native compilation. The historical design notes do not
grant syntax or semantics for those features.
