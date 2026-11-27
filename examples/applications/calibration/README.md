# Audit instrument calibration

These synthetic CSVs each hold seven rows. Values are exact decimal
millimetres. A reference row may have a blank reading; a measured reading
must be present. Blank and zero have different meanings.

Run from this directory so relative CSV paths resolve here:

```sh
flux check audit.flx
flux run-hir audit.flx
```

The check does not create any output file. The run prints `(8, 9, 5)`:
eight raw inner-join pairs, nine raw left-join rows, and five unique sample
findings. It creates `findings.csv` with the rows in [expected.csv](expected.csv).
Run again without moving the first report, and the command returns a typed
existing-file error without changing the report.

- `schema.flx` exports the row types. The entry reads both CSVs against them.
- `matching.flx` counts the raw joins and groups both inputs to find repeated
  sample IDs. E yields four raw pairs, but goes into just one finding.
- `findings.flx` quarantines repeated IDs before comparing values and sorts
  the findings by sample. Its tacit offset is measured minus reference.

B's offset is exactly `75.20 - 74.95 = 0.25` mm. C has no reference row;
G has one, but its reading is absent. D lacks a measurement. A and F agree
numerically even though their displayed decimal scales differ; F is a
present zero, not a missing value.
