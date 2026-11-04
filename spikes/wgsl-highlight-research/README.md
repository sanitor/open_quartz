# WGSL highlighting research probe

This is a **research-only** workspace, not a product dependency or a syntax-highlighting implementation. The four manifests are isolated Cargo workspaces with individual lockfiles. They compare two Tree-sitter WGSL grammars and two versions of a raw WGSL lexer against the current repository catalog and 15 edge inputs. `naga` is used only as a reference for edge cases; lexer success is not shader validation.

Run from the repository root with Node.js, Rust and Cargo installed:

```sh
node spikes/wgsl-highlight-research/run.mjs
```

Optionally pass a fresh output directory as the only argument. The default is `test-results/research/wgsl-highlight-<timestamp>/`. The runner compiles the candidates with `--release --locked`, writes the input corpus, individual results and `summary.json`, and puts the shared Cargo target directory in the output directory. Results are intentionally not committed. First execution may require network access; the target directory can take hundreds of MB. Do not point it at an output directory that must be preserved: matching JSON files are overwritten.

Measurements are parser/lexer microbenchmarks on synthetic repeated source, **not** an end-to-end editor benchmark. The full methodology, observed failures, timing, rendering design, licensing caveat and unverified risks are documented in [the research report](../../docs/WGSL_SYNTAX_HIGHLIGHT_RESEARCH.md).
