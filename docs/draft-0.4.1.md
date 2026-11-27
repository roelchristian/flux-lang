# Flux Language Specification: Draft 0.4.1

This draft extends [Draft 0.4](draft-0.4.md) with typed recoverable failure. It retains strict prefix notation, static inference, immutable bindings, exact arithmetic and explicit effects. It does not introduce unrestricted exceptions or implicit execution.

## Types and signs

| Form | Meaning |
|---|---|
| `?a` | Optional value, including ordinary absence |
| `a^e` | Result: successful value of type `a` or recoverable error of type `e` |
| `!a` | Suspended action returning `a` when executed |
| Fatal diagnostic | Terminates evaluation; not a value and not caught by recovery |

The result type constructor is distinct from the optional, action and function constructors. `?q^e` means `(?q)^e`; `!q^e` means `(!q)^e`. Use `!(q^e)` for an action returning a result. `[q^e]` holds independently fallible elements, while `[q]^e` is one fallible computation yielding a sequence. Parenthesize nested results: unparenthesized `a^e^f` is invalid. A function's result and error types are part of its ordinary public signature; there is no inferred hidden throws clause.

| Surface form | Operands and purpose |
|---|---|
| `^{ … }` | Delimited fallible boundary with statements and a final expression |
| `^: e` | One error payload; exit the nearest fallible boundary |
| `^? r` | One result; return its success or exit the nearest fallible boundary with its error |
| `^= r { … }` | One result and delimited recovery arms |
| `^ x y` and `^/ x` | Unchanged exponentiation and square root |

Lex compound signs by maximal munch. Fixed expression arity is decided before checking. `^{ … }` does not start an action or grant effect permissions.

## Boundaries, raising and propagation

A fallible boundary evaluates its statements in order, then wraps the value of its final expression in success. It does not flatten a final expression that is itself a result. `^: e` evaluates its payload once; `^? r` evaluates its operand once and extracts exactly one result layer. Both failure paths skip the remainder of their target boundary and return its typed failure. The success and error types of the boundary are inferred or checked against context. All raise and propagation sites targeting the same boundary must agree on one error type. Compatibility is nominal where the error type is nominal; conversion between error types must be explicit.

Targets are lexical. A nested `^{ … }` owns raises and propagation inside it. A named function, lambda or independently suspended action begins a fresh propagation context and may establish its own boundary. None can target a surrounding boundary solely because its text occurs there. Ordinary nested pure blocks, recovery arms and pipeline stages do not by themselves establish a boundary. Failure leaves completed effects intact; there is no rollback. A discarded result is a static error, rather than an implicit ignore or an implicit propagation. A pipeline must use its result-valued previous stage (for example, with `^? $` inside a boundary) rather than silently replace it.

```flux
diverr :: { | zero; };
div(x,y:q):q^diverr := ^{
    ?{ = y 0 : ^: zero; _ : / x y; }
};
a := div 12 3;
b := div 12 0;
```

The zero path returns a result containing `zero`. It does not evaluate the ordinary division. The success path returns a result containing `4`.

## Nominal errors and recovery

```flux
loaderr :: { | missing; | changed s; };
```

Nominal alternatives carry zero or more declared payload fields. A constructor consumes exactly its declared payload count; a nullary constructor needs no call marker. Alternative identity is nominal, not inferred from spelling. Constructor patterns bind the immediate payload fields in their declared order. Bindings are scoped to their arm. `_` discards a matched error, and an identifier catch-all binds the whole error. A catch-all is last; duplicate, unknown, wrong-arity, non-exhaustive and unreachable arms are rejected. For a non-nominal error type, use a catch-all.

On success, `^=` returns the success unchanged and evaluates no arm. On failure, it evaluates only the first matching arm; the arm's value must agree with the success type. An arm can recover to a normal value, raise a different error into an enclosing boundary, or propagate a result via `^?`. Raising again is not an implicit rethrow. Errors that are forwarded preserve their diagnostic origin; translating to a new error introduces a new raise site.

```flux
value := ^= div 12 0 { zero : 0q; };
```

## Effects and library boundaries

Importing a library does not perform I/O. An I/O action is inert until an
explicit, effect-legal `!` executes it. The caller must then decide whether
to recover, propagate or return its result. The following signatures show
curried operands; `a` is a value type and `row` is a supported named-field
schema.

| Operation | Result at the call site |
|---|---|
| `io.print value` (also `io.println`) | `!(()^ioerr)` |
| `io.read path` | `!(s^ioerr)` |
| `io.write path text` | `!(()^ioerr)` |
| `io.exists path` | `!(b^ioerr)` |
| `csv.read path` with no typed schema | `!([[s]]^csverr)` |
| `csv.read path` in a checked `#row` result context | `!(#row^csverr)` |
| `csv.write path table` | `!(()^csverr)` |
| `math.div x y` for exact numeric operands | `q^numerr` (pure) |

Paths and text are strings. `io.exists` reports successful false for a
missing path, successful true when metadata lookup succeeds, and an
`ioerr` for other access failures. It is not a permission check or proof
that a subsequent open will succeed. File reads accept regular files, not
special streams or directories, and are subject to UTF-8 decoding and size
limits. File writes create a fresh file and do not overwrite an existing one.
A file-operation failure does not imply rollback of completed actions.

`ioerr` alternatives `missing s`, `denied s`, `exists s`, `readerr s`,
`writeerr s`, and `encoding s` carry a path, or the output-stream label for
a print failure. The string is not the OS diagnostic. It denotes a missing
path, access denial, no-clobber conflict, other read failure, other write
failure, or invalid UTF-8, respectively.

| `csverr` alternative | Payload meaning |
|---|---|
| `fileerr ioerr` | Underlying file or encoding failure |
| `header s` | Header or schema validation detail (such as an unexpected column) |
| `duplicate s` | Duplicate header name |
| `absent s` | Required header name missing from input |
| `cellerr n s s` | Record number, field name, and decoded cell text before typed conversion |
| `rowerr n` | Malformed record number |

Read-side record numbers are one-based logical CSV records, including the
header as record 1 when present; a newline inside quotes does not advance
that number. Raw reads return strings and do not validate a typed header.
Typed reads first decode the full input and validate record structure and
header/schema correspondence before converting cells. Conversion then visits
records in source order and fields in declared schema order, even if input
columns were reordered. A correctly headed empty table can succeed. Blank optional cells represent absence, but a
malformed nonblank cell is a `cellerr`, even in an optional column.
`io.read_csv` and `io.write_csv` are aliases for the CSV operations.
Importing either `csv` or `io` makes the corresponding `csverr` and
`ioerr` types and constructors available unqualified. Prefixes or aliases
used to call module members do not qualify constructor patterns: write
`fileerr e`, not a module-member call. Structured origin and operation context
accompany errors
independently of their payload; handling the payload does not require
matching the source location or a platform-dependent diagnostic message.

An unchecked numerical domain violation (for example ordinary division by zero), failed forced optional extraction or incompatible runtime shapes remain fatal diagnostics. Checked operations intentionally convert documented domain or range failures into typed results. In `math`, `math.div` returns an exact rational or a `numerr` with nullary constructor `divzero` or `range`. Neither recovery nor propagation can intercept a fatal diagnostic. Collection maps do not implicitly propagate result elements: a map returning `[a^e]` visits each input independently and retains the success or error for each element.

## Typed execution and script exit

Checking resolves each operator, result type, boundary target and recovery alternative before lowering. HIR must distinguish the success and failure branches, carry the lexical exit target and use resolved constructor identities. The evaluator consumes those decisions rather than re-parsing patterns or inferring error types from runtime payloads. New error syntax has no compatibility fallback.

At script entry, an unhandled recoverable failure terminates with exit status 1 and a structured error on stderr (type, constructor, message and available source origin/context). Already completed stdout or other effects remain committed. Recovered errors are not also emitted as terminal diagnostics. A successful unit result is accepted as ordinary completion. This entry policy does not make an arbitrary expression statement a legal place to discard a result.

## End-to-end example

The loader translates the CSV error into its own type. A valid empty table
is a separate application-level error. The final `()` makes explicit that a
successful script result carries no value.

```flux
<< "csv";
obs :: { name:s; value:q; };
err :: { | source csverr; | empty; };

load(p:s):!(#obs^err) := !{
    ^{
        t:#obs := ^= ! csv.read p { e : ^: source e; };
        ?{ = (# t) 0 : ^: empty; _ : t; }
    }
};

!{
    ^{
        t := ^? ! load "input.csv";
        avg .value t;
        ()
    }
};
```

## Migration and boundaries of this draft

Executing a newly fallible action is no longer sufficient on its own:

```flux
<< "io";
!{
    ^{ ^? ! io.print "report"; () }
};
```

Here `^?` forwards a print failure to the script exit. To suppress it
intentionally, recover with `^= ! io.print "report" { _ : (); }` instead.
Move `!` inside the caret expression: raising or recovering an action
value does not execute it. The retired `!>` spelling is not restored.

Result mapping or folding, error mapping, sequencing a collection of
results, and optional/result adapters may be considered separately as
library combinators. They are not automatically supplied or implied by
ordinary map or pipeline operations in this draft. Implicit error unions,
resumable exceptions, retry/rollback semantics, and untyped exception
interoperability are outside the scope of this revision.
