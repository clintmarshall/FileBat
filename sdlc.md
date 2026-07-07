# SDLC — FileBitch Development Workflow

## Launch the App (with CDP)

From the Bash tool (Git Bash), use Python to set the env var correctly:

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

**Why Python?** `cmd /c set VAR=val && exe` doesn't propagate env vars to child processes reliably. Node.js `spawn` (used in E2E) works but requires the E2E script. Python `subprocess.Popen` with explicit `env=` is the most reliable from the Bash tool.

## Quick Iterate Loop (Day-to-Day)

1. Launch the app (see above)
2. `chrome-devtools__list_pages` → find the FileBitch page
3. `chrome-devtools__take_snapshot` → full DOM with uids
4. Interact: `click`, `fill`, `screenshot`, `evaluate_script`
5. Make a code change → rebuild with `cargo build` → relaunch → take another snapshot to verify

**DO NOT kill the app before connecting.** The Chrome DevTools MCP connects to the already-running app.

**DO NOT run the E2E script** for interactive verification — use the Chrome DevTools MCP directly. The E2E script is for automated verification and kills the app on teardown.

## Automated Verification

```bash
npm test                      # Frontend unit tests (vitest + jsdom)
cd src-tauri && cargo test    # Rust unit + integration tests
npm run test:e2e              # Real app — Playwright + WebView2 CDP
```

### Windows Gotcha

`cargo test` fails with "access denied" if `filebitch.exe` is running. **Run `cargo test` BEFORE `tauri dev`, not after.**

## Definition of Done

- Tests pass (all three layers)
- E2E test passes
- Behavior matches the requirement
- `architecture.md` updated if structure changed

## Interaction & Error Recovery Protocol

If any `chrome-devtools__*` interaction fails, times out, or triggers an error, **DO NOT STOP.** You must execute the following diagnostic loop immediately:

### State 1: "Element Not Found" or Timeout

* **Reason:** The DOM may have hot-reloaded mid-execution, wiping out your target UID/selector.
- **Immediate Fix:** Run `chrome-devtools__list_pages` to confirm the target websocket target ID hasn't rotated. Then, take a fresh `chrome-devtools__take_snapshot` to fetch the updated DOM state. Do not guess selectors from your memory code base.

### State 2: "Target Closed" or "Cannot Connect"

* **Reason:** The Tauri Rust backend or Vite frontend crashed/panicked under the hood during a file-system operation.
- **Immediate Fix:** Check your system terminal process logs. If the process is dead, run `npx tauri dev` to bring the container frame back up, wait 5 seconds, and re-run `chrome-devtools__list_pages`.

### State 3: Click executes but "Nothing Changes"

* **Reason:** The browser engine inside WebView2 received the event, but a frontend state error (e.g., an unhandled promise rejection in React/Vue/Svelte) froze the UI thread.
- **Immediate Fix:** Run `chrome-devtools__evaluate_script` with `console.error` listeners or check window logs to extract the runtime trace. Fix the frontend bug in your source code, let the application hot-reload, and attempt the interaction loop again.

**CRITICAL MANDATE:** You have full authorization to alter code files to fix runtime exceptions uncovered during live interaction. "I cannot do that" is an invalid response while the app environment is running.

## GitHub CLI Authentication

The `GH_TOKEN` environment variable is set but **invalid**. Before running any `gh` commands, unset it to use the keyring token:

```bash
unset GH_TOKEN; gh <command>
```

Without `unset GH_TOKEN`, `gh` will use the broken token and fail with "HTTP 401: Bad credentials". The keyring token (`gh auth status` shows `clintmarshall`) works fine.
