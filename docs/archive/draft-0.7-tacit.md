# Flux Draft 0.7: tacit function algebra

A tacit expression builds an ordinary first-class function. Its syntax is
strictly prefix, and every combinator consumes a fixed number of operands.

```text
tacit-block = "&{" tacit-expr "}" ;
tacit-expr  = "." | callable-or-function-value
            | "@." tacit-expr tacit-expr
            | "@*" tacit-expr tacit-expr tacit-expr
            | "@?" tacit-expr tacit-expr
            | "(" tacit-expr ")" ;
```

A block contains exactly one expression. An atom denotes a visible declared
function, referenceable operator, constructor or selector, or a lexical
function-valued binding. A scalar, an action-executing special form, and the
pipeline placeholder are not function atoms. Parentheses do not create a
train or change arity. The new combinators and bare period are not ordinary
expressions outside the block; outside it, `&.` is the identity reference.
The lexer still distinguishes `.field`, `.1`, `..`, `.:`, `.{`, decimal
literals and qualified names. For compatibility, a complete pre-existing
operand keeps its lexical boundary in compact ordinary application, for
example `@.field`, `@.1`, and `@?{`. Separate the tacit operator from its
operands by whitespace when adjacency would form one of these tokens.

| Expression | Application to `x` |
| --- | --- |
| `.` | `x` |
| `@. G H` | `@ G (@ H x)` |
| `@* F G H` | `@@ F ((@ G x),(@ H x))` |
| `@? F H` | `@@ F (x,(@ H x))` |

Fork computes the left branch and then the right branch, once each, before
applying its curried combiner. Hook uses the input as its left branch; it is
not a conditional. Evaluation stops on the same fatal diagnostics as its
ordinary equivalent. Constructing a function does not run its body. Passing
an optional, typed result or suspended action through identity preserves the
value unchanged; composition never implicitly unwraps or executes it.

Pure tacit values generalize as ordinary function values. Identity has type
`'a->'a`. Numeric constraints and open record projections use the ordinary
checker. For example:

```flux
offset := &{ @* - .measured .reference };
a := @ offset .{ measured := 120; reference := 80; };
b := @ offset .{ station := "north"; measured := 125.50d; reference := 80.25d; };
sqv:[q]->[q] := &{ @* * . . };
ss:[q]->q := &{ @. +/ sqv };
norm:[q]->r := &{ @. ^/ ss };
```

Here `a` is 40, and `b` is 45.25d. An explicitly specialized sequence
function can use the ordinary lifting of its arithmetic; a user-defined
function is not automatically lifted. Use `@:` for explicit mapping.

A declaration such as `sq:q->q := &{ @* * . . };` binds a value. Call it with
`@ sq 5`, not `sq 5`. Export and import it using the existing explicit
interface rules. Exporting it does not expose generated binders or grant
direct-call syntax. All successful execution uses the same typed HIR path
as ordinary functions. Rank operators, implicit trains, direct tacit
callable declarations, and new optimization passes are outside this draft.
