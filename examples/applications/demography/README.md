# Demography: population change and regional shares

From the repository root:

```sh
flux run-hir examples/applications/demography/main.flx
```

The script totals a synthetic ward census by district and year, then
left-joins newer totals to their baselines. East has no earlier total; West
has an earlier total of zero. Neither has a defined percentage change, but
the report keeps the two cases distinct.

A second join attaches the regional classification in `examples/data/regions.csv`
to the current-year totals. The script sums residents within each region and
divides by the total across classified districts. Unlike the displayed
change percentages, these shares remain exact rationals.

The classified current-year total is 3262 residents. Coastal has 2300
(`1150/1631` of the total), and Inland has 962 (`481/1631`). The output also
lists each district's current and earlier totals and its change where defined.
