# Codex CLI integration

The preset runs `codex exec --json --sandbox workspace-write`, reading the task
prompt from stdin in the registered repository. It reuses the user's CLI login
and model configuration. Review slots can use `exec --json --sandbox read-only`.
Slot arguments replace the entire argument list.

The adapter consumes completed agent messages without JSON escaping so
`[EASYVIBE-RESULT]` and `[EASYVIBE-REVIEW]` remain readable by task governance.
Reasoning and command/file/tool summaries are displayed, but tool output cannot
become a task result. Updated messages are not replayed into the result buffer.

Success requires both process exit 0 and a `turn.completed` event, with no
`turn.failed`. Recoverable `error` events are visible and do not independently
fail a recovered turn. Stop and timeout retain the shared session behavior.
Token counts come from `turn.completed.usage`; missing model, price, duration,
and cache-write counts are not invented. Codex is still marked experimental.

Connection probes override write/bypass flags with a read-only sandbox and add
`--skip-git-repo-check` for the temporary directory. Their timeout is 120 seconds
to allow connection retries and transport fallback.

## Automated checks

From `easyvibe-backend`:

```sh
cargo test -p easyvibe-session
cargo test -p easyvibe-db
cargo test -p easyvibe-app agent_conf::tests
```

The session fixtures run with PowerShell on Windows and `sh` on Unix. Existing
session tests require `echo`, `true`, `false`, and `sleep`; on Windows add Git's
`usr/bin` directory to PATH. The existing agent configuration version test uses
`/bin/echo`, so skip `probe_version_echo_and_missing` on Windows.

Application builds currently require five ignored `reference/` Harness source
files: `manifest.json`, `inject-prompt.md`, `rule_development.md`,
`rule_bugfix.md`, and `grill-me/SKILL.md`. Obtain the official originals from the
maintainer. For this local validation, the installed `~/.easyvibe/harness/`
copies were supplied as build inputs; they are not part of this change.

## Live check (uses a model call)

With an installed, authenticated CLI:

```sh
cargo test -p easyvibe-session codex_live_workspace_write_and_result -- --ignored --nocapture
```

Set `EASYVIBE_TEST_CODEX_CMD` if Codex is not on PATH. This check initializes an
isolated temporary Git repository, invokes Codex through `SessionManager`, and
verifies a written file, captured result marker, successful status, and token
metadata. It removes its temporary repository after the process ends.

Official event and sandbox contract:
[Codex non-interactive mode](https://learn.chatgpt.com/docs/non-interactive-mode).

## Validation recorded on 2026-10-05

Windows, Codex CLI 0.159.2:

- Session suite: 19 passed; the live check is normally ignored and was run separately.
- Database suite: 11 passed.
- Agent configuration suite: 4 passed; the pre-existing `/bin/echo` version probe was excluded on Windows.
- Live session: passed file creation, task-result capture, successful status, and token metadata checks.
- Renderer: 64 tests passed, TypeScript check passed, production build passed.
- Backend application build passed using the local Harness inputs described above.

The renderer startup test now imports the checked-in Hover map fixture instead
of depending on the maintainer's absolute home-directory path. Full application
tests and macOS validation were not run. Existing compiler and bundle-size
warnings remain outside this integration change.
