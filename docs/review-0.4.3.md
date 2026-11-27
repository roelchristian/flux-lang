# Draft 0.4.3 source-module implementation review

Base: the typed-only frontend from Draft 0.4.2 (PR #4).

The loader discovers local file dependencies before arity-dependent parsing,
canonicalizes each path, and reuses one module per canonical file. Parsing
occurs in independent lexical namespaces using only direct public interfaces.
Resolved declarations and nominal identities are checked in a linked graph
and lowered to one typed HIR; no imported script entry is allowed. Checked
interface metadata is available to Rust consumers and `flux interface FILE`.
This is not a bytecode ABI, persistent compilation cache, package system,
or remote loader.

## Independence and revisions

Public CLI/API tests verify the finance example, local relative resolution,
aliases, private exports, generics and constraints, function-valued exports,
nominal types and sum constructors, diamond graphs, cyclic imports, and
source-file diagnostic provenance. Separate read-only probes checked the
same loading rules across ordinary and strict CLI paths, including a module
member shadowing a registry or prelude spelling. Ten previous differential
specimens continue to match.

The review reproduced namespaced internal identifiers appearing in REPL
schema output and linked type errors. Rendering now uses readable source
names without erasing distinct nominal identities. Cross-module diagnostics
report the original file. Imported nominal types from public interfaces may
appear in signatures; a library's private nominal cannot leak from its own
public interface. REPL fragments retain their own import bases; loading a
declaration does not execute an action.
