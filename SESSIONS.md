# Session Log

## 2026-07-03 — Fallow Health Sweep

**Goal:** Run fallow, remove noise, refactor complexity, set up git

### Refactoring
- Extracted keyboard handler (CRAP 172 → 26) into 5 focused functions
- Extracted context menu helpers (`buildContextMenuItem`, `clampToViewport`)
- Extracted `renderEntries` row creation (`createFileRow`, `bindRowEvents`)
- Extracted `startRename` into `createRenameInput` + `setupRenameEvents`
- Deduplicated Tauri IPC tests with `bootAndScan` + `getInvokeCall` helpers

### Architecture
- Created `src/utils.ts` — shared `formatSize`, `formatDate`, `entryIcon`
- Created `src/test/helpers.ts` — shared test setup (`createDom`, `bootApp`, etc.)

### Tests
- Added 6 keyboard navigation tests (Ctrl+A, ArrowDown, ArrowUp, Ctrl+C, Ctrl+X, Delete)
- Refactored `app.test.ts` to import from `utils.ts` instead of copy-pasting

### Tooling
- Created `fallow.toml` — excluded build artifacts, ignored test helper exports
- Created `fallow-progress.md` — health progression table with journey tracking
- Created `fallow-chart.html` — visual charts (MI, CRAP, dead files, duplication)
- Added `playwright` to devDependencies
- Set up `.gitignore`, initialized git, pushed to GitHub

### Documentation
- Added fallow workflow reference to `CLAUDE.md`

**Metrics:**
| MI | Max CRAP (prod) | Dup % | Clone Groups | Tests |
|----|----------------|-------|--------------|-------|
| 79.1 → 93.9 | 172 → 26 | 9.8% → 1.7% | 11 → 2 | 39 → 45 |

**Decisions:**
- Compare chart cells against previous row (not baseline) — more signal
- Only colour numeric columns — date and notes stay neutral
- `playwright.tauri.cjs` is e2e, not prod — skip its CRAP score
- Per-cell colouring (green/red) instead of whole-row — mixed signals per row

**Left for next time:**
- Run e2e tests (`npm run test:e2e`)
- Consider `handleActionKeys` CRAP 26 (acceptable for now)
- 2 remaining clone groups (18 lines of test boilerplate)

**Files touched:** `app.ts`, `app.test.ts`, `app.integration.test.ts`, `keyboard.test.ts`, `utils.ts`, `test/helpers.ts`, `fallow.toml`, `fallow-progress.md`, `fallow-chart.html`, `package.json`, `CLAUDE.md`, `.gitignore`
