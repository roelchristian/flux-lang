# Flux implementation guide

The [language reference](specification.md) defines current behavior; this
guide explains the Rust pipeline and verification.

## Compiler and runtime

| Stage | Contract |
| --- | --- |
| Lexer, parser and modules | Record byte spans. Discover local and imported callable arities before parsing prefix operands. Lower `name/arity` to ordinary named functions with hygienic synthetic inputs. |
| Tacit lowering | Build curried lambdas with hygienic inputs and eager captures outside the lambda body. Evaluate fork branches before curried application. No new runtime callable kind. |
| Resolution and inference | Assign lexical symbols and dependency groups. Check rank-one schemes, constraints, shapes and exact numeric joins. Stable syntax IDs associate expression types and selected operations with source occurrences. |
| Elaboration and admission | Lower checked calls, lifts, schemas, closures and error scopes to typed HIR. The compiler shares one borrowed syntax tree. Independently reused metadata must describe the same syntax, including declaration and pattern offsets; matching expression IDs alone is not sufficient. |
| Evaluation | Execute only admitted HIR in scripts and the REPL. The `legacy-oracle` Cargo feature enables a separate differential-testing interpreter, not fallback. Runtime I/O is attached to suspended actions. |
| Formatting | Use original tokens and opt-in parser ranges to change layout without changing the token stream. |

Evaluation is eager and resource-bounded. Dense-array HIR selects the
same numeric operation for direct or first-class application and carries
the specific lift for scalar or pairwise broadcasting. The runtime
checks dynamic shapes and walks row-major offsets rather than copying
expanded singleton data. Numeric, JSON and array limits are in the
[reference](specification.md); relational evaluation is ordered,
not a columnar optimizer.

Mathesis pointwise functions reuse the checked scalar, sequence and dense
lift evidence. Exact integer and polynomial kernels take their numeric domain
from elaboration, and numerical methods use ordinary checked callback
application with explicit work limits.

The combined `!^{ ... }` opener lowers to ordinary action/fallible HIR.
Inference checks immediate unit action statements before elaboration
inserts lexical propagation nodes at their original spans.

## Machina execution

Dense arrays use immutable shared buffers with logical shape, strides
and offset. Eligible elements use packed Boolean (one byte per value,
not a bitset), signed/unsigned 64-bit integer or binary64 buffers. Wide exact values, decimals,
unsupported operations and user callbacks use the generic checked
executor. Transposition, permutation and section update view metadata;
contiguous reshape reuses storage where possible. Access, rendering,
comparison and fallback kernels all use logical row-major order.

Specialized kernels cover selected arithmetic, comparisons,
broadcasting, packed `:r` conversion, real negation, absolute value,
`math.exp`, `math.sin`, Boolean operations, intrinsic reductions and
contiguous or strided contractions. Eligibility
uses checked HIR and the operand layout. Integer overflow in a narrow
kernel falls back to exact arithmetic, not wrapping or approximation.
Matrix kernels may use a scratch layout for locality but preserve
multiply-then-add and the original accumulation order.

The numerical plan only fuses direct same-shape chains of packed
real negation or Boolean complement when policy allows. It evaluates
the leaf once, charges the original logical stage work, and replays
unfused stages if the layout or memory headroom is unsuitable. It is
not shape inference, arbitrary arithmetic fusion, SIMD or parallelism.

## Runtime policy

Rust hosts configure `ResourcePolicy` and `RuntimeOptions { resources,
optimize }`. Use `run_source_with_options`, `run_path_with_options`
or the prepared program's `run_with_host_options` / `view_with_host_options`.
Existing entry points retain defaults. Each execution validates its
options and scopes its budget; a nested execution restores the caller's
policy and accounting when it ends.

The CLI accepts these decimal environment settings for execution:

| Environment variable | Default | Budget |
| --- | ---: | --- |
| `FLUX_MAX_RANK` | 16 | Maximum array rank, from 1 through 16 (rank-zero values remain supported) |
| `FLUX_MAX_ELEMENTS` | 100,000 | Logical array dimensions and element counts; also used for checked range materialization |
| `FLUX_MAX_MEMORY` | 268,435,456 | Bytes of tracked live dense payload and instrumented scratch storage |
| `FLUX_MAX_WORK` | 1,000,000 | Evaluator work, including charged logical kernel work |
| `FLUX_MAX_EXACT_BITS` | 1,048,576 | Size of exact numeric results in bits |
| `FLUX_MAX_NUMERIC_ITERATIONS` | 100,000 | Work ceiling for instrumented numerical iterations |
| `FLUX_MAX_DEPTH` | 128 | Evaluation depth, from 1 through 128 |

Limits must be positive and fit the host representation. Dedicated
algorithm, source and I/O bounds may still apply. Method-specific
failure can be a typed solver result; exhaustion of a tighter host
iteration budget is fatal. Memory accounting is
not a process RSS cap: shared views charge a payload once. Completed
ordinary sequences, noninstrumented allocations, allocator overhead,
frames and operating-system I/O are not globally tracked, even
where a construction scratch buffer has a lease. Set budgets
conservatively rather than assuming an arbitrarily large setting is
safe for the host.

`FLUX_OPTIMIZE=0` selects generic typed execution instead of the
specialized paths. The default, also selected by `FLUX_OPTIMIZE=1`, is
optimized execution. This switch does not select the legacy interpreter
or bypass resource checks. Generic mode keeps the shared view
architecture; it is not a copy-only emulation of Draft 0.18.
Physical buffers can have different sizes,
so a tight tracked-memory policy need not admit both layouts. For a
same-binary comparison:

```sh
flux run-hir examples/mathematics/dense-views.flx
FLUX_OPTIMIZE=0 flux run-hir examples/mathematics/dense-views.flx
```

## Verification

Use Rust 1.85 or later. From the project root:

```sh
cargo fmt --all -- --check
cargo test --locked --all-features --all-targets
cargo clippy --locked --all-features --all-targets -- -D warnings
cargo build --locked --release
```

Integration tests are grouped into `compiler`, `language`, `data`,
`tooling` and `examples` targets (`cargo test --locked --test NAME`).
The [example harness](../examples/README.md#inputs-and-verification)
runs ordinary and strict HIR paths. Compare two release binaries with
`python3 benchmarks/differential.py BEFORE AFTER --typed-oracle`;
[the exact numeric oracle](../benchmarks/exact_numeric_oracle.py) supplies
independent integer, rational and decimal expectations.

## Performance

Use locked release builds and record workload size, compiler, platform
and mode. Retain the old binary for paired comparisons.

| Harness | Workload |
| --- | --- |
| [compare.py](../benchmarks/compare.py) | Alternate binaries on script, checking and grouped CSV workloads |
| [phases.py](../benchmarks/phases.py) | Parse, resolve, infer, elaborate and evaluate separately |
| [tacit.py](../benchmarks/tacit.py), [relational.py](../benchmarks/relational.py) | Composition and relational workloads |
| [machina.py](../benchmarks/machina.py) | Checked paired wall time and Linux peak RSS for shared workloads, dense kernels and contractions |

```sh
python3 benchmarks/machina.py FLUX_018 FLUX_019 --reference FLUX_017 \
  --generic --generic-env FLUX_OPTIMIZE=0 \
  --elements 10000 100000 --matrix-sizes 64 --runs 7 --allow-limits
```

Machina samples check shapes and independent exact or tolerance
references before timing. Reported wall time includes Flux startup,
checking and setup, not only a kernel. Setup-only controls and
`--kernel-repeats` distinguish those costs. Matrices are tested with
both a transposed right operand and a contiguous copy of it.
`--large` and `--standard-matrices` are opt-in; configure higher host
budgets explicitly. A checked limit or timeout is not a successful
timing. See the harness's `--help` for per-binary overrides.

### Draft 0.19 paired sample

Locked Rust 1.85 release builds of Drafts 0.17–0.19 were checked on
Linux x86-64 (Xeon Platinum 8573C), pinned to one logical CPU.
These are median elapsed milliseconds from seven alternating samples;
the generic column is the same 0.19 binary with `FLUX_OPTIMIZE=0`.
Samples include checking and preparation. Timing varied (the optimized
100k real sum ranged from 8.29 to 18.38 ms); this is not a kernel-only
or cross-platform claim.

| Workload | 0.17 | 0.18 | 0.19 | 0.19 generic |
| --- | ---: | ---: | ---: | ---: |
| Scalar squares, 10k | 5.36 | 5.41 | 5.36 | 5.37 |
| Static check, 4k declarations | 60.84 | 59.17 | 59.22 | 60.44 |
| Typed CSV and grouping, 6k rows | 10.00 | 10.23 | 9.65 | 9.37 |
| Dense integer input, 100k | 6.08 | 6.00 | 7.39 | 6.60 |
| Dense integer arithmetic, 100k | 36.11 | 34.93 | 9.67 | 26.68 |
| Dense real arithmetic, 100k | 39.31 | 39.94 | 9.26 | 29.10 |
| Dense real sum, 100k | 28.18 | 29.12 | 11.42 | 29.71 |
| Real negation chain, 100k | 50.03 | 50.31 | 8.26 | 36.68 |
| Transpose with section, 100k | 14.55 | 15.01 | 7.64 | 6.86 |
| Dense sine, 100k | n/a | 37.07 | 8.88 | 26.95 |
| Dense exponential, 100k | n/a | 36.84 | 9.17 | 26.97 |
| 64² product, contiguous right | 17.01 | 17.47 | 3.23 | 17.22 |
| 64² product, strided right | 17.14 | 17.39 | 2.81 | 17.87 |

The setup-only real input control took 22.05 → 7.71 ms from 0.18 to
0.19; some end-to-end gains come from preparation. Linux median peak
RSS for the 100k real arithmetic workload was 22,788 → 13,480 KiB
(18,476 KiB in generic mode). Dense integer setup itself regressed
in this run. The scalar, static check and CSV medians changed little
compared with the dense kernels.

### Raised-budget runs

These separate runs used three checked samples at each size. Both 0.19
matrix modes used `FLUX_MAX_ELEMENTS=10000000`,
`FLUX_MAX_WORK=5000000000` and `FLUX_MAX_MEMORY=2800000000` on
the 64-bit test host. The older builds hit work limits at 128 and
256 and range limits at 512 and 1,024. The table reports elapsed
milliseconds; a cutoff is not a measured median.

| Square matrix side | Optimized, strided | Optimized, contiguous | Generic, strided | Generic, contiguous |
| ---: | ---: | ---: | ---: | ---: |
| 128 | 5.67 | 6.25 | 124.45 | 119.80 |
| 256 | 24.79 | 28.28 | 963.99 | 982.04 |
| 512 | 166.09 | 180.80 | 8 s cutoff | 8 s cutoff |
| 1,024 | 1,293.79 | 1,334.97 | 8 s cutoff | 8 s cutoff |

At 1,024², optimized median peak RSS was 102,540 KiB (strided) and
110,936 KiB (contiguous). To reproduce, use `--standard-matrices`
with `--match '^matmul-'`, the three limits above as `--after-env`
overrides, and `--runs 3 --timeout 8 --allow-limits --allow-timeouts`.

The larger real-vector test used `FLUX_MAX_ELEMENTS=10000000`,
`FLUX_MAX_WORK=300000000` and `FLUX_MAX_MEMORY=1300000000`.
At 1M elements, optimized real arithmetic and sum took 95.58 and
82.23 ms; at 10M, 1,214.32 and 1,192.75 ms. The 10M arithmetic
run used 941,720 KiB median peak RSS, including input construction.
Draft 0.18 hit its range limit; no large-vector generic ratio was
measured. Sine and exponential were only measured at the smaller
sizes above.

Measurements are environment-specific; see the
[Draft 0.9 comparison](https://github.com/roelchristian/flux-lang/pull/13)
for an example. Earlier reports are in Git history.
