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

## Interactive App Launch (CRITICAL — DO NOT SKIP)

To interact with the running app via Chrome DevTools MCP, launch with CDP port:

```bash
uv run python -c "
import subprocess, os, time
env = os.environ.copy()
env['WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS'] = '--remote-debugging-port=9222 --disable-http-cache --disable-cache'
try: subprocess.run(['taskkill', '/F', '/IM', 'filebitch.exe'], capture_output=True)
except: pass
time.sleep(1)
p = subprocess.Popen(['E:/projects/filebitch/target/debug/filebitch.exe'], env=env, creationflags=subprocess.DETACHED_PROCESS)
print(f'Launched PID: {p.pid}')
"
```

**Why Python?** `cmd /c set VAR=val && exe` does NOT propagate env vars to child processes. Only Python `subprocess.Popen` with explicit `env=` works reliably from the Bash tool.

**After launch:** wait ~5 seconds, then `chrome-devtools__list_pages` → `take_snapshot` → interact.

**NEVER** use `npx tauri dev` to launch for interactive work — it does NOT set the CDP port.
**NEVER** use `cmd /c start` with `set` — env vars don't propagate.
**NEVER** kill the app before connecting — connect to what's already running.

### Kill the App (CRITICAL)

When the app freezes or before `cargo build` to release the binary lock:

```bash
# Method 1: Escape slashes for Git Bash (preferred)
taskkill //F //IM filebitch.exe

# Method 2: Wrap in cmd /c
cmd /c taskkill /F /IM filebitch.exe
```

**Why `//` in Git Bash?** Git Bash interprets single `/FLAG` as a relative path. Double `//` escapes to a single `/` for the Windows command.

**DO NOT** try to close via Chrome DevTools MCP — the close button is outside the WebView.
**DO NOT** try `window.__TAURI__.window.getCurrentWindow().close()` — won't work if frozen.

### Windows File-Lock Gotcha

`cargo test` fails with "access denied" if `filebitch.exe` is running. **Run `cargo test` BEFORE `tauri dev`, not after.** The E2E script handles its own process cleanup.

### Windows Shell Gotcha

The Bash tool is **Git Bash** (POSIX sh) — it interprets `/FLAG` as a relative path. Two options:

```bash
# Option 1: Escape slashes
taskkill //F //IM filebitch.exe

# Option 2: Wrap in cmd /c
cmd /c taskkill /F /IM filebitch.exe
```

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
