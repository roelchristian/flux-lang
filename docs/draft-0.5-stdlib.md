# Flux Draft 0.5: Standard-library foundations

This revision extends the native library registry, not the source grammar.
Names retain fixed arity and pass through the same checker and typed HIR
as core operations. Read this alongside the specifications for
[typed errors](draft-0.4.1.md), [local modules](draft-0.4.3.md), and
[exact numbers](draft-0.5-numerics.md).

## Prelude and effects

`print VALUE` is the unqualified spelling of fallible `io.print` and has
the same type and semantics. Its result is a suspended `!(()^ioerr)`
action. Constructing or importing it does not print. Execute it explicitly
with `!` where effects are legal, and handle or propagate its result.

```flux
greeting := print "Salve, munde.";
!{
    ^{ ^? ! greeting; () }
};
```

The other namespaces remain explicit imports. The prelude makes
`ioerr` and its constructors available; `rx`, `dt` and `json` expose
their own error types and constructors when imported. Importing a library neither
runs its actions nor broadens purity rules. Ordinary runtime I/O paths are
relative to the process working directory unless an operation says
otherwise; the source module loader retains its independent file-relative
rules.

## Optional and result combinators

Import `opt` and `res` explicitly. The signatures below use type variables
for ordinary checked types; these are not dynamic operations. Callbacks are
function values, for example `&transform` or a lambda.

| Operation and operand order | Type |
|---|---|
| `opt.map F VALUE` | `(a->b) ?a -> ?b` |
| `opt.bind F VALUE` | `(a->?b) ?a -> ?b` |
| `opt.result ERROR VALUE` | `e ?a -> a^e` |
| `res.map F VALUE` | `(a->b) (a^e) -> b^e` |
| `res.maperr F VALUE` | `(e->f) (a^e) -> a^f` |
| `res.collect RESULTS` | `[a^e] -> [a]^e` |
| `res.partition RESULTS` | `[a^e] -> ([a],[e])` |
| `res.ok VALUE` | `(a^e) -> b` |
| `res.optional VALUE` | `(a^e) -> ?a` |

`opt.map` and `opt.bind` invoke the callback only on presence; bind
returns its optional without adding another layer. `opt.result` preserves
presence as success and turns absence into the supplied error with the
conversion site as origin. The error is an ordinary evaluated argument,
not a callback. `res.map` invokes only for success, and `res.maperr` invokes
only for failure. Mapping an error changes the payload but retains the
original diagnostic provenance; a different error type is permitted.

`res.collect` returns successes in order if all values succeeded, or the
first failure in sequence order including its provenance. The input is
a sequence of evaluated results, not a sequence of actions or promises:
this operation does not retroactively skip work that produced later elements.
`res.partition` preserves relative order within each projection and returns
plain payloads, not failure objects carrying provenance. `res.ok` tests
success; `res.optional` explicitly discards the failure payload and origin.
None of these operations executes a suspended action automatically.

## Streams, files and process information

| Operation | Type |
|---|---|
| `io.readln` | `!((?s)^ioerr)` |
| `fs.ls PATH` | `s -> !([s]^ioerr)` |
| `fs.mkdir PATH` | `s -> !(()^ioerr)` |
| `fs.exists PATH` | Same as `io.exists`: `s -> !(b^ioerr)` |
| `fs.base PATH` | `s -> ?s` |
| `fs.join LEFT RIGHT` | `s s -> s` |
| `sys.args` | `![s]` |
| `sys.pid` | `!n` |
| `sys.env NAME` | `s -> !((?s)^ioerr)` |

`io.readln` distinguishes EOF (absent) from a blank line (present empty
string). It removes a terminating LF and a preceding CR; a final CR without
LF is data. Invalid UTF-8 and oversized input are recoverable I/O errors,
not EOF. The reference line limit is 1 MiB; encountering it does not
promise to discard the remaining bytes of that line.

`fs.ls` returns sorted UTF-8 basenames, not full paths; a non-Unicode entry
produces an encoding failure. `mkdir` creates one directory, not missing
parent directories, and fails if its destination exists. `fs.base` and
`fs.join` use native lexical path components without accessing the
filesystem; they do not canonicalize paths or promise containment in a
directory. Existing file reads, writes and CSV policies are unchanged.

`sys.args` returns the arguments after the CLI `--` separator without an
executable or script name. `sys.pid` returns the process identifier.
`sys.env` returns absence for a missing variable and an `ioerr` for a
present value that cannot be represented as text. These operations are
actions, not compile-time constants.

## Text, regular expressions, dates and statistics

| Operation | Type |
|---|---|
| `str.upper TEXT`, `str.lower TEXT`, `str.trim TEXT` | `s -> s` |
| `str.split DELIMITER TEXT` | `s s -> [s]` |
| `str.join DELIMITER PARTS` | `s [s] -> s` |
| `rx.find PATTERN TEXT` | `s s -> (?s)^rxerr` |
| `dt.days DATE` | `s -> z^dterr` |
| `dt.date DAYS` | `z -> s^dterr` |

Case mapping and whitespace trimming use Unicode, not locale-specific
rules. Case mapping can change the number of scalars. Split preserves
empty fields, including endpoint empty fields with an empty delimiter;
join inserts the delimiter between adjacent parts.

Regex search returns the first leftmost whole match as a present string,
or ordinary absence if no match exists. Invalid or over-complex patterns
return `badpattern s` carrying the pattern. This revision uses bounded
Unicode regex execution without backreferences or lookaround. Reference
limits include a 64 KiB pattern and 8 MiB search input; over-limit search
input is a resource diagnostic.

Dates are proleptic Gregorian `YYYY-MM-DD`, from year 0001 to 9999.
Day zero is `1970-01-01`; prior dates use negative numbers. Calendar-invalid
text or a day number outside the supported calendar yields `baddate s`.
Time of day, time zones and locale-specific formats are not part of this
library subset.

`stat` supplies `avg` and `mean` aliases for the mean, `pct` for percentile
and `sd` for standard deviation, with the existing numeric semantics.
`cor` remains recognized but not executable.

## Typed JSON

`json.encode VALUE` returns `s^jsonerr`. `json.decode TEXT` returns
`a^jsonerr` with a concrete expected type `a` from the statically checked
context. Decode does not produce a dynamic JSON value. Supported shapes
include `n`, `z`, `q`, `d`, `b`, `s`, `c`, unit, optionals, sequences,
tuples, and closed structural or nominal records. The checker rejects
unsupported decode target types, including `r` and shapes such as `?()` or
an optional of an optional that would assign two meanings to `null`.
Unsupported values passed to encode produce `jsonshape s`.

Integers and finite decimals are exact; decimal scale is preserved. A
rational can be encoded as a JSON number only when it has a finite decimal
expansion; there is no implicit rounding and no invented fraction-string
format. Arrays represent ordered sequences and tuples. Objects reconstruct
fields in the checked record order. Decoding rejects duplicate, missing,
and unexpected fields instead of silently overwriting or dropping them.
An optional absent value is `null`; present payloads that would also encode
as `null` are rejected rather than silently conflated with absence.

Malformed JSON text yields `invalidjson s`; a document or value that does
not match the checked representation yields `jsonshape s`. The reference
implementation limits input and output to 1 MiB and also bounds nesting,
node counts, and numerical expansion. JSON operations
are pure and do not read or write files. Library error constructors are
ordinary nominal alternatives; printed labels are not exception keywords.

Process execution, more file mutations, general time zones, richer regex
operations and advanced statistics are outside this revision.
