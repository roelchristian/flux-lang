# Consolidated reference: independent review

Reviewed against the first implementation PR (`a902a6e`) and the
reference interpreter, not against illustrative historical proposals.
An independent reader checked the assembled chapters, examples and the
new archive and identified:

- The header-discovery versus typed-interface boundary was misstated.
  The reference now separates the parser's imported arities from typed
  interface checking after linked inference.
- Optional construction/extraction, statistical and sequence operators,
  Boolean literals, and the precise table/mask length rules were missing
  from the first assembly. These contracts are now in their subject
  chapters with runnable examples.
- A historical link still pointed to the old layout of the docs directory.
  It now targets the preserved snapshot, and the relative-link test
  covers the archive as well as active documentation.

The public example harness passes all 52 registered examples through the
ordinary and strict HIR workflows. The added documentation check requires
local link targets to exist and each linked runnable `.flx` example in
the specification to be registered in that harness. Relative anchors
and prose-level specification correctness still require human review.
The saved historical drafts are not normative. There are no compiler,
syntax, or standard-library changes.
