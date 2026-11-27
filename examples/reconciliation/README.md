# Reconcile two small financial CSVs

All data is synthetic. Run the checked HIR example from this directory so
the two relative CSV input paths resolve here. From the repository root:

```sh
cargo build --locked
cd examples/reconciliation
../../target/debug/flux check reconcile.flx
../../target/debug/flux run-hir reconcile.flx
```

A successful run prints `(8, 9, 5)`: eight raw inner-join rows, nine raw
left-join rows, and five exception keys. It writes `report.csv` without
replacing an existing file. Move the old report or run from a fresh copy
of this directory before repeating.

```csv
key,status,ledger,statement,difference
B,amount_difference,1,1,0.25
C,ledger_only,1,0,
D,statement_only,0,1,
E,duplicate_source,2,2,
G,missing_statement_amount,1,1,
```

The difference is ledger minus statement: for B, **75.20 − 74.95 = +0.25**
in exact decimal arithmetic. Counts describe the original source rows.
The raw joins preserve E's four pairs; the reconciliation groups by ID
and quarantines any ID repeated on either source before comparing
amounts. A blank difference means no valid one-to-one subtraction, not
zero. G has a statement row with an absent amount; C has no statement row
at all. F matches on a present `0.00` amount and A matches on `100.10`,
so neither appears in the exception report.
