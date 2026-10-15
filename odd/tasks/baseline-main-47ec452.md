# Baseline congelado — main 47ec452

Medido con `cogito bench --report` (binario de `feat/eval-harness` @ 744debc) sobre un worktree limpio de `main` @ 47ec452 (`~/proyectos/cogito-wt/baseline-main`, `git status` vacío). Corrida: 1 repetición. Artefacto crudo fuera del repo: `~/.cache/cogito/bench-runs/2026-10-15T20-31-15Z/`.

```
Cloning ripgrep... ✓
Cloning bat... ✓
cogito bench report
repetitions: 1   tokenizer: cl100k_base
artifacts:   /home/tecnodespegue/.cache/cogito/bench-runs/2026-10-15T20-31-15Z

PROVENANCE — the measured worktree
repo                 commit         worktree   dirty  index_sync_ms
ripgrep              3fce3b5bb023   clean          0            8.5
bat                  d9559c69f541   clean          0         1398.9
ripgrep              3fce3b5bb023   clean          0           13.3
bat                  d9559c69f541   clean          0          884.3

QUALITY — recall@budget (higher is better)
task       lang   budget  hits  expected   recall
rg-01      en       2000     2         2   100.0%
rg-02      en       2000     2         2   100.0%
rg-03      es       2000     2         3    66.7%
rg-04      es       2000     2         2   100.0%
rg-05      en       2000     0         4     0.0%
bat-01     en       2000     2         2   100.0%
bat-02     es       1000     0         2     0.0%
bat-03     en       2000     2         2   100.0%
bat-04     es       2000     0         2     0.0%
bat-05     en       4000     2         2   100.0%
mean       all                              66.7%
mean       en                               83.3%
mean       es                               41.7%

LATENCY — milliseconds (lower is better)
index sync
repo               index_sync_ms
ripgrep                      8.5
bat                       1398.9
ripgrep                     13.3
bat                        884.3
query
label                   query_ms
regex                        1.3
Matcher                      8.8
search                       2.4
flags                        1.0
syntax                       4.0
Theme                        1.7
highlighter                 25.3
assets                       2.2
rg-01                        2.5
rg-02                      177.6
rg-03                       54.8
rg-04                      186.7
rg-05                      253.1
bat-01                      69.8
bat-02                      50.2
bat-03                     126.2
bat-04                       2.3
bat-05                     551.8

COST — pack tokens and tool calls (lower is better)
label              pack_tokens tool_calls  baseline_calls
regex                        -         12               7
Matcher                      -         12               7
search                       -         12               7
flags                        -         12               7
syntax                       -         12               7
Theme                        -         12               7
highlighter                  -         12               7
assets                       -         12               7
rg-01                      882          1               -
rg-02                     1990          1               -
rg-03                     1981          1               -
rg-04                     1989          1               -
rg-05                     1995          1               -
bat-01                    1976          1               -
bat-02                     993          1               -
bat-03                    1983          1               -
bat-04                      90          1               -
bat-05                    3999          1               -

───────────────────────────────────────────────────────────────────────────
Repository: ripgrep  (commit 3fce3b5b)
term                 │   bl_find │   bl_read │  co_find │  co_read │  savings │   pct%
---------------------+-----------+-----------+----------+----------+----------+-------
regex (5)            │     18747 │     35710 │      346 │     6787 │    47324 │   86.9
Matcher (5)          │     11801 │     24455 │      331 │     5383 │    30542 │   84.2
search (5)           │     43680 │     44252 │      380 │    14765 │    72787 │   82.8
flags (5)            │      6643 │     50228 │      393 │     4185 │    52293 │   92.0
---------------------+-----------+-----------+----------+----------+----------+-------
               TOTAL │     80871 │    154645 │     1450 │    31120 │   202946 │   86.2

───────────────────────────────────────────────────────────────────────────
Repository: bat  (commit d9559c69)
term                 │   bl_find │   bl_read │  co_find │  co_read │  savings │   pct%
---------------------+-----------+-----------+----------+----------+----------+-------
syntax (5)           │    179645 │     28934 │     1385 │    19457 │   187737 │   90.0
Theme (5)            │     20115 │     40106 │      233 │     4570 │    55418 │   92.0
highlighter (5)      │     12687 │     71217 │      207 │     8362 │    75335 │   89.8
assets (5)           │    126367 │     28853 │      275 │     8890 │   146055 │   94.1
---------------------+-----------+-----------+----------+----------+----------+-------
               TOTAL │    338814 │    169110 │     2100 │    41279 │   464545 │   91.5
```

Nota: el bloque PROVENANCE y LATENCY lista ripgrep y bat dos veces (una por cada llamada de medición). Es un defecto de presentación del reporte, no de los números; queda para corregir.

Reproduce los números de quality y costo de la baseline previa (66,7 % recall; 86,2 % ripgrep, 91,5 % bat). Latencia: una sola repetición, no es estable todavía.
