# output-trim v1 — trim large Bash outputs behind an opt-in flag

## Goal

Don't re-send the middle of long Bash stdout every turn. Verified the
protocol shape first, then measured the local saving range, then shipped
the Rust hook + the `cogito setup` wiring.

## Verified JSON shape

Claude Code's `PostToolUse` accepts a replacement through
`hookSpecificOutput.updatedToolOutput` (works for any tool, not only MCP).
The replacement has to match the shape the built-in tool's own output
would have — for Bash that is the `BashToolResponse` object:

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PostToolUse",
    "updatedToolOutput": {
      "stdout": "<replacement text>",
      "stderr": "<kept verbatim>",
      "interrupted": false,
      "isImage": false
    }
  }
}
```

Verified end-to-end against the local `claude 2.1.299` with a 600-line
fixture (`seq 1 600`):

- Wrote the hook shim to `bash /tmp/cogito-trim-verify/trim.sh`, threshold
  200 lines (every real Bash call passes through unchanged), 60/20
  head/tail.
- Passed the settings file via `claude -p "run: cat /tmp/.../large.txt"
  --settings /tmp/.../settings.json --dangerously-skip-permissions
  --output-format json --include-hook-events`.
- The model itself confirmed the test output it received: it said "The
  file has 600 lines, but the `cat` output only showed lines 1–60 and
  581–600. The tool output itself was trimmed, and it says so:
  `[cogito-test trimmed 520 lines; re-run with a narrower command]`."
- Cost of the verification: $0.008, 1 cache write of 34,693 tokens, 4
  cache reads. The replacement reached the model verbatim.

Conclusion: object-shape replacement works. We do NOT need PreToolUse
rewriting of the user's command, which the brief forbids.

## Saving range (measured)

Walked the local transcripts in `~/.claude/projects/*.jsonl` (10
sessions, 1594 Bash tool_results). 30 calls (`~1.9%`) had stdout above
the 8000-char threshold.

The "re-sent" column multiplies the per-call saving by the number of
turns remaining in that session after the result appeared — that is
what `cache_read` charges each turn.

| | Tokens |
|---|---|
| First-turn saving total | 53,268 |
| Re-sent saving total | **50,350,344** |
| Per-session re-sent range | 3,136 → 39,779,509 |

One session accounted for 39.7 M of the 50.3 M tokens. The variance comes
from how long the session kept the long output in context — a 600-line
`git log` shown once near the start of a 47-turn session costs ~28 × 600 =
~16,800 tokens of cache reads; the same output at the start of a 1,400-
turn session costs ~1,399 × 600 = ~839,400 tokens.

The script that produced these numbers lives in the verification scratch
dir (`/tmp/cogito-trim-verify/measure_trim_savings.py`); the same logic
is the `token_sources.py` in `feat/d3-replay`, extended with the
"long-Bash outputs" question.

## Implemented

- `crates/cogito-harness/src/claude_post_tool_use.rs`:
  - `decide(event, enabled) -> Option<Value>` pure function. Reads
    `tool_response.{stdout,stderr,interrupted,isImage}` and returns the
    replacement JSON or `None`.
  - `run()` entry point (reads stdin, prints the JSON, exits 0).
  - Constants: `THRESHOLD_CHARS=8000`, `HEAD_KEEP=60`,
    `TAIL_KEEP=20`, `TEST_TAIL_KEEP=200`, marker copy with the
    `git log -n / sed -n / | tee` hint exactly as the brief asks.
  - Test suite (12 tests, all green): disabled passthrough, threshold
    passthrough, non-Bash passthrough, head/tail trim, unambiguous
    marker, test-summary tail, 200-line tail cap, replacement shape
    carries `stdout/stderr/interrupted/isImage`, interrupted→untouched,
    empty/malformed event does not panic, missing-`stdout` field leaves
    the result alone.
- `crates/cogito-harness/src/main.rs`: new `mod claude_post_tool_use;`
  and `claude-post-tool-use => claude_post_tool_use::run()` in the Hook
  dispatcher.
- `crates/cogito-harness/src/agents.rs`:
  - `CLAUDE_POST_TRIM_SCRIPT` constant (the `.sh` shim).
  - `output_trim_enabled()` env reader (1 / true / yes / on,
    case-insensitive).
  - `Agent::hooks()` appends the `PostToolUse` `HookSpec` only when
    `output_trim_enabled()` is true; `extra_files()` follows the same
    `hooks()` call so the `.sh` is only written when the entry is
    registered. `install` and `uninstall` rely on that list, so the
    script and entry round-trip together.
  - 4 new tests guarded by a process-wide `ENV_LOCK` (Rust 2024 marks
    `set_var` / `remove_var` unsafe, so the mutation is in `unsafe`
    blocks; the lock makes it actually safe): env parser
    (`"1"/"true"/"yes"/"on"/" TRUE "/"Yes"` enable, everything else
    doesn't), no-hook-when-off, registered-when-on, full round-trip
    `setup + uninstall`.

## Out of scope (deliberately)

- **Gentle Shell / Pi, opencode, Codex, Gemini CLI, Crush.** This is a
  Claude Code hook. The brief limits the palanca to one agent and one
  event; the other agents have their own tool-event models and we did
  not touch them.
- **`PostToolUseFailure`.** Errors reach that event, not `PostToolUse`;
  we don't see them today.
- **`updatedToolOutput` for non-Bash tools.** Spec only mentions long
  Bash outputs. Read/Grep/etc. still get the original short or long
  form. (Read in particular is already bounded by the PreToolUse
  guardrail.)
- **Tool-side wrapper / PreToolUse rewriting.** Forbidden by the brief.
- **`.cogito/config.toml` flag.** Env var is enough for v1. A future
  release can read `output_trim = true` from there too.

## Risks

- **`updatedToolOutput` semantics changed recently.** It used to be
  MCP-only. Our verification ran against `claude 2.1.299`. Older
  Claude Code versions may still require the old `updatedMCPToolOutput`
  shape — at that point the hook would JSON-parse fine but be ignored
  by Claude, which is a silent no-op (still safe: model sees the
  original).
- **Stale registration.** Running `setup` with the flag off does not
  clean up a PostToolUse entry that an earlier `setup` registered with
  the flag on. The registered-but-disabled hook is a no-op, not a
  silent strip — the model sees the original tool result. We chose to
  accept the stale entry rather than mutate `settings.json` differently
  based on a missing flag, which is hard to keep idempotent.
- **Mid-line long outputs.** A single 100 KiB line yields `line_count =
  1`; head/tail slicing keeps the whole thing; the hook returns
  `None` (skip). Acceptable: such outputs are rare, and the typical
  offender is `minified` files, where the model has no use for a tail.
- **Test runner false positives.** `passed` and `failed` are common
  English words inside command output. A 1,500-line log that happens
  to contain a lone `passed` will keep its last 200 lines. False
  positive rate is low enough that the cost of losing a real test
  summary is much worse than the savings on a false one.

## How to enable

```sh
# enable on the next setup run
COGITO_OUTPUT_TRIM=1 cogito setup claude

# (or any other truthy value: true / yes / on, case-insensitive)
COGITO_OUTPUT_TRIM=true cogito setup
```

The user can flip the env var at runtime without re-running setup — the
registered hook reads the same var on every call.
