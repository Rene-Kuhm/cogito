# refs-graph v1 — name-based reference edges for TypeScript and Rust

Track C2. Worktree `~/proyectos/cogito-wt/refs-graph`, branch `feat/refs-graph`,
from `feat/impact` at `4a0e8a0`. Gates after T1: `cargo test --all` 260 passed,
0 failed, 4 ignored; the pre-change count is not measured on this machine.

Note: `plan.md` and `common.md` do not exist on this machine (searched
`~/proyectos`, `~/odd`, `/tmp`); the section "C2" and the common rules arrived
verbatim in the task prompt and are transcribed below as S1-S7.

## Specs

S1. "aristas de referencia por nombre (un uso de un símbolo importado o
declarado en el mismo archivo), resueltas con las aristas de importación que ya
están en el índice. Sin inferencia de tipos."

S2. "Solo TypeScript y Rust en v1; para cualquier otro lenguaje, `impact` debe
responder "not supported" en vez de adivinar."

S3. "`impact` debe decir en su salida que es sobreaproximación."

S4. "Tests con fixtures temporales en TypeScript y Rust: el conjunto de
referencias es exacto." y "Test de "not supported" para un lenguaje no
soportado."

S5. "Tabla de precisión/recall en `odd/tasks/refs-precision-v1.md`", con tamaño
de muestra y etiquetas hechas por un agente.

S6. "no toques `pack.rs`. Si cambiás la superficie MCP, actualiza `mcp_test.rs`
y las instrucciones en el mismo commit."

S7. Commits convencionales:
`feat(impact): name-based reference edges for TypeScript and Rust`,
`docs(impact): precision and recall of reference edges`.

## Decisions (why, not what)

D1. The old signal was `files_declaring`: any file that declares the same name.
That is a guess about a *declaration*, never a use, and it cannot tell a real
reference from an unrelated homonym (`mcp_test.rs` pins the false positive as
"the file that declares the same symbol name"). C2 replaces it: a file is linked
only when the index has an import edge to the changed file *and* that file
contains a use of the name that is not its own declaration.

D2. "Use" is counted over tree-sitter identifier leaves, so a name inside a
comment or a string is not a use. A file that declares `N` once and mentions it
`k` more times has `k` uses; a file that only declares `N` has none. This is the
only "resolution" v1 does — no types, no scopes, no shadowing analysis.

D3. The referencing file must be reachable through an existing import edge in
the index (`index::importers`). A same-name declaration without an import edge
is not a reference. A reference may land on the changed file itself (a use of a
name declared in the same file); that self edge is reported but never counts as
an affected file, because that file is already in the diff.

D4. Supported extensions are exactly `ts`, `tsx`, `mts`, `cts` and `rs`. Plain
`.js`/`.jsx` is *not* in v1: the task names TypeScript, and treating JavaScript
as TypeScript would be another guess. Any other extension gets an explicit
"not supported" line naming the file, never a fallback to `files_declaring`.

D5. `Impact.namers` is removed, so the affected set shrinks by exactly the
name-only false positives. That is a visible MCP-surface change, so
`mcp_test.rs`, the MCP `INSTRUCTIONS`, the `impact` tool description,
`agents.rs`, `README.md` and `docs/integrations.md` change in the same commit
(S6).

## Tasks

- T1 — reference edges (S1, S2, S3, S6) · inline · commit 1
  `feat(impact): name-based reference edges for TypeScript and Rust`
- T2 — precision/recall over the holdout repos (S4, S5) · inline · commit 2
  `docs(impact): precision and recall of reference edges`

## Log

L1 (user, verbatim): See the task prompt: "Objetivo: aristas de referencia por
nombre (un uso de un símbolo importado o declarado en el mismo archivo),
resueltas con las aristas de importación que ya están en el índice. Sin
inferencia de tipos. Solo TypeScript y Rust en v1; para cualquier otro lenguaje,
`impact` debe responder "not supported" en vez de adivinar." plus acceptance and
restrictions. `plan.md`/`common.md` were not found on disk.
