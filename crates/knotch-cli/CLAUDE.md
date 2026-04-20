# knotch-cli

The reference `knotch` binary. Thin clap wrapper over
`knotch-agent` (hook surface) and the presets (read/write CLI
surface). Third-party harnesses build their own binary against
`knotch-agent` directly rather than extending this crate.

@../../.claude/rules/hook-integration.md
@../../.claude/rules/event-ownership.md
@../../.claude/rules/no-unsafe.md

## Subcommand layout

Each subcommand lives in `src/cmd/<name>.rs` with a `pub(crate)
struct Args` (clap derive) and a `pub(crate) async fn run(
config: &Config, out: OutputMode, args: Args) -> anyhow::Result<()>`.
Hook dispatch lives under `src/cmd/hook/<name>.rs`, one file per
Claude Code hook event.

`--json` and `--quiet` are global flags on the top-level `Cli`;
subcommand code receives `OutputMode` and respects it uniformly.

## Extension recipe — add a user-facing subcommand

1. Create `src/cmd/<name>.rs` with `Args` + `run`.
2. Register the module in `src/cmd/mod.rs`.
3. Add a variant to the `Command` enum in `src/main.rs` and the
   matching arm in the `runtime.block_on` dispatch.
4. If the subcommand writes events, route through a
   `knotch-agent` helper — never `Repository::append` directly
   (see @../../.claude/rules/event-ownership.md).

## Extension recipe — add a Claude Code hook

Follow `knotch-agent/CLAUDE.md` ("Extension recipe — add a hook"),
then wire the CLI side:

1. Create `src/cmd/hook/<name>.rs` with a `pub(crate) async fn
   run(config: &Config, input: HookInput) -> anyhow::Result<HookOutput>`
   that resolves the active unit and calls the matching
   `knotch-agent` function.
2. Register in `src/cmd/hook/mod.rs::HookCommand` and the dispatch
   match.
3. Add the hook entry to `init::knotch_hook_block()` **and**
   `plugins/knotch/hooks/hooks.json` (the plugin bundle).
4. Re-run `cargo xtask plugin-sync` after any skill edits.

## Do not

- Print anything to stdout from a `hook::*` command except the
  JSON output defined in the exit-code contract
  (@../../.claude/rules/hook-integration.md).
- Emit exit codes other than 0 or 2 from hook dispatch.
- Add a subcommand that calls `Repository::append` inline — the
  `knotch-agent` helpers are the only allowed write path.
