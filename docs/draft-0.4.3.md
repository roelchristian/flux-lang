# Flux Language Specification: Draft 0.4.3

Draft 0.4.3 adds local source modules to the typed frontend. It preserves
the import/export notation, ASCII syntax, fixed arity, and explicit effects
of earlier drafts. It does not add packages, remote loading, or a second
runtime.

## Imports and interfaces

```flux
<< "finance.flx" : fin;
<< "io";
```

Registry imports such as `"io"` retain their existing meanings. A source
import names a local `.flx` file relative to the importing file's canonical
directory, not the process working directory. Dot segments and parent
traversal are allowed. Absolute paths and URL/scheme forms are not source
imports. A source import may specify an alias; otherwise the file basename
without its `.flx` extension supplies the namespace, if it is a valid Flux
identifier. The loader discovers the graph before parsing expressions so the parser already knows how many operands an imported
callable consumes. This does not permit implicit application of a value:
an exported binding with a function type is still a value, applied with
`@` or `@@`, rather than a named callable with syntax-level arity.

A source interface contains exported names, their kinds and syntactic
arity, their checked annotations or type schemes and constraints, and
identities for exported nominal declarations. Modules are checked against
their own lexical declarations, registry imports and the public interfaces
of direct dependencies. Imports do not implicitly re-export names, and an
importer cannot see a dependency's private names.

## Exporting a library

```flux
>> (net, margin);

net(revenue,cost:q):q := - revenue cost;
margin(revenue,cost:q):?q :=
    ?{
        = revenue 0 : ?0;
        _ : ?1 (/ (net revenue cost) revenue);
    };
```

An exported function must have explicit parameter and result types. A
polymorphic interface may include explicit type variables and constraints;
uses instantiate schemes independently. Exported values must have an
explicit type. A nominal record or sum can be exported as a type declaration.
Exporting a sum makes its alternatives available through the module
namespace, including use as a constructor or in a recovery pattern.
Public types and constructors use the same qualified notation as callable
members, for example `fin.invoice` in an annotation or `fin.missing` as an
error constructor. A local definition can use its own unqualified names;
a consumer cannot use an unqualified source-library constructor merely
because it imported the library. Two distinct modules do not share nominal
identity merely because they spell or define a type the same way. An export cannot expose a nominal
type owned by the library but kept private, whether in a function/value
type, public record field or constructor payload. An interface may mention
a public nominal type from a dependency without re-exporting its name;
clients that need that name must import its owner directly.

A consumer uses the namespace specified by the import:

```flux
<< "finance.flx" : fin;
<< "io";

profit := fin.net 12000 8500;
ratio := fin.margin 12000 8500;

!{
    ^{
        ^? ! io.print (profit,ratio);
        ()
    }
};
```

With these two files side by side, `flux check app.flx` validates both
without output, and `flux app.flx` prints `(3500, ?1 7/24)`. The process
working directory still controls any relative paths passed to ordinary
runtime I/O; it does not control the resolution of a source import.

A library cannot contain a script entry; only the root source file can.
Imported declarations cannot execute effects implicitly. In particular,
initializing a binding to a suspended action does not perform that action.

## Identity, order and diagnostics

A canonical path identifies one source module within a compilation. Distinct
aliases can refer to the same canonical module without duplicating its
nominal types or eager initializers. Aliases must not be ambiguous within
one importing module. A diamond import has one instance of the shared
dependency: its declarations are initialized once in dependency order, not
once per incoming import edge. The root entry runs after its dependencies.
The compiler lowers the program to a single typed HIR. Internally, symbol
numbers are scoped to that checked graph; an interface is not a source of
numeric identities that can be reused in a different compilation.

Import cycles are compile-time errors, including cycles formed by different
relative spellings of the same file. Diagnostics identify the source file
and location where the failing import, declaration, or expression occurs.
Dependency-cycle reports include the import chain. Checking discovers and
validates dependencies but does not execute the entry, actions, or dependency
initializers. An uncaught recoverable failure raised in a library retains
its original file and position, with propagation-site notes that may refer
to the importer.

The Rust entry points for file-backed programs are `check_path` and
`run_path`. A source string has no implicit import directory; tools supplying
in-memory root text must supply an explicit virtual path.

This draft excludes a package registry, remote fetching, implicit module
discovery, and dynamic imports. It does not add separate compiled interface
files or persistent incremental caches across compiler invocations.
