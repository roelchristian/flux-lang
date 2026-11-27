# Syntax specification

**Draft 0.2 | Working title: unnamed | Language design memorandum**

A statically typed, functional, array-oriented programming language with an ASCII-only symbolic grammar and strict Polish expressions.

**Status.** This revision supersedes Draft 0.1, retaining its mathematical character while regularising names, conversion syntax, lexical scope, inference and the statistical vocabulary. It is a proposed normative specification of syntax and surface semantics, not a claim that a compiler already exists. New decisions are identified in the final section. The language name and file extension are intentionally left open. Code specimens use only ASCII; the specification itself uses ordinary prose. Except where expressly continued, code blocks are independent examples; lines ending in semicolons which are not declarations are REPL inputs.

## 1. Character and governing principles

The source should resemble an algebraic treatise: short declarations, uniform operators, aligned equations, and a small vocabulary of signs. The language is general-purpose, with particular emphasis on exact arithmetic, statistics, spreadsheets, relational data and numerical transformations. Functions are statically typed; exported functions require signatures, whilst private functions and local quantities may be inferred. Expressions are strict prefix, with fixed syntactic arity; parentheses may delimit an expression but never introduce infix notation. Names and type constructors are lowercase by convention and, in this draft, uppercase letters are excluded from identifiers. English grammatical keywords are unnecessary. English or other names may still be chosen for domain fields and library operations.

```text
p(n,r:n):n := ? {
    > r n : 0;
    _     : */ (.. (+ (- n r) 1) n);
};
c(n,r:n):n := ? (> r n) 0 (// (p n r) (*/ (.. 1 r)));
dp(x,y:[q]):q := +/ (* x y);
```

The first function computes permutations, the second combinations, and the third a dot product. These declarations establish the central idiom: a single typed header, a definition sign, a prefix expression and a terminating semicolon. Functions are ordinary values when explicitly referenced; sequences, tables and dictionaries are first-class. Values are immutable by default. Effects are isolated and explicit.

### 1.1 Canonical written form

The formatter preserves the strict Polish parse tree. Bare applications suit elementary operands (`+ x y`); nested operands of a larger formula may be grouped (`/ (+ a b) (- c d)`). Prefer grouping a compound operand to a reduction, and parenthesise a lambda supplied as an operand. Use aligned guarded cases for substantial alternatives, a single formula for simple definitions, and local quantities for intermediate results. Indent blocks by four spaces. End declarations and cases with semicolons; the final expression of a block has none. These are formatting conventions, not additional syntax. Whitespace remains insignificant except at lexical boundaries. Use concise mathematical names for local quantities, and descriptive field and public interface names where their meaning matters. Signs are reserved chiefly for grammar, elementary algebra and frequent collection operations; specialised operations may be ordinary named functions.

```text
sq(x:q) := * x x;
dp(x,y:[q]) := +/ (* x y);
cp(f:'b->'c,g:'a->'b,x:'a):'c := @ f (@ g x);
```

### 1.2 Design boundaries

The draft specifies a first implementation surface: tokenisation, declarations, expressions, typing syntax, collections, pattern matching, effects, modules, and a standard set of operators. Higher-rank polymorphism, dependent types, arbitrary-rank tensor broadcasting, macros, custom punctuation operators and mutable references are deliberately deferred. The compiler architecture, optimisation strategy, ABI, package system and exhaustive library API belong in subsequent documents. Examples are proposed-language source, not programs verified by an existing compiler.

## 2. Lexical grammar

A source file is an ASCII byte stream. Outside a string or character literal, spaces, tabs and line endings separate tokens but have no grammatical meaning. UTF-8 BOMs and non-ASCII source bytes are rejected. Runtime strings are Unicode and may represent any Unicode scalar through escapes or externally supplied data. There are no reserved alphabetic keywords. Primitive type names are reserved only in the type namespace. There are no reserved alphabetic value names: even a prelude function may be hidden by an ordinary lexical binding. Symbolic primitive operators are fixed. The prelude is an implicit enclosing scope, not a collection of reserved words. In particular, `n`, `z`, `q`, `d`, `r`, `b`, `s` and `c` may be used as value names.

| Lexical class | Form or rule |
|---|---|
| Identifier | `[a-z][a-z0-9_]*` |
| Type variable | `'` followed by an identifier, e.g. `'a` |
| Wildcard | `_` alone; never a binding |
| Qualified identifier | `m.f` (adjacent segments, no spaces) |
| Comment | `--` through end of line, outside literals |
| Selector | `.name` or `.1`, `.2`, etc.; contiguous |
| Delimiters | `(` `)` `{` `}` `[` `]` `,` `;` |
| Composite openers | `#[` (table), `#{` (dictionary) |
| Numerals | Decimal digits; optional adjacent negative sign |
| Symbols | Longest recognised token wins, e.g. `&&/`, `:=`, `::` |

Token boundaries follow maximal munch. `&&/` is one operator, not `&&` followed by `/`; `:q` consists of `:` and the type identifier `q`, not a single token. A case or dictionary value beginning with a conversion must separate the two colons (`key : :q x`); otherwise maximal munch yields `::`. The retired statistical spellings `+//`, `?/`, `~/` and `~:` are rejected as reserved obsolete tokens; use `avg`, `pct`, `sd` and `cor`, respectively. `?0` and `?1` are optional constructors, whereas `? 0` starts a conditional expression with an invalid numerical predicate. `-2` is a signed literal; `- 2 1` is binary subtraction; `--` begins a comment even without intervening whitespace. `1..3` is tokenised as `1`, `..`, `3`, and is still invalid as an infix range. `# [1 2]` is cardinality; `#[...]` begins a table literal. A selector `.2` is not a decimal literal: a decimal fraction must have a leading digit (`0.2`). A qualified identifier such as `m.f` is a module-qualified name, never record access. Unknown punctuation sequences are rejected or split into recognised tokens according to maximal munch, then validated by the parser.

### 2.1 Literals

| Literal | Meaning |
|---|---|
| `12`, `-12` | Nonnegative / signed integer (contextual type) |
| `12n`, `12z`, `12q` | Explicit natural, integer, rational |
| `0.125`, `0.125q` | Exact rational 1/8 |
| `12.50d` | Exact finite decimal |
| `1.25r`, `1e-3r` | Binary64 real approximation |
| `0b`, `1b` | Boolean false, true |
| `"abc"`, `"\u{00e9}"` | String, including escaped Unicode |
| `'x'`, `'\n'` | Single character (closing quote required) |
| `()` | Unit value |
| `?0`, `?1 12` | Absent optional / present optional |

An unsuffixed integral literal adopts a compatible expected numerical type, otherwise defaults to `n` if nonnegative and `z` if negative. Unsuffixed fractional literals are exact `q` unless a compatible expected type is given. Scientific notation requires the `r` suffix in 0.2. Boolean suffixes are legal only on `0` and `1`; binary numeral notation is not provided. A character literal is recognised by its closing quote and valid single-character content; otherwise `'name` is a type-variable token. Strings and characters support `\\`, `\"`, `\'`, `\n`, `\r`, `\t`, `\0`, `\u{hex}`; unescaped line breaks are invalid. An escape representing a surrogate or a value above U+10FFFF is invalid.

## 3. Types and declarations

The following primitive types occupy the type namespace. There is no distinction of case between types, functions and constructors; their roles are grammatical and lexical.

| Type | Interpretation |
|---|---|
| `n` | Arbitrary-precision natural number |
| `z` | Arbitrary-precision signed integer |
| `q` | Exact rational, normalised denominator |
| `d` | Exact finite base-ten decimal |
| `r` | Finite IEEE binary64 value |
| `b` | Boolean |
| `s` | Unicode string |
| `c` | Unicode scalar character |
| `()` | Unit (one inhabitant) |
| `[a]` | Immutable, ordered sequence of `a` |
| `(a,b)` | Fixed tuple |
| `{k:v}` | Dictionary keyed by `k`, with values `v` |
| `?a` | Optional `a` |
| `!a` | Suspended effectful computation yielding `a` |
| `a b->c` | Curried function type, `a->b->c` |
| `#row`, `#[a b c]` | Table with named record or positional tuple rows |

Here `a`, `b`, `c`, `k`, `v` and `row` in composite forms are metasyntactic placeholders, not reserved names. Actual polymorphic source uses explicit apostrophe-prefixed variables such as `'a`. Type application is prefix (`tree 'a`); arrow types associate to the right. Parentheses group types or form tuple types. `#[s q q n]` is shorthand for `#(s,q,q,n)`, not a sequence of heterogeneous values. The `#[` token requires adjacency. The expression `# [1 2]` remains a separate cardinality operation.

### 3.1 Definitions and parameters

```text
k:n := 17;
x := + k 5;
f(x,y:q):q := + (^ x 2) (^ y 2);
sq(x:q) := * x x;
g(x:'a):'a := x;
h(x,y) := + x y;
```

`:=` introduces a new immutable name. It never changes an existing binding. `=` is equality and is never the definition sign. A declaration may annotate a value (`x:q := 3;`) or let inference determine its type (`x := 3;`). In a function header, commas separate names; a colon after a contiguous group assigns one type to the whole group: `f(x,y:q,k:n):q`. Fully annotated named functions have typed parameters and a result type. Private named functions may omit the entire signature (`h(x,y) := ...;`) or annotate all parameters and infer the result (`sq(x:q) := * x x;`). Mixing annotated and unannotated parameters, or supplying only a result annotation, is deferred. An explicit constraint list is permitted only with a fully annotated signature; otherwise constraints are inferred. All exported named functions require full signatures. Inferred results and constraints are determined by the checker, never by the parser; ambiguous or insufficiently constrained recursive definitions require annotations. There must be at least one formal parameter; use a unit parameter for an operation requiring no information (`f(u:()):n := 1;`, called as `f ()`). A zero-parameter value is a binding, not a callable declaration.

Function names occur exactly once per declaration. Separate type-only function declarations and repeated equation heads are not part of the primary grammar. One body may contain any number of guarded alternatives or pattern cases.

### 3.2 Polymorphism and constraints

```text
id(x:'a):'a := x;
k(x:'a,y:'b):'a := x;
cp(f:'b->'c,g:'a->'b,x:'a):'c := @ f (@ g x);
sq(x:'a):'a | num 'a := * x x;
```

Apostrophe-prefixed variables are universally quantified at a function declaration unless captured from an enclosing type declaration. The optional `|` in a declaration header introduces a comma-separated list of constraints after the result type. `num`, `eq`, `ord` and `hash` are illustrative prelude constraint names; they are ordinary type-level names. The checker infers principal types where possible, with constrained numeric and collection overloads. A function-value binding is generalised only if its definition is a syntactic value (value restriction); recursive groups are monomorphic internally until checked. Full type inference is a design goal, not an assertion that every array overload is uniquely inferable: ambiguous constraints require an annotation.

### 3.3 Records, aliases and algebraic types

```text
pt :: { x:q; y:q; };
price :: d;
tree 'a :: { | lf 'a; | br (tree 'a) (tree 'a); };
p := pt { x:=3; y:=4; };
f(p:pt):q := + ^ .x p 2 ^ .y p 2;
```

`::` defines a type. A brace body whose members start with `|` defines a sum type; one containing `name:type;` fields defines a nominal record; a type expression defines a transparent alias. Sum constructors are declared with explicit field types; their syntactic arity is the number of field types. Constructors are ordinary callable names in the value namespace and cannot collide with another value in the same scope. The record constructor reserves the same spelling in the value namespace, though it is a special form with no ordinary callable arity. Record construction uses the declared type name followed by a brace-delimited complete field assignment; order is irrelevant, and missing, duplicate or unknown fields are errors. No implicit defaults or mutation of fields are provided. Anonymous records and row polymorphism are deferred. Record and tuple selectors are prefix: `.x p`, `.2 (a,b)`. For tables and sequences of structured values they project a column or a sequence of fields.

## 4. Expressions and evaluation

Every expression consumes exactly the tokens required by its syntactic form and arity. There is no operator precedence or infix application. All built-in callable symbols have fixed arities, as do named function declarations, prelude callables and sum constructors. A lexical name denoting a value consumes no operands; a name denoting a declared callable consumes all its syntactic operands. Neither type inference nor overload resolution is allowed to change the number of operands.

```text
/ + a b - c d;
/ (+ a b) (- c d);
f g x h y;
```

The first two expressions are identical, representing `(a+b)/(c-d)`. In the third, assuming `f` is binary and `g`, `h` unary, the parse is `f (g x) (h y)`. Whitespace does not create application groups. An ordinary call `f x y` is legal; a comma-separated `f(x,y)` passes a single tuple expression, not two operands. Since whitespace is insignificant, `f(x)` is equivalent to `f (x)` for a unary callable; the canonical spelling is `f x`. Parentheses delimit a whole expression, form a comma-separated tuple, or denote unit. Thus `(f x y)` is a grouped application, `(x,y)` is a pair, and `f (x,y)` supplies one pair to a unary callable. Unconsumed tokens before a delimiter are syntax errors.

### 4.1 Function values

```text
sq(x:q):q := * x x;
f := &sq;
@ f 4;
@: &sq [1 2 3];
g := @ &+ 2;
@ g 3;
@: (\(x)-> ^ x 2) [1 2 3];
```

`&` takes the name of a visible declared callable, non-nullary constructor, referenceable fixed-arity operator or selector and obtains its curried first-class function value; it does not accept an arbitrary expression. `@` is binary application of a function value to one argument, so `@ @ f x y` applies two arguments. Partial application is explicit (`@ &+ 2`); a bare named `+` always takes two operands. Anonymous functions are introduced by `\`, a parenthesised comma-separated parameter list and `->`; parameters may be fully annotated or inferred. Their arity is the parameter count, their body is a single prefix expression, and a lambda itself is a value. A function-valued variable is never implicitly applied. Lambdas capture lexical bindings immutably. The reference to a special form that is not an ordinary callable (e.g. `&?`, `&!`) is invalid.

### 4.2 Blocks and sequencing

```text
f(x:[q]):q := {
    a := +/ x;
    b := :q (# x);
    / a b
};
```

A pure block contains zero or more local definitions, terminated by semicolons, followed by a final expression without a semicolon. Its value is that expression. The outer definition is terminated by a semicolon after the closing brace. An empty result is written `()`. A local value must be defined before it is evaluated. Named functions in the same lexical block may be mutually recursive, and their signatures are collected before bodies are parsed. Ordinary lexical shadowing applies to both values and callable names. The nearest visible value-namespace binding determines whether an identifier consumes operands; its inferred type never changes that decision. Parameters are values (including function-valued parameters) and may hide an enclosing callable; apply them with `@`. Local named callables may hide outer values, and ordinary local values may hide outer callables or constructors. Duplicate bindings in the same scope remain invalid. Named functions in a declaration scope and module-level constructors are visible throughout their respective scopes; ordinary local values become visible only after their definitions (their initialisers see the preceding environment). A function body sees the declaration environment at its own header, augmented by its parameters and the scope's hoisted callables. Pattern variables bind only in their selected branch; an identifier resolving to a visible sum constructor in a pattern is a constructor pattern, not a new variable. Record construction and direct callable application use the nearest visible value binding. Module qualification permits access to an imported callable even if its unqualified spelling is hidden. Named pure call operands evaluate left to right; the conditional, optional fallback and pattern constructs have explicitly lazy branches.

### 4.3 Conditional and guarded expressions

```text
p(n,r:n):n := ? {
    > r n : 0;
    _     : */ (.. (+ (- n r) 1) n);
};
s(x:z):z := ? {
    < x 0 : -1;
    > x 0 :  1;
    _     :  0;
};
```

`? c x y` evaluates `c` (a scalar Boolean) and exactly one branch. Both branches have the same result type. The guarded form `? { predicate : expression; ... _ : expression; }` evaluates predicates in order and selects the first true predicate. The final `_` branch is obligatory. To avoid ambiguity, an unparenthesised pure block cannot be the first operand of the ternary `?`; wrap it in parentheses. A Boolean array is not a valid conditional predicate; use the reductions `&&/` and `||/` or filtering `?:`.

### 4.4 Pattern matching

```text
v(x:?q):q := ?= x {
    ?0   : 0;
    ?1 y : y;
};
sz(t:tree 'a):n := ?= t {
    lf _   : 1;
    br l r : + sz l sz r;
};
```

`?=` takes a scrutinee followed by a brace-delimited sequence of cases. Each pattern is followed by `:` and an expression terminated by `;`. Patterns include `_`, variables, scalar literals, tuples, exact-length sequence literals, and declared constructors with their fixed number of subpatterns. Constructor names are resolved in the nearest visible value environment before new pattern variables are introduced, so lowercase syntax is sufficient. A visible constructor is interpreted as a constructor pattern; other unqualified identifiers introduce variables. A qualified constructor is permitted where its module exports it. Variables in a pattern are fresh bindings and may not repeat within the same pattern. Branches must unify to one type. Pattern alternatives are tried in source order, with only the selected branch evaluated. The checker requires exhaustiveness and warns on unreachable alternatives; an explicit wildcard is necessary if coverage cannot be established. Repeated positional equations at the function declaration level are intentionally replaced by this expression form.

## 5. Arithmetic, logic and equality

The numeric model privileges exactness. A binary scalar arithmetic operation accepts compatible numerical types: `n` promotes to `z` when mixed with `z`, integral types promote to `q` or `d` as required, and mixing `q` with `d` yields `q`. Approximate `r` values are not implicitly mixed with exact values (contextual literals may adopt `r`). An explicit numeric conversion has the prefix form `:q x`, with `q` replaced by any of `n`, `z`, `q`, `d`, `r`. The target is read in the type namespace, and the form consumes exactly one operand. Conversion is not a named callable; `&q` does not refer to a conversion. To pass a conversion as a function, write `\(x)-> :q x`. Invalid or lossy exact conversions report domain errors. Converting `q` to `d` is exact only for a terminating decimal. Converting to `r` rounds to binary64; converting a finite `r` to `q` represents its actual binary value exactly.

For example, `n:n := 12; x:q := :q n; y:r := :r x;` is legal: the value `n` is distinct from the natural-number type. `:q xs` converts the elements of a numerical sequence, preserving its shape. Conversion does not introduce a general cast syntax for nonnumeric types.

| Operator | Arity | Semantics |
|---|---:|---|
| `+`, `-`, `*` | 2 | Addition, subtraction, multiplication |
| `/` | 2 | Exact quotient for exact types; `r` quotient for `r` |
| `//`, `%` | 2 | Integral floor quotient and corresponding remainder |
| `^` | 2 | Power with a natural exponent |
| `-:` | 1 | Arithmetic negation (`n` yields `z`) |
| `^/` | 1 | Square root, returns `r` |
| `\|` | 1 | Absolute magnitude |
| `<`, `<=`, `>`, `>=` | 2 | Ordered comparison |
| `=`, `/=` | 2 | Elementwise equality / inequality |
| `==` | 2 | Whole-value structural equality |
| `~=` | 3 | Absolute-tolerance equality: tolerance, left, right |
| `~` | 1 | Boolean negation |
| `&&`, `\|\|` | 2 | Boolean conjunction / disjunction |

`-` on two naturals is checked and yields `n` only if the result is nonnegative. Division by zero, invalid square roots, nonfinite floating-point results, integer underflow and nonconforming array shapes are domain errors, never silent values. Unchecked domain errors terminate the current evaluation with a diagnostic; the optional-producing statistical functions instead return `?0` in their specified undefined cases. A later checked-arithmetic library may provide total alternatives. `|` preserves the scalar numerical type; `^` uses a natural exponent (including `0`); the multiplicative identity is used for exponent zero. `//` is floor division, `% a b` equals `a - b * floor(a/b)`, and both require integral operands. `~=` requires a nonnegative tolerance, with explicit operand promotion; it is equivalent to the mathematical inequality `abs(x-y) <= tolerance` (mathematical notation, not source), computed without natural underflow.

`=` and `/=` apply the ordinary equality predicate at each element after broadcasting. `==` compares entire same-typed values recursively, including array lengths, table rows and order, record fields, and dictionary key-value associations irrespective of insertion order. It always returns a scalar `b` and never broadcasts. Functions and actions do not support equality. Finite real equality treats signed zero as equal. `&&` and `||` evaluate both operands and lift elementwise; use `?` for short-circuiting computations with domain restrictions.

```text
= [1 2 3] [1 4 3];      -- [1b 0b 1b]
== [1 2] [1 2 3];       -- 0b
~= 0.01 1.25 1.251;     -- 1b
&&/ > [1 2 3] 0;        -- 1b
```

## 6. Sequences and algebraic operators

A sequence is an immutable, ordered collection. `[1 2 (+ 1 2)]` contains three expressions, evaluated independently. Commas are not used between sequence elements; use parentheses for tuples. Empty `[]` has an inferred element type, which must be established by its context or annotation. Indices are one-based. Range `.. lo hi` includes both bounds; when `lo > hi` the range is empty. Bounds must have the same integral type (or an exact integral common type). Range representation may be lazy or compact, but observable semantics are those of an ordered finite sequence.

### 6.1 Lifting and shape

The listed arithmetic, comparison and Boolean scalar operators lift over one-dimensional sequences. For two sequences, lengths must agree; a scalar is repeated to match the sequence operand, including an empty sequence. Three-operand approximate equality applies the same rule to all sequence arguments. Nested sequences are values; implicit recursive tensor broadcasting, rank extension and singleton-array broadcasting are not included in 0.2. Selectors lift separately over sequences of records/tuples. `==` and collection operators are excluded from implicit scalar lifting. Numeric conversions also lift elementwise. The checker resolves the scalar element type before checking lifting, then inserts shape checks where lengths are not known.

```text
+ [1 2 3] [4 5 6];      -- [5 7 9]
* 3 .. 1 4;             -- [3 6 9 12]
+/ (^ (.. 1 5) 2);          -- 55
p(n,r:n):n := ? {
    > r n : 0;
    _     : */ (.. (+ (- n r) 1) n);
};
```

The permutation example is safe because the guard prevents natural underflow; `p 5 0` forms the empty range `6..5`, whose product is 1. An out-of-range choice (`r>n`) returns 0 by explicit convention. There is no implicit truncation of unequal sequences.

### 6.2 Collection operators

| Operator | Arity | Operand order and result |
|---|---:|---|
| `#` | 1 | Collection; cardinality `n` (string: scalar count) |
| `..` | 2 | Lower bound, upper bound; inclusive sequence |
| `@:` | 2 | Unary function, collection; map |
| `?:` | 2 | Predicate or Boolean mask, collection; filter |
| `=:` | 2 | Key function, collection; dictionary of groups |
| `>:` | 2 | Key function, collection; stable ascending sort |
| `<:` | 2 | Key function, collection; stable descending sort |
| `+:` | 2 | Left, right; concatenate compatible collections |
| `&:` | 2 | Two equal-length sequences; sequence of pairs |
| `*:` | 2 | Left, right; Cartesian product |
| `=.` | 1 | Sequence; stable distinct values |
| `.:` | 2 | One-based index, sequence/table, or key, dictionary; optional value |
| `[:` | 2 | Count, sequence/table; take first count |
| `/:` | 3 | Curried binary function, seed, sequence; left fold |
| `\:` | 3 | Curried binary function, seed, sequence; scan with seed |

`@:` maps a sequence to a sequence, a table to a sequence of results (even if those results are records), and a dictionary to a dictionary retaining the original keys. Table reconstruction is explicit with `#:`. Filtering preserves the collection kind for sequences and tables, preserves input order and requires masks of the same length. Dictionary grouping uses hashable equality keys; each group preserves the source collection kind and order, and keys are enumerated by first occurrence. Sorting uses total orderable keys and is stable. `+:` joins sequences or tables with identical row types; it is not arithmetic. `*:` over sequences returns pairs; table cross products produce tables of pair rows. `&:` requires equal lengths. `.:` yields `?0` for an out-of-bounds index, including zero, or an absent dictionary key. `[:` clips at the length. The fold calls its function as `@ @ f accumulator element`; scan emits the seed and all successive accumulator values.

```text
@: &(+) [1 2];         -- invalid: references cannot be grouped
@: &sq [1 2 3];        -- if sq is declared, [1 4 9]
/: &+ 0 [1 2 3];       -- 6
\: &+ 0 [1 2 3];       -- [0 1 3 6]
?: > [1 2 3] 1 [1 2 3]; -- [2 3]
.: 2 [7 8 9];          -- ?1 8
```

The operator families are mnemonic: a trailing `:` generally marks collection transformations, a leading `:` introduces numeric conversion in expression position, `/` marks elementary reduction, `.` marks projection or access, and `?` marks choice or partiality. These are naming conventions, not automatic operator generators. Only the registered spellings are legal in 0.2.

## 7. Reductions and statistics

Elementary reductions are one-token operators with fixed arity. Statistical summaries use short, ordinary prelude callable names, which are resolved by the same rules as other named functions. Their signatures may be overloaded for scalar element types without changing syntactic arity. They operate on a one-dimensional sequence of compatible scalar elements, not directly on a table. Project a table column first. An empty sum yields the additive identity, an empty product the multiplicative identity, and an empty logical reduction its mathematical identity. A standalone empty sequence needs an element-type annotation even when an identity is available.

| Operator | Arity | Result and undefined cases |
|---|---:|---|
| `+/` | 1 | Sum; empty identity 0 |
| `*/` | 1 | Product; empty identity 1 |
| `&&/` | 1 | All true; empty identity `1b` |
| `\|\|/` | 1 | Any true; empty identity `0b` |
| `>/`, `</` | 1 | Maximum/minimum, optional; empty `?0` |
| `avg` | 1 | Arithmetic mean, optional; empty `?0` |
| `pct` | 2 | Percentile (0-100), sequence; optional |
| `sd` | 1 | Population standard deviation, `?r` |
| `cor` | 2 | Pearson correlation, `?r` |

For exact inputs, mean and percentile return `?q`; for `r` inputs they return `?r`. Percentile uses linear interpolation between sorted observations at position `h=(len-1)*p/100` with zero-based mathematical positions (equivalent to the conventional inclusive linear quantile); `p=50` is the median. The percentile operand must be within 0 to 100, otherwise a domain error is reported. Empty input yields `?0`; a singleton yields its value. Population standard deviation divides by the population count; it yields `?0` on empty input and `?1 0r` on a singleton. Correlation returns `?0` for fewer than two observations or zero variance; lengths must match. Exact input may be converted internally to `r` for standard deviation and correlation, with a diagnostic on unrepresentable magnitude. Optional values are never silently removed before aggregation. Floating-point reductions use a specified left-to-right reference order; a compiler may fuse operations without changing observable results. More stable approximate algorithms can be named library operations.

```text
+/ (.. 1 100);            -- 5050
*/ (.. 1 5);              -- 120
avg [1 2 3 4];          -- ?1 5/2 (displayed rational)
pct 50 [1 2 3 4];        -- ?1 5/2
sd [2 2 2];             -- ?1 0r
cor [1 2 3] [2 4 6];     -- approximately ?1 1r
```

A rational-only implementation of the mean illustrates the ordinary functional core (the prelude's overloaded `avg` also accepts the other numerical types described above):

```text
av(x:[q]):?q := ? {
    = # x 0 : ?0;
    _       : ?1 (/ (+/ x) (:q # x));
};
```

Composition needs no dedicated punctuation. It can be defined as an ordinary library helper (not an additional reserved or prelude name):

```text
o(f:'b->'c,g:'a->'b):'a->'c :=
    \(x)-> @ f (@ g x);
```

For example, `@: (o &avg &.price) g` computes mean prices over groups of a known table schema when `o` is in scope; the context resolves the overloaded callable reference. The explicit version `@: (\(x)-> avg (.price x)) g` is equivalent. A reference such as `&avg` has the curried signature of the resolved overload and requires enough context to resolve it.

The notation `5/2` in a comment is a mathematical display, not an infix source literal; source uses `/ 5 2`. General folds remain available via `/:`. User-defined punctuation reductions are deferred, since dynamically registering spellings or arities complicates lexing and separate compilation.

## 8. Absence, errors and effects

`?a` denotes an optional value. `?0` is a polymorphic nullary constructor; `?1` is a unary constructor. No universal null exists. This section deliberately uses signs instead of alphabetic constructor names.

| Symbol | Arity | Meaning |
|---|---:|---|
| `?0` | 0 | Absent value |
| `?1` | 1 | Present value |
| `?+` | 1 | Presence test, returns `b` |
| `??` | 2 | Default expression, optional value; lazy default |
| `?!` | 1 | Explicit checked unwrap; absence is a domain error |
| `?=` | special | Pattern match |
| `!` | special | Execute action within effect block / REPL |
| `!{...}` | special | Suspend an effectful block |
| `<@` | 2 | Import action: path, worksheet |
| `@>` | 3 | Export action: path, worksheet, data |
| `!>` | 1 | Output action: value |

```text
x:?q := ?0;
y := ?? 0 x;             -- 0q
u := ?1 12q;
?+ u;                    -- 1b
?! u;                    -- 12q
```

Optional-specific operators act on a single optional value and are not implicitly lifted. Mapping over nullable columns is explicit (`@: &?+ .2 t`). `??` evaluates the optional operand first, then the fallback only if absent. Unwrapping via `?!` is intentionally visible; pattern matching is preferred where absence requires distinct behaviour. Built-in statistical optionals distinguish an undefined statistic from an operational failure. Domain errors and failed I/O carry source-located diagnostics; 0.2 does not yet prescribe a catch/recovery syntax for arbitrary operational failures. Recoverable application failures may be represented by ordinary algebraic sum types.

### 8.1 Pure and effectful contexts

An action of type `!a` describes an operation without executing it. It may be stored, passed to functions or returned from a pure function. `!{ ... }` constructs a suspended action. Within it, `! e` runs an action and yields its result; local bindings can depend on those results. Unlike a pure block, an effect block permits intermediate expression statements terminated by semicolons, evaluated in order (discarding their results). Its final expression, without a semicolon, supplies the action's result. A top-level `main:!()` binding is the executable entry point; the runtime runs its action. A REPL additionally permits `! e` at the top level. `!` cannot occur as an executing expression in a pure function body or pure block, except inside a suspended effect block.

```text
main:!() := !{
    t:#[s q q n] := ! <@ "houses.xlsx" "sales";
    ! !> # t;
    ! @> "summary.xlsx" "raw" t
};
```

Here `! !>` executes an output action; `!>` itself is a single lexed callable. `<@` and `@>` are ordinary effect-producing operations, not executing constructs. The import's expected table schema determines the conversion from cells; an explicit schema is required when not otherwise inferable. The workbook format is determined from its file extension. The import/export interface initially targets Excel, with other file formats and connector adapters deferred. Output writes must report failures; precise overwrite policy and workbook update semantics belong to the I/O library contract and are not defined by the syntax.

`<-` is reserved for possible future assignment to a mutable reference. It has no executable semantics in 0.2. Mutation, exceptions and general effect polymorphism require their own design. The distinction remains strict: `:=` introduces an immutable quantity, `=` compares elementwise and `==` compares entire structures.

## 9. Tables, dictionaries and relational algebra

A table is an ordered collection of rows with a fixed tuple or record schema. It preserves duplicate rows. Fields are homogeneous typed columns; a nullable column explicitly uses `?a`. Positional schemas are particularly economical for ad hoc work; named schemas are preferable for durable datasets. Dictionary keys must be hashable and comparable for equality; dictionary literal keys are unique, and duplicates are errors. Dictionaries enumerate in insertion order, but equality ignores order. Column access does not require string-based indexing.

```text
h :: { city:s; price:q; area:q; beds:n; };
t:#[s q q n] := #[
    ("Makati",12500000,120,3)
    ("Pasig",8400000,95,2)
    ("Makati",18000000,150,4)
    ("Quezon City",6500000,85,2)
    ("Pasig",9200000,110,3)
];
p := .2 t;
a := .3 t;
g := =: &.1 t;
m := @: (\(x)-> avg (.2 x)) g;
```

`#[...]` constructs a table from compatible row expressions, each a tuple or record. `#{ key : value; ... }` constructs a dictionary. Empty tables and dictionaries require a contextual type. `#h` denotes a table of records of type `h`, and `.price t` would project prices from such a table. `#:` converts a sequence of compatible rows to a table; `:#` converts a table to its ordered row sequence. Table literals and selectors preserve schema information and the checker rejects impossible field or tuple positions. Dictionary projections are not implicit; use a map or lookup (`.: key dict`, returns an optional value). For a dictionary `.:` is key access rather than positional access; the arity remains two.

### 9.1 Relational operators

| Operator | Arity | Meaning |
|---|---:|---|
| `#:` | 1 | Construct table from row sequence |
| `:#` | 1 | Extract ordered sequence of rows |
| `=:` | 2 | Group by unary key function |
| `?:` | 2 | Filter by predicate or Boolean column |
| `>:` / `<:` | 2 | Stable ascending / descending sort by key |
| `+:` | 2 | Concatenate same-schema tables |
| `*:` | 2 | Cartesian product of tables |
| `*=` | 4 | Inner equijoin: left key, right key, left, right |
| `*?` | 4 | Left equijoin, same argument order |

An inner join returns a table whose rows are `(leftrow,rightrow)` pairs. A left join returns `(leftrow,?rightrow)` pairs, with absent right rows represented by `?0`. The keys must have a common equality/hash type. Joins do not flatten schemas or infer a natural join from coincident field names; duplicate matches produce all pairs. Order is left input order, then matching right input order. Missing key values are ordinary optionals if explicitly used as keys: `?0` equals `?0` structurally, not SQL null. To obtain SQL-style nonmatching nulls, filter absent keys before joining. Database backends must preserve this semantic distinction. Projection and aggregation remain ordinary expressions rather than a special query language.

```text
j := *= &.1 &.1 t u;
g := =: &.1 t;
m := @: (\(x)-> avg (.2 x)) g;
s := [: 10 <: &.2 t;
```

`u` is an illustrative second table with a compatible first-column key. The first expression joins, the second groups, the third computes mean price by group, and the last takes the ten most expensive rows. `>:` and `<:` preserve their previously proposed ascending and descending meanings.

### 9.2 Missing and malformed cells

A positional import with `#[s q q n]` rejects a blank or malformed mandatory cell. A schema such as `#[s ?q ?q n]` admits absent price and area cells but still rejects nonblank malformed numbers. A first worksheet row is interpreted as a header by the initial Excel adapter; the positional schema describes the data columns in worksheet order. Named record schemas match headers exactly. Details of format-specific coercions, dates, duplicate headers and Excel formula evaluation require a separate adapter specification. There is no silent default to zero and no implicit deletion of incomplete records.

```text
t:#[s ?q ?q n] := ! <@ "houses.xlsx" "sales";
v := ?: (&& (@: &?+ .2 t) (@: &?+ .3 t)) t;
p := @: &?! .2 v;
a := @: &?! .3 v;
w := && > p 0 > a 0;
p1 := ?: w p;
a1 := ?: w a;
avg (/ p1 a1);
```

This REPL specimen removes missing values first, explicitly unwraps the remaining columns, then removes nonpositive prices and areas before computing mean price per square metre. It preserves row alignment and never converts a missing observation to a numerical zero.

## 10. Modules and source structure

A file is a module. Imports and explicit exports are declarations at the outermost level and have no executable effect. Modules are resolved before expression parsing, since imported names must carry arity information. An imported name is accessed using a qualified identifier; no implicit wildcard import is provided.

```text
<< "statistics" : st;
>> (p,m);

p(n,r:n):n := ? {
    > r n : 0;
    _     : */ (.. (+ (- n r) 1) n);
};
m(x:[q]):?q := st.mean x; -- illustrative exported library callable
```

`<< "module" : alias;` introduces the module alias; `>> (name,name,...);` declares the exported names. Both type and value entries for an exported spelling are exported if both exist. A module interface includes kind (value, callable, constructor, type), syntactic arity, type signature and record or sum metadata required for parsing. Public values require explicit types, public functions fully explicit signatures, and public type definitions are exported in full in the first version. Names not listed in exports are private. Circular module imports are disallowed; recursive functions within a module are permitted. Module path resolution, package registries and naming of files are deferred. `>>` here denotes module export only, superseding its earlier proposed use as an effect sequencing operator.

A source module contains declarations only. The interactive environment permits semicolon-terminated expressions as well as declarations and prints expression results; a script's entry point is `main:!()`. Program source never silently performs effects through ordinary variable initialisation. Top-level immutable definitions are initialised according to their dependencies; a cyclic dependency between values is an error. Function bodies execute only upon application.

## 11. Formal surface grammar

The following EBNF is the normative structural outline. Quoted symbols are terminals; braces in EBNF mean repetition, brackets mean optionality. `id`, `qid`, `tvar`, `literal`, `selector`, `pattern_literal` and operator spellings are lexical classes from section 2. Expression and type parsing use the environment of callable and type-constructor arities. This is an attribute grammar rather than a purely context-free grammar for arbitrary user-defined names. Delimiters are actual terminal characters when quoted.

```text
module       = { import | export | type_decl | def } ;
import       = '<<' string ':' id ';' ;
export       = '>>' '(' id { ',' id } ')' ';' ;
type_decl    = id { tvar } '::' type_rhs ';' ;
type_rhs     = type | '{' field { field } '}'
                   | '{' variant { variant } '}' ;
field        = id ':' type ';' ;
variant      = '|' id { type_atom } ';' ;
def          = id [ ':' type ] ':=' expr ';'
             | id '(' params ')' [ ':' type [ constraints ] ]
               ':=' expr ';'
             | id '(' names ')' ':=' expr ';' ;
params       = param_group { ',' param_group } ;
param_group  = names ':' type ;
names        = id { ',' id } ;
constraints  = '|' constraint { ',' constraint } ;
constraint   = id type_atom { type_atom } ;

expr         = literal | value_name | '(' ')'
             | '(' expr ')' | '(' expr ',' expr { ',' expr } ')'
             | '[' { expr } ']' | '#[' { expr } ']'
             | '#{' { expr ':' expr ';' } '}'
             | record_name '{' { id ':=' expr ';' } '}'
             | pure_block | lambda | reference | call | conversion
             | conditional | guarded | match | effect_block
             | execution ;
pure_block   = '{' { def } expr '}' ;
lambda       = '\' '(' ( names | params ) ')' '->' expr ;
reference    = '&' callable_name ;
conversion   = ':' numeric_type expr ;
numeric_type = 'n' | 'z' | 'q' | 'd' | 'r' ;
call         = callable_name expr^arity(callable_name) ;
conditional  = '?' expr expr expr ;
guarded      = '?' '{' { expr ':' expr ';' }
               '_' ':' expr ';' '}' ;
match        = '?=' expr '{' case { case } '}' ;
case         = pattern ':' expr ';' ;
effect_block = '!' '{' { def | expr ';' } expr '}' ;
execution    = '!' expr ;
pattern      = '_' | id_binding | pattern_literal | '(' ')'
             | '(' pattern ',' pattern { ',' pattern } ')'
             | '(' pattern ')' | '[' { pattern } ']'
             | constructor pattern^arity(constructor) ;

type         = type_atom { type_atom } '->' type
             | type_atom ;
type_atom    = primitive | tvar | '(' ')'
             | type_name type_atom^arity(type_name)
             | '(' type ')' | '(' type ',' type { ',' type } ')'
             | '[' type ']' | '{' type ':' type '}'
             | '?' type_atom | '!' type_atom | '#' type_atom
             | '#[' type_atom type_atom { type_atom } ']' ;
```

### 11.1 Disambiguation rules

`expr^k` means exactly `k` recursively parsed expressions, not an exponent token in the language. Likewise, a parameterised type constructor consumes its registered number of type atoms. Function types used as a single variant field or type argument must be parenthesised; `tree 'a` is one type atom, and `(tree 'a)` is equivalent. A `#[...]` table schema contains at least two positional field type atoms (parenthesise arrow types); one-field tables use a named one-field record. A table row type after `#` must be a tuple or record. Parameter groups are collected up to the next colon; a comma after a complete type begins the next group. A parameter list is either wholly annotated or wholly unannotated. For a wholly annotated private parameter list, the result annotation may be omitted. The result type and constraints then follow from inference; an exported signature remains complete. A grouped type annotation consumes one type expression, ending at the header delimiter, comma, constraint bar or `:=`.

At expression position, `?` followed by `{` selects guarded syntax, and `!` followed by `{` selects an effect block; to execute an action produced by a pure block, group that block with parentheses. `&` accepts a known referenceable callable; lexical operators that begin with `&` (e.g. `&:` or `&&`) are independently tokenised. `?0` is a nullary constructor and `?1` a unary constructor. At expression position `:` followed by a primitive numeric type introduces one-operand conversion. In patterns the nearest visible constructor binding determines constructor syntax; other unqualified identifiers bind variables. A visible constructor name is not a variable pattern, but a constructor hidden by an ordinary outer value is no longer visible under that unqualified name. `id ':' type ':='` and `id ':='` begin local definitions; a header `id '(' ... ')'` followed by `:` or `:=` is also a definition. Other starts lead to expressions. Record constructors are recognised by the nearest visible record-constructor binding followed by `{`, and cannot be applied as ordinary functions. The lexer distinguishes composite openers `#[` and `#{` from standalone `#` followed by a delimiter.

The declaration prepass discovers names and signatures by scanning balanced token delimiters and semicolon-terminated declaration boundaries, without first parsing callable bodies. It classifies function headers by parameter counts and sum constructors by type fields. Once each scope has an arity environment, the parser recursively consumes the exact number of operands per callable. It records each ordinary local value at its declaration boundary, and installs parameter and branch-variable bindings before parsing their corresponding bodies; each such value has zero syntactic operands. Nested scopes repeat this procedure with the nearest visible binding taking precedence. Module-level values are known from the prepass and may depend on one another acyclically; ordinary local values are visible only after their definition. Forward references to named callables are legal; their bodies use the lexical environment at the declaration site, as specified in section 4.2. A variable's inferred function type does not retroactively change its syntactic arity. This guarantees stable parsing when a function's inferred types change.

## 12. Conformance specimens

These are reference examples for a future lexer, parser, type checker and interpreter. Results are written as comments; standalone expressions are REPL inputs. Each specimen has its own scope.

### 12.1 Elementary mathematics

```text
f(x:n):n := */ (.. 1 x);
p(n,r:n):n := ? {
    > r n : 0;
    _     : */ (.. (+ (- n r) 1) n);
};
c(n,r:n):n := ? (> r n) 0 (// (p n r) (f r));

f 0;       -- 1
p 5 3;     -- 60
p 5 0;     -- 1
p 3 5;     -- 0
c 5 3;     -- 10
+/ (^ (.. 1 5) 2); -- 55
```

### 12.2 Functional abstraction and recursion

```text
s(x:q):q := ^ x 2;
f2(f:'a->'a,x:'a):'a := @ f (@ f x);
f2 &s 2q;                    -- 16q

f(x:n):n := ? (< x 2) x (+ (f (- x 1)) (f (- x 2)));
f 10;                       -- 55

m(x:[q]):?q := avg x;
m [1q 2q 3q];               -- ?1 2q
m [];                       -- ?0
```

### 12.3 Numerical analysis with safe domains

```text
v(x:[q]):?q := ?= (avg x) {
    ?0   : ?0;
    ?1 m : ?1 (/ (+/ (^ (- x m) 2)) (:q # x));
};
dp(x,y:[q]):q := +/ (* x y);

v [1q 2q 3q];               -- ?1 (2/3)
v [];                       -- ?0
dp [1q 2q] [3q 4q];          -- 11q
```

### 12.4 Excel house-price analysis

The following is a complete interactive session for a workbook whose first four columns are city, price, floor area and bedroom count. It uses exact rational inputs. The schema is nonnullable; the alternative for nullable cells appears in section 9.2.

```text
t:#[s q q n] := ! <@ "houses.xlsx" "sales";
u := ?: (&& (> .2 t 0) (> .3 t 0)) t;
p := .2 u;
a := .3 u;

# u;                        -- property count
+/ p;                       -- total value
avg p;                      -- mean price
pct 50 p;                    -- median price
pct 90 p;                    -- 90th percentile
sd p;                       -- population standard deviation
cor p a;                     -- price-area correlation
pct 50 (/ p a);             -- median price per square metre
?: < p 10000000 u;          -- affordable properties
[: 10 <: &.2 u;             -- top ten by price

g := =: &.1 u;
s := @: (\(x)-> avg (.2 x)) g;
! @> "summary.xlsx" "cities" s;
```

For the five illustrative rows in section 9, the count is 5, total 54,600,000, mean 10,920,000, median 9,200,000 and 90th percentile 15,800,000. City means are Makati 15,250,000, Pasig 8,800,000 and Quezon City 6,500,000. The export accepts a dictionary as two columns, with optional values encoded as blank cells. Actual workbook import/export behaviour remains to be implemented and tested.

### 12.5 Maintained spreadsheet script

For a durable script, named schemas and short reusable functions replace positional column knowledge. This module assumes the worksheet headers are exactly `city`, `price`, `area`, `beds` (otherwise the adapter must reject the mismatch; a header mapping facility is deferred). The positional interactive example remains appropriate for ad hoc use. The dictionary result has type `{s:?q}`.

```text
h :: {
    city:s;
    price:q;
    area:q;
    beds:n;
};

v(x:h):b :=
    && (> .price x 0) (> .area x 0);

m(t:#h):?q := avg (.price t);

main:!() := !{
    t:#h := ! <@ "houses.xlsx" "sales";
    u := ?: &v t;
    g := =: &.city u;
    s := @: &m g;

    ! @> "summary.xlsx" "cities" s
};
```

The effect form, explicit action execution, table schema and export contract are unchanged. `main` is a private runtime entry binding, not an exported function. The same group means as in section 12.4 are expected when applied to the same five valid rows.

### 12.6 Scope, conversions and inferred results

```text
n:n := 12;
r:q := :q n;
q:r := :r r;
sq(x:q) := * x x;
dp(x,y:[q]) := +/ (* x y);
f(x:n):n := + x 1;
g(f:n->n,x:n):n := @ f (@ f x);

r;                         -- 12q
q;                         -- 12r
sq 3q;                     -- 9q
dp [1q 2q] [3q 4q];         -- 11q
g &f 2;                    -- 4
f 2;                       -- 3
#{ "x" : :q 3; };           -- dictionary {s:q}
```

The names `n`, `r` and `q` are legal value bindings, despite the equally spelled primitive types. The inferred result types of `sq` and `dp` are both `q`. The parameter `f` in `g` hides the enclosing direct callable only within that body; `&f` at the call site still references the outer function. The spaced colons in the dictionary preserve token boundaries.

### 12.7 Negative cases

| Source | Required rejection or outcome |
|---|---|
| `1 + 2;` | Infix expression is invalid |
| `+ 1;` | Missing operand |
| `+ 1 2 3;` | Surplus operand |
| `x = 3;` | Not a definition; invalid expression at declaration site |
| `f(x,y:q):q := + x y; f(3,4);` | A tuple is one operand; the second operand is missing (and the first is ill-typed) |
| `+ [1 2] [3];` | Shape mismatch; no singleton-array broadcast |
| `? [1b 0b] 1 0;` | Predicate must be scalar |
| `- 1n 2n;` | Natural underflow domain error |
| `+/ [] ;` | Ambiguous empty element type unless contextual |
| `?! ?0;` | Explicit unwrap of absence is a domain error (when type known) |
| `.5 (1,2);` | Selector outside tuple schema |
| `x := ! <@ "a.xlsx" "s";` | Execution outside action block in a source module |
| `x := 1; x := 2;` | Duplicate binding in one scope |
| `x <- 2;` | Reserved mutation syntax; unsupported in 0.2 |
| `:s 1;` | Conversion target must be a primitive numeric type |
| `&q;` (with no callable `q`) | A conversion is not a referenceable named callable |
| `#{ "x" ::q 3; };` | `::` is a type-definition token, not two colons |
| `f(x:q,y) := + x y;` | Mixed typed and untyped parameters unsupported |
| `>> (f); f(x:q) := * x x;` | Exported function requires a complete signature |
| `+// [1 2];` | Retired statistic spelling; use `avg` |

## 13. Consolidated design decisions and implementation notes

The established preferences are preserved: ASCII, strict prefix expressions, lowercase names, typed functions, a single declaration header, parentheses/braces/semicolons, immutable bindings, and array-oriented reduction and tabular analysis. The following changes distinguish Draft 0.2 from Draft 0.1: numeric conversions now use `:n`, `:z`, `:q`, `:d`, `:r`; primitive type spellings are free in the value namespace; nearest-binding lexical shadowing supersedes the callable-shadowing prohibition; private fully parameter-annotated functions may infer their result; `avg`, `pct`, `sd`, `cor` supersede `+//`, `?/`, `~/`, `~:`; grouping and aligned cases have canonical formatting guidance; a maintained, named-schema spreadsheet script complements the positional REPL specimen. Explicit application `@`, references `&`, effects `!`, type and definition signs, fixed arity and the existing data and numerical contracts remain. The composition helper `o` is an illustrative ordinary library definition, not new syntax. These are proposed normative decisions for this revision, not claims of an implementation.

The earlier baseline also made several decisions necessary for an implementable grammar. Explicit type variables (`'a`) avoid collisions with single-letter primitive types; `0b` and `1b` avoid reserved Boolean words; optional constructors become `?0` and `?1`. Guarded and pattern expressions replace repeated equation heads. One-based selectors and indices, a longest-match lexer, explicit callable references, and a declaration prepass make the arity-dependent grammar precise. Optional-valued summaries, finite binary64 numbers, exact decimal/rational promotion, checked natural subtraction and explicit missing-data handling establish conservative data semantics. The module syntax, join operators, effect blocks and symbolic I/O proposed in Draft 0.1 are retained without redesign. General-purpose named library functions remain legal, and the symbolic core is finite rather than infinitely extensible.

The recommended implementation sequence is (1) lexer and declaration scanner, (2) arity-aware prefix parser and AST, (3) lexical resolution and typed intermediate representation, (4) type inference and overload constraints, (5) pure interpreter and REPL, (6) sequence/table runtime and statistical library, (7) Excel adapter, and (8) module interfaces and optimisation. A conformance suite should test positive and negative syntax, lexical maximal munch, scope/arity resolution, optional propagation, shape errors, exact arithmetic, and sheet round-trips before any compiler backend is attempted. Fusion of ranges, maps, masks and reductions should be introduced only after a reference evaluator fixes semantics.

Open matters for a later design revision: language name and extension; exact type-class implementation and row/shape polymorphism; total arithmetic and recoverable operational failures; effect polymorphism and mutable references; higher-dimensional arrays; date/time and categorical values; Excel header/formula/overwrite policies; package resolution and database backends; a canonical formatter implementation for larger modules; and the extent of user-defined symbolic operators. The specification deliberately leaves these open rather than embedding premature answers in the grammar. No parser or runtime has yet been implemented or executed against these examples.
