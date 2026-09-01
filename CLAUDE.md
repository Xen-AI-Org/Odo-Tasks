# CLAUDE.md

> 글로벌 지침(`~/.claude/CLAUDE.md`) 상속: 사고규율(Before/After Acting), 행동규율(Break/Cross/Ground), 환경식별, 호칭, Python/uv 규칙 등.

## Project Rules

<!-- 대화 중 발견된 프로젝트 규칙이 여기에 추가됩니다. -->

### Build / verification notes

- Run `npm run check` to type-check and build the frontend with `vite build` and run `cargo check`.
- Run `cargo build --manifest-path src-tauri/Cargo.toml` to compile the full Tauri binary.
- Run `cargo test --no-run --manifest-path src-tauri/Cargo.toml` to confirm tests compile. Running the test binary in a headless environment may fail if the WebView2 runtime / GUI libraries are not available (`STATUS_ENTRYPOINT_NOT_FOUND`); this is usually an environment issue, not a code issue.
- `Cargo.toml` no longer includes `cdylib` in `[lib] crate-type`. This avoids the 65,535 PE export-ordinal limit on `x86_64-pc-windows-gnu`. Add it back if you need to build for Android.
- The desktop app uses `keyring` (account `ai-api-key`) to store the AI provider API key. Legacy `openai-api-key` is read as a fallback for OpenAI provider only. Do not write keys to source, env files, or logs.
- Environment overrides for AI (for dev/testing only): `OPENROUTER_API_KEY`, `OPENAI_API_KEY`, `ODO_AI_PROVIDER`, `ODO_AI_MODEL`, `ODO_AI_BASE_URL`. If an env key is active, the UI shows "Set via environment variable" and it is never echoed in logs.
- Use `chrono` without the `clock` feature; parse RFC 3339 timestamps from `mcp::now_iso()` when you need current time.
