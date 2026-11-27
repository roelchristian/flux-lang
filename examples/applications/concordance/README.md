# Concordance

From the repository root:

```sh
cargo build --release --locked
target/release/flux run-hir examples/applications/concordance/main.flx
```

Reads the synthetic text in `examples/data/corpus.txt`, lowercases it, treats
the listed punctuation and whitespace as word separators, and groups tokens
by spelling. This is an explicit tokenizer for the sample, not Unicode word
segmentation. The ranking sorts by count descending, keeping alphabetical
order on ties.

```text
alphabetical (word, frequency)
[(and, 1) (follows, 2) (rain, 3) (sun, 2) (wind, 2)]
frequency (descending; ties alphabetical)
[(rain, 3) (follows, 2) (sun, 2) (wind, 2) (and, 1)]
```
