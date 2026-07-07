# SDLC — FileBitch Development Workflow

## Quick Iterate Loop (Day-to-Day)

1. App is running with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222`
2. `chrome-devtools__list_pages` → find the FileBitch page
3. `chrome-devtools__take_snapshot` → full DOM with uids
4. Interact: `click`, `fill`, `screenshot`, `evaluate_script`
5. Make a code change → `tauri dev` hot-reloads → take another snapshot to verify

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
