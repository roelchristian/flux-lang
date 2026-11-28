# Measuring Flux

Use a locked release build and specify the input size and platform. Retain
the old binary if comparing revisions. `benchmarks/compare.py` alternates
the binaries on end-to-end script, checking and grouped CSV workloads.
`benchmarks/phases.py` measures parse, resolve, inference, elaboration and
evaluation separately. `benchmarks/tacit.py` and `benchmarks/relational.py`
cover composition and relational workloads.

Measurements are revision- and environment-specific. See the
[Draft 0.9 elaboration PR](https://github.com/roelchristian/flux-lang/pull/13)
for a paired comparison and its limits. Use Git history to find earlier
benchmark reports.
