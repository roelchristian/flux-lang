# Daily weather summaries

From the repository root:

```sh
cargo build --release --locked
target/release/flux run-hir examples/applications/meteorology/main.flx
```

Reads synthetic `examples/applications/meteorology/observations.csv` as a typed table. Blank
measurements are allowed. The fixture's quality rules reject invalid dates,
empty station names, present temperatures outside −90 to 60 °C, and
negative rainfall. Malformed CSV or nonnumeric measurement text is an input
error, not a missing observation.

The program prints `(accepted, 7, rejected, 3)` and four summaries sorted by
station and date. Each tuple contains station, date, accepted rows,
temperature observations, exact mean/median/min/max temperature, rainfall
observations, and total rainfall. For example:

```text
(north, 2026-07-01, 3, 2, ?1 20, ?1 20, ?1 18, ?1 22, 2, ?1 5/2)
(north, 2026-07-02, 1, 0, ?0, ?0, ?0, ?0, 1, ?1 0)
(south, 2026-07-03, 1, 1, ?1 27, ?1 27, ?1 27, ?1 27, 0, ?0)
```

Here `?0` is absent and `?1 0` is an observed zero. Summaries do not
mistake a missing temperature or rainfall total for zero.
