# Session Log

## 2026-07-05 — BFS Streaming Scan + Full Tree Drilldown

**Goal:** Enable tree drilldown at any depth. Replace O(depth × files) redundant walks with leaf-only sizing + rollup.

### What Was Done
- Rewrote `disk_usage.rs` as BFS streaming loop:
  - Phase 1: BFS discovers folders in batches of 200, emits `scan:structure` after each batch
  - Phase 2: Leaf folders sized in parallel (10 threads) as discovered
  - Phase 3: Rollup propagates totals bottom-up after discovery completes
- Frontend: removed depth selector, cleaned debug console.logs
- 7 Rust unit tests for `readdir_children`, `size_folder`
- All 54 Rust tests pass, 135 frontend tests pass, E2E passes

### Crash — Memory Allocation Failure
Scanning E: drive crashed with `memory allocation of 16777216 bytes failed` / `STATUS_STACK_BUFFER_OVERRUN`.

**Root cause:** Every `scan:structure` event emits the ENTIRE accumulated folder list. On a large drive with thousands of folders, each batch sends a growing JSON payload. The 16MB allocation failure is likely the IPC bridge trying to serialize a massive folder list.

**Fix needed:** Don't emit the full list every time. Instead:
- Emit the full list once (initial structure)
- Emit incremental deltas (`scan:subtree` with just the new batch)
- OR emit the full list but cap batch size and throttle emissions

### Remaining Work

**1. Top-level rollup first (priority)**
- The UI should show just the top level initially with rolled-up data
- As children are discovered and sized, patch the top-level rows
- The tree should NOT re-render on every batch — just fill in stats for existing rows
- Only emit new rows when the user expands a folder

**2. Fix the crash**
- Reduce IPC payload size — don't send the entire folder tree every batch
- Consider incremental structure events instead of full re-renders

**3. Progress bar**
- Currently only updates at 100%. Add incremental progress per completed folder.

**4. Cancel between phases**
- Test cancel during structure phase vs sizing phase

### Branch
`feature/tree-drilldown` — 3 commits

---

## 2026-07-04 — Disk Usage Tree + Two-Phase Parallel Scan

**Goal:** Replace flat table with tree view, then overhaul the scan to be two-phase with parallel sizing.

### Tree View (Frontend)
- Added `UsageTreeNode` interface, `expandedPaths` state, `buildUsageTree()` function
- Replaced `renderUsageResults()` with `renderUsageTree()` — CSS Grid with 5 aligned columns
- Tree rows: toggle ▶/▼ + 📁 + name + size + bar + files + folders
- Click to expand/collapse, auto-expand on first render
- Added depth selector (1–10 levels, "All") next to scan path
- Created `src/usage-tree.test.ts` — 10 unit tests for tree building

### Two-Phase Parallel Scan (Rust)
- **Phase 1 — Structure:** Shallow walk (max_depth=1) discovers folder hierarchy, emits `scan:structure` event with tree shape. Frontend renders skeleton instantly with "—" placeholders.
- **Phase 2 — Sizing:** 10 worker threads pull folders from a work queue. Each thread walks one folder, counts files/subfolders, emits `scan:chunk`. Frontend patches individual rows by `data-path`.
- Progress = completed / total folders (exact, not estimated).
- Removed `max_depth` limit — scans everything now.
- Removed `depth` field from `FolderUsage` model.
- Added `FolderStructure` and `ScanStructure` models.

### Frontend Event Handlers
- `scan:structure` → `renderUsageTreeSkeleton()` — builds tree from Rust's structure data
- `scan:chunk` → `patchUsageRow()` — finds row by `data-path`, updates size/bar/files/folders cells
- Removed `buildUsageTree()` — frontend no longer builds tree from flat paths
- `patchUsageRow` handles path format mismatches (backslash vs forward slash, trailing slash)

### Bugs Fixed
- Drive root name was empty (`"E:/"` → `split('/')` → `['', '']` → `pop()` → `''`)
- Orphan folders rendered at depth 0 (parent not in map → became root)
- `CSS.escape` undefined in jsdom test environment
- Self-matching parent for drive roots (`"C:/"` was its own parent)

### Test Updates
- Updated `app.integration.test.ts` to emit `scan:structure` before `scan:chunk`
- Updated `src/test/helpers.ts` DOM to include `<select id="scan-depth">`
- Updated `playwright.tauri.cjs` to check `.usage-tree-row` instead of `.analytics-table tr`
- All 135 frontend tests pass, 40 Rust unit tests pass, E2E passes

### Current State
- Tree renders instantly on scan start (structure phase)
- Stats fill in as threads complete (chunk phase)
- Scanning `E:\` (95 folders, 3.5M files, 5.3TB) completes in ~22 seconds
- **Unverified:** Whether the stats filling in correctly — user needs to visually confirm
- **Unverified:** Whether parallel sizing produces correct totals vs sequential walk

### Files Changed
- `src/app.ts` — tree types, skeleton renderer, row patcher, depth selector, new event listeners
- `src/styles/main.css` — tree CSS (grid layout, toggle icons, indentation, children guide line)
- `src/index.html` — depth selector `<select>`
- `src/usage-tree.test.ts` — 10 tree-building unit tests (NEW)
- `src/test/helpers.ts` — added scan-depth select to DOM
- `src/app.integration.test.ts` — updated to emit scan:structure
- `playwright.tauri.cjs` — updated selector for tree rows
- `src-tauri/src/domain/models.rs` — removed `depth` from `FolderUsage`, added `FolderStructure`, `ScanStructure`
- `src-tauri/src/usecases/analytics/disk_usage.rs` — complete rewrite: two-phase, 10-thread parallel sizing
- `src-tauri/src/usecases/analytics/aggregator.rs` — removed `depth` field from `FolderUsage` construction (still used by duplicates)

### Left for Next Time
- **Verify data correctness** — compare parallel scan totals against sequential walk
- **Progress bar** — currently only updates at 100%. Add incremental progress updates per completed folder.
- **Cancel between phases** — test cancel during structure phase vs sizing phase
- **Chunking large folders** — if one folder has 100k subfolders, one thread chews it all. Future: split large folders into sub-tasks.
- **Remove dead code** — `FolderUsageAccumulator` is no longer used by disk_usage but still exists (used by aggregator tests). Clean up when confident.
- **Remove debug logging** — `console.log` calls in `renderUsageTreeSkeleton` and `patchUsageRow`

## 2026-07-04 — Fallow OOM Fix, Test Dedup, E2E Refactor, Chart Overhaul

**Goal:** Fix fallow running out of memory, eliminate test duplication, refactor E2E script, improve chart readability

### Fallow OOM Fix
- Fallow was parsing 1.5M of HTML coverage reports as source code → OOM
- Created `.fallowrc.json` with top-level `ignorePatterns` (coverage/, html/, frontend-dist/, target/, *.old, *.lock)
- Updated `.gitignore` to exclude generated artifacts (coverage/, html/, *.old)
- Dropped `fallow.toml` — consolidated all config into `.fallowrc.json`

### Test Factory Extraction
- Extracted 5 factories into `helpers.ts`: `selectFirstRow`, `startRename`, `openContextMenu`, `openGlobalContextMenu`, `dispatchKey`
- Refactored `app.unit.test.ts`: 1408 → 1166 LOC
- Duplication: 17.2% (675 lines, 10 clone groups) → 0%

### E2E Script Refactor
- Split 259-line IIFE in `playwright.tauri.cjs` into 10 named functions
- CRAP: 306 → 56 (cyclomatic 17→7, cognitive 23→8)
- Functions: `cleanupWebViewCache`, `killStaleProcesses`, `launchApp`, `connectBrowser`, `waitForAppReady`, `setupPageOverrides`, `setupConsoleLogging`, `testAppInit`, `testFileList`, `testAnalyticsToggle`, `testScanPath`, `testDiskUsageScan`

### Chart Overhaul
- All lines green by default, segments turn red when thresholds breached (MI < 85, CRAP ≥ 30, dead > 0, dup > 0)
- Test code lines rendered dotted
- Split CRAP tracking: CRAP (app) vs CRAP (test) columns in table and chart
- Labels below points to prevent overflow

### Stash Recovery
- Recovered stashed WIP from `fix/scan-events` branch (CLAUDE.md workflow, coverage deps, expanded integration tests)
- Merged main into branch, resolved conflicts in progress tracking files
- Created PR #2, merged to main

### Running the App
```bash
# Terminal 1 — Vite dev server (compiles TS, serves on :1420)
npx vite

# Terminal 2 — Tauri dev (connects to Vite, launches window)
npx tauri dev
```

**Metrics:**
| MI | CRAP (app) | CRAP (test) | Dup % | Dead Files | Dead Exports |
|----|-----------|-------------|-------|------------|-------------|
| 92.2 → 93.1 | 43 → 10.2 | 306 → 56 | 17.2% → 0% | 6.7% → 0% | 20% → 0% |

**Decisions:**
- Green-by-default chart lines, red reserved for threshold breaches
- Test code lines dotted to distinguish from app code
- Split CRAP tracking (app vs test) — test code obscures real code signal
- `ignorePatterns` at top-level in `.fallowrc.json` (not just under `duplicates`)
- Consolidate fallow config to single file (`.fallowrc.json` > `fallow.toml`)

**Left for next time:**
- `testFileList` at 56 CRAP (E2E test — acceptable, hard to reduce further)
- Run full e2e scan test to completion
- Consider adding coverage data for accurate CRAP scores

**Files touched:** `.fallowrc.json`, `.gitignore`, `fallow.toml` (deleted), `fallow-progress.md`, `fallow-chart.html`, `playwright.tauri.cjs`, `src/app.unit.test.ts`, `src/test/helpers.ts`, `CLAUDE.md`, `package.json`, `package-lock.json`, `src/app.integration.test.ts`, `src/keyboard.test.ts`

**Commits:**
| Hash | Message |
|------|---------|
| `b0a5093` | Fix fallow OOM — ignorePatterns, tighten .gitignore, update progress |
| `dfa15a3` | Restore WIP from stash — CLAUDE.md workflow, coverage deps, test updates |
| `813b7dc` | Migrate fallow config — drop fallow.toml, consolidate to .fallowrc.json |
| `187f9b3` | Update fallow progress — config consolidation row (MI 93.4, dead files/exports 0%) |
| `d3f0228` | Extract test factories — eliminate all duplication (17.2%→0%) |
| `f4915c3` | Refactor playwright.tauri.cjs — extract 10 named functions (CRAP 306→56) |
| `86fd110` | Split CRAP tracking — separate app and test code in table and chart |
| `6eeae90` | Chart colours — CRAP lines teal, reserve red for attention |
| `dbf2baf` | Chart overhaul — green default, red on threshold breach, dotted test lines |
| `2600fc0` | Fix chart label overflow — move values below points, increase top padding |

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
