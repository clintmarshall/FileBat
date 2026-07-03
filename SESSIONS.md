# Session Log

## 2026-07-03 — Fallow Health Sweep, Refactoring, Tauri IPC Standardisation

**Goal:** Run fallow, remove noise, refactor complexity, standardise Tauri IPC casing, set up git

### Refactoring
- Extracted keyboard handler (CRAP 172 → 26) into 5 focused functions
- Extracted context menu helpers (`buildContextMenuItem`, `clampToViewport`)
- Extracted `renderEntries` row creation (`createFileRow`, `bindRowEvents`)
- Extracted `startRename` into `createRenameInput` + `setupRenameEvents`
- Deduplicated Tauri IPC tests with `bootAndScan` + `getInvokeCall` helpers

### Architecture
- Created `src/utils.ts` — shared `formatSize`, `formatDate`, `entryIcon`
- Created `src/test/helpers.ts` — shared test setup (`createDom`, `bootApp`, etc.)
- Added `#[serde(rename_all = "camelCase")]` to all 9 Tauri boundary structs:
  `Entry`, `Volume`, `FolderUsage`, `DuplicateGroup`, `UsageSnapshot`,
  `ScanProgress`, `ScanChunk`, `ScanError`, `ScanComplete`
- Frontend now uses camelCase everywhere — one convention for all Tauri IPC

### Tests
- Added 6 keyboard navigation tests (Ctrl+A, ArrowDown, ArrowUp, Ctrl+C, Ctrl+X, Delete)
- Refactored `app.test.ts` to import from `utils.ts` instead of copy-pasting
- Updated Rust serialization tests to expect camelCase keys
- Updated integration tests to use camelCase invoke args and event payloads

### Tooling
- Created `fallow.toml` — excluded build artifacts, ignored test helper exports
- Created `fallow-progress.md` — health progression table with journey tracking
- Created `fallow-chart.html` — visual charts (MI, CRAP, dead files, duplication)
- Added `playwright` to devDependencies
- Set up `.gitignore`, initialized git, pushed to GitHub

### E2E
- Ran e2e tests — drives, analytics toggle, scan path all pass
- Scan starts correctly (camelCase args work) but `scan:complete` event never arrives
- Known issue: spawn_blocking/channel event emission (separate from camelCase)

### Documentation
- Added fallow workflow reference to `CLAUDE.md`
- Created `SESSIONS.md` with categorized entries

**Metrics:**
| MI | Max CRAP (prod) | Dup % | Clone Groups | Tests |
|----|----------------|-------|--------------|-------|
| 79.1 → 93.9 | 172 → 26 | 9.8% → 1.7% | 11 → 2 | 39 → 45 |

**Decisions:**
- Compare chart cells against previous row (not baseline) — more signal
- Only colour numeric columns — date and notes stay neutral
- `playwright.tauri.cjs` is e2e, not prod — skip its CRAP score
- Per-cell colouring (green/red) instead of whole-row — mixed signals per row
- `#[serde(rename_all = "camelCase")]` is the standard for all structs crossing the Tauri boundary
- New structs added in future should follow this pattern automatically

**Left for next time:**
- Fix scan events not reaching frontend (spawn_blocking/channel issue)
- Run e2e scan test to completion
- Consider `handleActionKeys` CRAP 26 (acceptable for now)
- 2 remaining clone groups (18 lines of test boilerplate)

**Files touched:** `app.ts`, `app.test.ts`, `app.integration.test.ts`, `keyboard.test.ts`, `utils.ts`, `test/helpers.ts`, `fallow.toml`, `fallow-progress.md`, `fallow-chart.html`, `package.json`, `CLAUDE.md`, `.gitignore`, `src-tauri/src/domain/models.rs`

**Commits:**
| Hash | Message |
|------|---------|
| `aab0754` | Initial commit — FileBitch file manager with fallow health tracking |
| `a0f9c7e` | Finish fallow refactorings — MI 93.9, dup 1.7%, 2 clone groups |
| `3a5f59f` | Add session log |
| `a211f28` | Categorize session log entries |
| `24a3c08` | Add #[serde(rename_all = "camelCase")] to all Tauri boundary structs |
| `0161a9e` | Update session log with camelCase work |
| `01b5e7d` | Update fallow progress with camelCase row |
