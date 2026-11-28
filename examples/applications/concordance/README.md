# Concordance

Run [the script](main.flx) from the repository root:

```sh
flux run-hir examples/applications/concordance/main.flx
```

The script lowercases the [synthetic corpus](../../data/corpus.txt),
splits on the listed punctuation and whitespace, and groups tokens by
spelling. This is an explicit tokenizer for the sample, not Unicode word
segmentation. The ranking sorts by count descending, keeping alphabetical
order on ties.

```text
alphabetical (word, frequency)
[(and, 1) (follows, 2) (rain, 3) (sun, 2) (wind, 2)]
frequency (descending; ties alphabetical)
[(rain, 3) (follows, 2) (sun, 2) (wind, 2) (and, 1)]
```
