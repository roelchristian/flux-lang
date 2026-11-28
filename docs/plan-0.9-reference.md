# Plan: Consolidated Flux language reference

Base: PR #13, `a902a6e`. This is the second ranked P0/P1 item. No syntax,
standard-library behavior or compiler changes.

1. Keep `docs/specification.md` as the one normative entry point and organize
   the reference by topic, not historical amendment order. State which
   claims are implemented, limited/experimental or merely proposed.
2. Independently author and reconcile (a) source, function and evaluation
   chapters, (b) types, constraints, numerics and relational data chapters,
   and (c) effects, modules, library and host chapters. Ground language
   claims in the implementation and the examples manifest. Prefer links to
   real examples over copied fences outside conformance.
3. Archive original snapshots and reviews visibly and retain their text;
   keep old URLs useful with archival notices or redirects. Rewrite the
   current implementation guide and update the README and example links.
4. Independently audit syntax claims, links, library inventory and declared
   limits, revise findings, then run the executable corpus and link checks.
   Publish as its own PR stacked on the verified PR #13 head.
