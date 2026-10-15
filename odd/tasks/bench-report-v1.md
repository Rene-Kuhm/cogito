# bench-report-v1 — Track A3: harness de medición

Track A3. `cogito bench` today reports token savings only. This feature adds the
other two metric classes the measurement protocol keeps apart (latency and
cost), a provenance header, and a raw artifact for every run.

Plan source: `plan.md` (Track A3) is not present in the worktree; the acceptance
list quoted in `## Specs` is the authoritative scope.

## Specs

- **S1** — "que `cogito bench` reporte, además de recall@budget, latencia (sync
  del índice y consulta) y costo (tokens del pack y número de llamadas de
  herramienta por tarea)"
- **S2** — "que escriba los artefactos crudos de CADA corrida fuera de
  cualquier worktree (directorio configurable, por defecto
  `~/.cache/cogito/bench-runs/<timestamp>/`)"
- **S3** — "Ninguna corrida se descarta: fallas y cero resultados quedan en el
  artefacto."
- **S4** — "El reporte imprime SHA y estado limpio del worktree medido, número
  de repeticiones y tokenizer, y las tres columnas por separado: calidad,
  latencia, costo. No mezcles en un solo score."
- **S5** — "Sin cambio de comportamiento para las salidas existentes: la salida
  actual de `cogito bench` debe seguir igual con el mismo flag por defecto,
  salvo que agregues columnas. Agregalas detrás de flag si hace falta para
  preservar la salida vieja."
- **S6** — "Tests nuevos para: el formato de las tres columnas, que la salida
  vieja no cambia sin el flag, y que un worktree sucio se marca en el reporte."
- **S7** — "Commit(s): `feat(bench): report latency and cost next to recall`, y
  uno para el baseline congelado en `docs/benchmarks.md` si corresponde."
- **S8** — "En el reporte: ruta de los artefactos del baseline y los números."
- **S9** — "El baseline de `main` tiene que reproducirse dentro de la varianza
  que declares. Corré el baseline sobre `main` (en el worktree
  `~/proyectos/cogito` NO, usá un worktree detached aparte creado con `git
  worktree add --detach`) y guardá los artefactos fuera del repo."

## Design decisions

- The three metric classes are printed as three separate sections, never
  combined into one score (S4).
- **calidad** = `recall@budget` over the labelled gold set, measured in-process
  through the same `pack::build` + shared `cl100k_base` counter the MCP
  `context_pack` tool uses. When no gold set or no checkout is available the
  section says `n/a` with the reason; no number is invented.
- **latencia** = `index_sync_ms` (per repo, the `Cogito::sync_index` call) plus
  `query_ms` (per measurement, the pack search/build).
- **costo** = `pack_tokens` (tokens the pack actually used) plus `tool_calls`
  (one `context_pack` call answers a task; the exploratory term path counts the
  calls each strategy makes).
- New output is behind `--report`; the default output stays byte-identical,
  including the deterministic JSON (S5).
- Artifacts are written for every run, outside the measured worktree, one
  `<timestamp>/` directory per run with `run.json` and `report.txt` (S2, S3).

## Tasks

| id | spec | what | route | commit |
|----|------|------|-------|--------|
| T1 | S1 S2 S3 S5 | measurements, provenance, artifacts, CLI flags | inline | `feat(bench): report latency and cost next to recall` |
| T2 | S1 S4 | three-section report renderer | inline | same |
| T3 | S6 | acceptance tests | inline | same |
| T4 | S4 S8 S9 | frozen `main` baseline + `docs/benchmarks.md` | inline | `docs(bench): freeze the main baseline` |
| T5 | — | gates: fmt, clippy, test | inline | — |

## Log

- **L1** (user request, verbatim acceptance): "que `cogito bench` reporte,
  además de recall@budget, latencia (sync del índice y consulta) y costo
  (tokens del pack y número de llamadas de herramienta por tarea), y que
  escriba los artefactos crudos de CADA corrida fuera de cualquier worktree
  (directorio configurable, por defecto `~/.cache/cogito/bench-runs/<timestamp>/`).
  Ninguna corrida se descarta: fallas y cero resultados quedan en el artefacto."
- **L2**: `plan.md` and `common.md` are not present in the worktree
  (`find` over the repo and the other worktrees found neither). Scope taken from
  the acceptance list in the task prompt. Recorded here so a later reader does
  not assume the plan was read.
- **L3**: protocol reference is `docs/MEASURING_SAVINGS.md` from
  codebase-memory-mcp (MIT), read from
  `/tmp/pi-github-repos/runtime-3KShjT/.../docs/MEASURING_SAVINGS.md`. Only the
  ideas are reused: freeze the experiment, keep the three classes apart, retain
  raw outputs and failures, publish SHA + clean state.
