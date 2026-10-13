<!-- cogito:start -->
**This project is indexed by Cogito** — run `cogito map --card` to see the
project card.

**Use these MCP tools instead of whole-file reads and greps:**
- `project_map` — module overview (once per session to orient)
- `search_code` — find symbols, files and modules by name (returns file:line)
- `file_outline` — a file's declarations with line ranges
- `related_files` — what a file imports and what imports it

**Guardrail:** Before reading a large file, call `file_outline` to get its
declarations with line ranges, then read only the range you need. Reading the
same range twice is always allowed.<!-- cogito:end -->