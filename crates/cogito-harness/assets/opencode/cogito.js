// Cogito plugin for opencode.
//
// Refreshes the project's stored fingerprint when a session starts, so it
// matches the current code. Injects the project card into the system prompt
// and intercepts whole-file Read and plain-name Grep calls.
//
// The guardrail rule itself (outline a first whole read, turn away an
// unchanged repeat, nudge a plain-name grep) lives in `cogito guard`, the same
// entry point Claude Code and Pi use, so the three cannot drift apart. This
// plugin only passes one call as JSON and throws the reason it gets back.
//
// The MCP tools come from the `mcp.cogito` entry in opencode.json; this
// plugin handles session lifecycle and guardrails.

const COGITO_CARD_MARKER = "<cogito_project>";

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

  // --- Guardrail: let `cogito guard` decide ---

  const guard = async (call) => {
    try {
      const payload = JSON.stringify(call);
      const out = await $`printf '%s' ${payload} | cogito guard`
        .cwd(directory)
        .quiet()
        .nothrow();
      if (out.exitCode !== 0) return undefined;
      const answer = JSON.parse(String(out.stdout || "").trim() || "{}");
      if (answer && answer.block === true && answer.reason) return answer.reason;
      return undefined;
    } catch {
      // A guard that fails must never cost the agent a tool call.
      return undefined;
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

    const reason = await guard({
      tool: "read",
      input: { path: file_path, offset: args.offset, limit: args.limit },
      cwd: directory,
      session: input.sessionID,
    });
    if (reason) throw new Error(reason);
  };

  // --- Guardrail: nudge plain-name greps ---
  //
  // opencode signature same as above.
  // `args.pattern` is the search pattern; `args.path` and `args.glob` scope it.

  const grepNudge = async (input, output) => {
    if (input.tool !== "grep") return;

    const reason = await guard({
      tool: "grep",
      input: output.args || {},
      cwd: directory,
      session: input.sessionID,
    });
    if (reason) throw new Error(reason);
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
