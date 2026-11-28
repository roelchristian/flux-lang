# Draft 0.7 implementation plan: tacit function algebra

**Baseline:** The complete Draft 0.6 stack through PR #8, including its final
review revisions. Production execution remains typed HIR only.

1. Add maximal-munch tokens and a dedicated Polish subgrammar. Teach balanced
   scans about the opener; preserve selectors, decimal tokens and qualified
   names. Track generated expression origins and enforce parser depth bounds.
2. Lower to ordinary function references, hygienic lambdas, single application
   and tuple application. Keep a callable value a value at a module boundary.
   No new evaluator or dispatch mechanism is anticipated.
3. Check ordinary numeric constraints, structural rows, contextual sequence
   lifting, higher-order evidence and action suspension on the lowered tree.
   Do not extend rank, numeric promotion or effect inference.
4. Independently exercise strict HIR, ordinary CLI, modules, REPL inspection
   and formatter. Compare explicit and tacit workloads with the same types,
   inputs and result checks. Run the unchanged P0/P1 regression and oracle
   suites, then audit and revise before release.
