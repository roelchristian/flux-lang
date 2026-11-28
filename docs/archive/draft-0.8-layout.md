# Flux Draft 0.8: layout and typographical conventions

Flux remains a strict prefix language. Arity and explicit delimiters, not
line breaks or indentation, determine how source is parsed. Whitespace
must still keep distinct tokens apart: in particular, `.`, `..`, `.field`,
and the adjacent identity reference `&.` are different forms.

## Margins

Use four spaces for each structural level. Keep short signatures and simple
expressions compact. Give complex operands their own indented lines, with
the same margin for siblings. This is the same expression whether printed
on one line or several:

```flux
distance(x,y:q):r :=
    ^/
        +
            * x x
            * y y;

sign(x:q):z := ?{
    < x 0q : -1z;
    = x 0q :  0z;
    _      :  1z;
};
```

Align neighboring guard colons or record field operators when that makes
the structure easier to scan. Give a substantial branch body a deeper
margin. In declarations, prefer compact `name:type` on one line; a long
signature may dedicate a line to each parameter. Retain all required
semicolons. Use parentheses where they help the reader see a boundary;
omit redundant pairs by hand only when arities make the tree clear.

## Tacit expressions and pipelines

Keep small functions economical. In a larger composition, subordinate
the two branches to their combiner. Pipelines run from top to bottom;
each stage gets its own line inside a substantial block.

```flux
point :: {
    x : q;
    y : q;
};

sq := &{ @* * . . };
magnitude:point->q := &{
    @* +
        @. sq .x
        @. sq .y
};

total := |{
    [1q 2q 3q 4q];
    @: sq $;
    +/ $
};
```

## Effects and recovery

Give effect and fallible blocks their own margins, even when nested. Keep
short recovery alternatives together; for longer ones, align the patterns
and indent the branch expressions.

```flux
!{
    ^{
        ^? ! print total;
        ()
    }
};
```

## Formatter

`flux fmt FILE` writes canonical source to stdout and never edits the
file. `flux fmt --check FILE` is quiet for a canonical file, and returns
status 1 with a diagnostic if formatting is needed. Invalid source or I/O
errors also produce failure diagnostics (not a successful formatting run).

The formatter preserves the token sequence, comments, string contents,
and explicit parentheses. It does not change operator arities, insert
semicolons, introduce an offside rule, or make an unverifiable rewrite.
Formatted source can be formatted again without changing it. Comment
indentation and surrounding whitespace may change; comment wording and its
position relative to source tokens are retained. Trailing horizontal
whitespace is removed.

See the [executable tour](../../examples/README.md) for examples written in
this style. Use the formatter as the mechanical baseline and add helpful
grouping or logical phase breaks when writing source.
