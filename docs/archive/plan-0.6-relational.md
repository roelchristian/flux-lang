# P1: relational and tabular computation

Base: the typed standard library and exact numerics (PR #7). Implement the
existing 0.2/0.3 join signatures rather than introducing new core punctuation.

- Inner `*=` and left `*?` equijoins use explicit independently typed key
  functions. They produce paired rows, preserve duplicate matches and
  left-then-right input order, and retain checked schemas when empty. A left
  join uses optional right rows; an absent optional key matches another
  absent key under Flux equality. `df.semi` and `df.anti` preserve left rows.
- Extend stable grouping to hashable composite tuple and optional keys.
  Keep exact numbers, type identity, and floating zero equality consistent
  between key hashing and checked equality. Compute key callbacks once per
  applicable row; cap materialized results independently of input size.
- Add stable same-schema `df.union` and `df.distinct`, compile-time schema
  projection and renaming via `df.select`, and explicit `df.dropna`,
  `df.fill`, and `df.present` transformations. Preserve existing `+:`,
  `df.agg`, and named/positional schema distinctions.
- Run a checked financial reconciliation using two synthetic CSVs, optional
  decimal amounts, duplicate-source detection, exact discrepancies, grouping,
  and deterministic CSV output. Confirm that missing amounts, unmatched keys
  and real zero remain distinct. Review independently and open a separate
  stacked PR. Windows, pivots, as-of joins, and a columnar engine are later.
