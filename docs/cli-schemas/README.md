# knotch CLI — machine-readable output reference

Every knotch subcommand accepts `--json`, which emits one JSON
object per line on stdout. Agent integrations should parse this
rather than scraping the human-readable format (which may change).

## Exit codes

| Code | Meaning | Retryable? |
|------|---------|------------|
| 0    | success | — |
| 1    | usage error (CLI arg invalid, unknown subcommand) | no — fix invocation |
| 2    | precondition violation (e.g. milestone already shipped) | yes, after resolving the precondition |
| 3    | transient I/O / lock / parsing failure | yes |
| 4    | fatal runtime error | no |

Hook subcommands use a stricter mapping — see
`.claude/rules/hook-integration.md`.

## Output schemas (per subcommand)

### `knotch init --json`

```json
{
  "event": "init",
  "root": "/path/to/project",
  "config": "/path/to/project/knotch.toml",
  "state_dir": "/path/to/project/state",
  "overwritten": false,
  "hooks_written": "/path/to/project/.claude/settings.json",
  "optional_example": "/path/to/project/.claude/knotch-optional-hooks.example.jsonc",
  "demo": false
}
```

### `knotch unit current --json`

```json
{"event": "unit_current", "slug": "signup-flow"}
```

When no active unit:

```json
{"event": "unit_current", "slug": null, "state": "uninitialized"}
```

### `knotch unit list --json`

```json
{"event": "unit_list", "units": ["signup-flow", "feature-x", "feature-y"]}
```

### `knotch show <unit> --format json` (or `--json show <unit>`)

```json
{
  "event": "show",
  "unit": "signup-flow",
  "current_phase": "build",
  "current_status": "in_progress",
  "shipped_milestones": ["add-login", "verify-email"],
  "events_recorded": 12
}
```

Null fields (`current_phase`, `current_status`) encode "no such
event recorded yet".

### `knotch mark|gate|transition --json`

```json
{
  "event": "phase_completed",
  "subject": "build",
  "accepted": 1,
  "rejected": []
}
```

`rejected` is an array of human-readable reasons (most common:
`"duplicate"` = idempotent replay success).

### `knotch log <unit> --json`

Emits a JSON array (not line-delimited) containing every event in
insertion order. Each element matches the `Event<W>` schema from
`knotch-kernel`.

### `knotch doctor --json`

```json
{
  "checks": [
    {"name": "root",         "status": "ok",   "detail": "/path (dir)"},
    {"name": "state_dir",    "status": "ok",   "detail": "/path/state (dir)"},
    {"name": "knotch.toml",  "status": "ok",   "detail": "/path/knotch.toml parses"},
    {"name": "units",        "status": "ok",   "detail": "3 healthy"},
    {"name": ".gitignore",   "status": "ok",   "detail": "contains .knotch/"},
    {"name": "queue",        "status": "ok",   "detail": "empty"},
    {"name": "secret scan",  "status": "warn", "detail": "no scanner in .git/hooks/pre-commit — ..."},
    {"name": "agent env",    "status": "ok",   "detail": "KNOTCH_MODEL=claude-opus-4-7 KNOTCH_HARNESS=claude-code/2.1"}
  ],
  "ok": true
}
```

Exit code 0 when every check is `ok`/`warn`, non-zero on any `fail`.

## Hooks output

Hook subcommands (`knotch hook <event>`) emit Claude Code hook wire
format directly — see the Claude Code
[hooks reference](https://code.claude.com/docs/en/hooks) for the
exact envelope. Three outcomes: exit 0 empty (Continue), exit 0 JSON
with `hookSpecificOutput.additionalContext` (Context), exit 2 stderr
(Block). Exit codes 1 / 3+ are never emitted by hook subcommands.
