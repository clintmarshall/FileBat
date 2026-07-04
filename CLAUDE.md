# FileBitch — Project Instructions

## Architecture Document

**`architecture.md` MUST be kept up to date as the project evolves.**

When doing any of the following, update the relevant section of `architecture.md` **before** finishing the task:

- Adding a new Tauri command → update the Commands table
- Adding a new dependency → update the Dependencies table
- Adding a new domain model → update the Domain Models section
- Changing the layer structure → update the Source Layout and Architecture diagrams
- Introducing a new design decision → add to Key Design Decisions
- Implementing a Future Consideration → move it to the main architecture and note the approach

If the architecture document drifts from reality, the project becomes unmaintainable. Treat it as living documentation, not a one-time artifact.

## Build Commands

```bash
# Dev (two terminals)
npx vite              # Terminal 1 — compiles TS, serves on :1420
npx tauri dev         # Terminal 2 — connects to Vite, launches window

# Production
npx tauri build
```

## Developer Workflow

### Order of Operations (follow this every time)

1. **Write failing tests first** (TDD). No implementation code without a test that fails.
2. **Run tests** — all three layers must pass:
   ```bash
   npm test                      # Frontend (vitest + jsdom)
   cd src-tauri && cargo test    # Rust unit + integration
   npm run test:e2e              # Real app — Playwright + WebView2 CDP
   ```
3. **Start the dev server** — leave it running:
   ```bash
   npx vite       # Terminal 1 (already running on :1420)
   npx tauri dev  # Terminal 2 — hot-reloads Rust changes automatically
   ```
4. **Make your changes** — Tauri recompiles and restarts the window automatically.
5. **Run `npm run test:e2e`** to verify in the real app — no asking the user.
6. **Update architecture.md** if the change affects structure, commands, or dependencies.

### E2E Tests (`playwright.tauri.cjs`)

Launches the compiled `filebitch.exe` with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222`, connects via Playwright CDP, and runs assertions against the real app. Checks:
- Drives appear in sidebar
- 📊 toggles Analytics panel
- Scan path is pre-populated

**Prerequisite:** `cargo build` must have run first (binary at `target/debug/filebitch.exe`).

### Windows File-Lock Gotcha

`cargo test` fails with "access denied" if `filebitch.exe` is running. **Run `cargo test` BEFORE `tauri dev`, not after.** The E2E script handles its own process cleanup.

### Windows Shell Gotcha

The Bash tool is **Git Bash** (POSIX sh) — it interprets `/FLAG` as a relative path. For Windows commands with `/FLAG` syntax, wrap in `cmd /c`:

```bash
# WRONG — Git Bash treats /PID as a path
taskkill /PID 1234 /F /T

# RIGHT — runs via cmd.exe
cmd /c taskkill /PID 1234 /F /T
```

To find the PID on CDP port 9222: `netstat -ano | findstr "9222"`

**E2E teardown:** The E2E script uses `taskkill /F /IM filebitch.exe` (image name, not PID) for cleanup. Never track dynamic PIDs — kill by image name.

### No Exceptions - Definition of Done

A change is not "done" until:
- Tests pass (all three layers)
- The E2E test passes
- The behavior matches the requirement
- fallow details captured - see fallow-progress.md for instructions when required

## Layer Rules

- **Domain** knows nothing about Tauri, std::fs, or the UI
- **Infrastructure** implements Domain traits using OS/database primitives
- **UseCases** orchestrate repos + apply business rules (sorting, filtering)
- **Commands** are thin IPC bridges — no business logic
- New code follows this pattern; do not bypass layers

### Development workflow

- Assess required change based on task or prompt
- If trivial move to next step, if more complex create plan with tracked tasks
- make change
- see No Exceptions - Definition of Done
