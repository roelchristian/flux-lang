# Astronomy: cross-matching a star catalogue and a survey

Run [the script](main.flx) from the repository root:

```sh
flux run-hir examples/applications/astronomy/main.flx
```

The script cross-matches the [catalogue](../../data/catalogue.csv) and
[parallax survey](../../data/parallax.csv) by star name and constellation.
It deduplicates the survey first: two otherwise identical Vega measurements
compare equal at different decimal scales. The survey's Altair row names the
wrong constellation, so it does not match. Field C matches an object but has
no parallax. The zero test row has a present measurement but does not yield a
distance.

For a positive survey parallax in milliarcseconds, the script computes
`1000 / parallax` parsecs exactly, then rounds to two decimal places. This is
a simple demonstration, not a model of measurement uncertainty.

The output starts with `(5, 6, 5)`: five catalogue objects, six survey rows
and five distinct survey rows. Four catalogue objects match the survey, and
two have positive parallax. Their distances are 2.64 pc for Sirius and 7.68 pc for Vega. The [manifest](../../manifest.json) records every output row.
