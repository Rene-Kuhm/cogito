// Cogito plugin for opencode.
//
// Refreshes the project's stored fingerprint when a session starts, so it
// matches the current code. Injects the project card into the system prompt
// and intercepts the first whole-file Read and plain-name Grep calls,
// returning Cogito's outline or search result instead.
//
// The MCP tools come from the `mcp.cogito` entry in opencode.json; this
// plugin handles session lifecycle and guardrails.
//
// Guardrail thresholds are shared with Claude Code:
//   OUTLINE_MIN_LINES = 250  (files shorter are read whole)
//   WHOLE_READ_LIMIT = 400   (reads with limit > 400 count as whole)

const OUTLINE_MIN_LINES = 250;
const WHOLE_READ_LIMIT = 400;
const COGITO_CARD_MARKER = "<cogito_project>";
const COGITO_NUDGE_DIR = ".cogito/nudges";

export const CogitoPlugin = async ({ directory, $ }) => {
  const isProject = async () => {
    try {
      // `-e`, not `-d`: in a git worktree or submodule `.git` is a file.
      return (
        (await $`test -e .git`.cwd(directory).quiet().nothrow()).exitCode === 0 ||
        (await $`test -d .cogito`.cwd(directory).quiet().nothrow()).exitCode === 0
      );
    } catch {
      return false;
    }
  };

  // --- Session lifecycle: scan ---

  const scan = async () => {
    try {
      if (!(await isProject())) return;
      await $`cogito scan --quiet`.cwd(directory).quiet().nothrow();
    } catch {
      // Cogito missing or the scan failed: never block the session.
    }
  };

  // --- Session lifecycle: inject card ---
  //
  // opencode signature:
  //   (input: { sessionID?: string; model: Model }, output: { system: string[] })

  const injectCard = async (input, output) => {
    try {
      if (!(await isProject())) return;
      const card = (await $`cogito map --card`
        .cwd(directory)
        .quiet()
        .nothrow()).stdout.trim();
      if (!card) return;
      // Guard: push once per session (check whether the marker is already in output.system)
      if (output?.system?.some((line) => line.includes(COGITO_CARD_MARKER))) return;
      output.system = output.system || [];
      output.system.push(card);
    } catch {
      // Never break a request.
    }
  };

  // --- Guardrail: outline large reads ---
  //
  // opencode signature:
  //   (input: { tool: string; sessionID: string; callID: string },
  //    output: { args: any })
  //
  // The `args` object may have `filePath` or `file_path`, `offset`, and `limit`
  // for the Read tool. Accept either path key.

  const readNudge = async (input, output) => {
    if (input.tool !== "read") return;
    const args = output.args || {};
    const file_path = args.filePath ?? args.file_path;
    if (!file_path) return;

    // Skip paths outside the project
    if (!file_path.startsWith(directory)) return;

    const offset = args.offset;
    const limit = args.limit;
    // "whole" means no offset and no limit, or a limit above WHOLE_READ_LIMIT
    const whole =
      (offset === null || offset === undefined) &&
      (limit === undefined || limit === null || limit > WHOLE_READ_LIMIT);
    if (!whole) return;

    try {
      const content = (await $`cat "${file_path}"`.cwd(directory).quiet().nothrow()).stdout;
      const lines = content.split("\n").length;
      if (lines < OUTLINE_MIN_LINES) return;

      const rel = file_path.slice(directory.length + 1);
      const outline = (await $`cogito outline "${rel}"`
        .cwd(directory)
        .quiet()
        .nothrow()).stdout.trim();

      if (!outline) return;

      const key = `read:${rel}`;
      const nudged = await firstTime(key, input.sessionID);
      if (!nudged) return;

      const msg = `${rel} has ${lines} lines. Cogito's outline instead of the whole file:\n\n${outline}\nRead the range you need with offset/limit. Reading the whole file again is allowed if you really need it.`;
      throw new Error(msg);
    } catch (e) {
      if (e.message && e.message.includes("Cogito's outline")) {
        throw e; // re-throw the nudge
      }
      // Swallow: let the call through on any failure.
    }
  };

  // --- Guardrail: nudge plain-name greps ---
  //
  // opencode signature same as above.
  // `args.pattern` is the search pattern; `args.path` and `args.glob` scope it.

  const isPlainName = (pattern) => {
    const p = pattern.trim();
    if (p.length < 2 || p.length > 80) return false;
    return /^[\w.\-\/ ]+$/.test(p);
  };

  const grepNudge = async (input, output) => {
    if (input.tool !== "grep") return;

    const args = output.args || {};
    const pattern = args.pattern;
    if (!pattern || !isPlainName(pattern)) return;

    try {
      const result = (await $`cogito search "${pattern}"`
        .cwd(directory)
        .quiet()
        .nothrow()).stdout.trim();

      if (!result) return;

      const key = `grep:${pattern}:${args.path || ""}:${args.glob || ""}`;
      const nudged = await firstTime(key, input.sessionID);
      if (!nudged) return;

      const msg = `Cogito's index for "${pattern}" (symbols and doc sections with file:line, files, modules):\n\n${result}\nUse file_outline on a result and read only its range. If you need every textual match, repeat the same Grep call and it will run.`;
      throw new Error(msg);
    } catch (e) {
      if (e.message && e.message.includes("Cogito's index")) {
        throw e;
      }
      // Swallow: let the call through on any failure.
    }
  };

  // --- Per-session bookkeeping: record key, return true only the first time ---

  const firstTime = async (key, sessionId) => {
    try {
      const safeId = String(sessionId || "default")
        .replace(/[^a-zA-Z0-9\-]/g, "")
        .slice(0, 64);
      const nudgeDir = `${directory}/${COGITO_NUDGE_DIR}`;
      const nudgeFile = `${nudgeDir}/${safeId || "default"}`;

      const seen = (await $`cat "${nudgeFile}" 2>/dev/null`.cwd(directory).quiet().nothrow()).stdout || "";
      const lines = seen.split("\n");
      if (lines.includes(key)) return false;

      await $`mkdir -p "${nudgeDir}"`.cwd(directory).quiet().nothrow();
      await $`cat >> "${nudgeFile}" << 'EOF'\n${key}\nEOF`
        .cwd(directory)
        .quiet()
        .nothrow();
      return true;
    } catch {
      return false;
    }
  };

  return {
    event: async ({ event }) => {
      if (event?.type === "session.created" || event?.type === "session.start") {
        void scan();
      }
    },
    "experimental.chat.system.transform": async (input, output) => {
      await injectCard(input, output);
    },
    "tool.execute.before": async (input, output) => {
      await readNudge(input, output);
      await grepNudge(input, output);
    },
  };
};

export default CogitoPlugin;
