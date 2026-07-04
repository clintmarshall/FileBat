# Fallow Progression

## How to Maintain This File

- **One row per meaningful change.** Every time we act on a fallow finding (refactor, deduplicate, suppress noise), add a row to the table. The row tells the story at a glance.
- **Run `fallow` after the change.** Pull metrics from the `■ Metrics:` summary line and the complexity section. Don't estimate — use the real numbers.
- **Notes column = the "what" in one line.** What did we do? "Extracted `handleScan` from 73-line arrow", "Deduplicated test helpers in integration tests", "Ignored build artifacts".
- **Changes section = the "how" in detail.** Below the table, add a `## Changes (date — label)` block listing exactly what files were touched and why. Future-you will thank present-you.
- **Baseline row captures the full picture.** The very first run gets a detailed breakdown below (dead code list, top offenders, refactoring targets). Subsequent runs only need the row + changes section.
- **Track trajectory, not perfection.** The goal is the arrow pointing up (MI), right (fewer dead files), and down (lower CRAP). A single bad day doesn't matter — the trend does.
- **Distinguish noise from signal.** Build artifacts, minified bundles, and HTML-imported CSS are noise. Real source code with real complexity is signal. Update `fallow.toml` to exclude noise, don't chase it.
- **When in doubt, keep it brief.** The table is the hero. Details are reference material. If a change was trivial (added a dep, tweaked a threshold), one bullet is enough.

## Metric Sources (where each column comes from)

| Column | Fallow Output Location |
|--------|----------------------|
| Loc | `■ Metrics: X LOC` |
| Dead Files | `dead files X% (Y of Z)` |
| Dead Exports | `dead exports X% (Y of Z)` |
| Avg Cycl | `avg cyclomatic X` |
| P90 Cycl | `p90 cyclomatic X` |
| MI | `maintainability X (grade)` |
| Max CRAP | Highest CRAP from `● High complexity functions` |
| Dup Lines | `X lines (Y%) duplicated` from `● Duplicates` |
| Dup % | Same as above |
| Clone Groups | Count of `dup:*` entries under `● Duplicates` |

---

## Visualisation

Open [`fallow-chart.html`](fallow-chart.html) in any browser for live charts (MI, CRAP, Dead Files %, Duplication %). The chart reads the same data as the table below — update both when adding rows.

## Progression Table

| Date | Loc | Dead Files | Dead Exports | Avg Cycl | P90 Cycl | MI | Max CRAP | Dup Lines | Dup % | Clone Groups | Notes |
|------------|------:|----------:|-------------:|---------:|---------:|----:|---------:|----------:|------:|-------------:|-------|
| 2026-07-03 | 2840 | 7 (43.8%) | 0 (0.0%) | 2.1 | 4 | 79.1 | 702.0 | 208 | 9.8% | 11 | Baseline — fallow first run |
| 2026-07-03 | 2192 | 0 (0.0%) | 0 (0.0%) | 1.9 | 3 | 94.9 | 210.0 | 208 | 10.0% | 11 | Noise removed — build artifacts ignored, playwright added to deps |
| 2026-07-03 | 2440 | 0 (0.0%) | 0 (0.0%) | 1.9 | 4 | 94.4 | 210.0 | 144 | 6.6% | 10 | Refactored: keyboard (CRAP 172→43), context menu, renderEntries, test dedup |
| 2026-07-03 | 2403 | 0 (0.0%) | 0 (0.0%) | 1.9 | 4 | 93.8 | 210.0 | 81 | 3.8% | 6 | Extracted utils.ts — formatSize/formatDate/entryIcon shared module |
| 2026-07-03 | 2394 | 0 (0.0%) | 0 (0.0%) | 1.9 | 4 | 93.9 | 210.0 | 35 | 1.7% | 2 | Final: handleActionKeys, startRename, test dedup. Done. |
| 2026-07-03 | 2394 | 0 (0.0%) | 0 (0.0%) | 1.9 | 4 | 93.9 | 210.0 | 35 | 1.7% | 2 | camelCase standardisation — #[serde(rename_all)] on 9 structs |
| 2026-07-03 | 2384 | 0 (0.0%) | 0 (0.0%) | 1.9 | 4 | 93.9 | 132.0 | 17 | 0.8% | 1 | Fix scan events — 4 bugs (overflow, serde rename, timing, CSS) |
| 2026-07-03 | 2522 | 0 (0.0%) | 0 (0.0%) | 1.9 | 4 | 93.8 | 306.0 | 17 | 0.8% | 1 | Fix entryType camelCase regression + contract/E2E tests |
| 2026-07-04 | 4233 | 1 (6.7%) | 3 (20.0%) | 1.6 | 3 | 92.2 | N/R | 106 | 17.0% | 9 | Fallow OOM fix — ignorePatterns, .gitignore tightened, unit tests added |
| 2026-07-04 | 4373 | 0 (0.0%) | 0 (0.0%) | 1.7 | 3 | 93.4 | 306.0 | 675 | 17.2% | 10 | Config consolidation — drop fallow.toml, add ignoreExports, main.css ignored |
| 2026-07-04 | 4182 | 0 (0.0%) | 0 (0.0%) | 1.7 | 3 | 93.2 | 306.0 | 0 | 0.0% | 0 | Extracted test factories — selectFirstRow/startRename/openContextMenu (dup 17.2%→0%) |
| 2026-07-04 | 4205 | 0 (0.0%) | 0 (0.0%) | 1.7 | 3 | 93.1 | 56.0 | 0 | 0.0% | 0 | Refactored playwright.tauri.cjs — 10 named functions (CRAP 306→56) |

## Changes (2026-07-03 — Noise Removal)

- Created `fallow.toml` with `ignorePatterns` for `src-tauri/frontend-dist/`, `target/`, `src/styles/main.css`
- Added `playwright` to `package.json` devDependencies

## Changes (2026-07-03 — Utility Extraction)

- Created `src/utils.ts` with exported `formatSize`, `formatDate`, `entryIcon`
- `app.ts` imports from `./utils` instead of defining locally
- `app.test.ts` imports from `./utils` — eliminated 3 copy-pasted function definitions
- Eliminated 4 clone groups (formatSize/formatDate/entryIcon duplication across test files)

## Changes (2026-07-03 — Complexity Refactoring)

### `src/app.ts` — Keyboard Handler (CRAP 172 → 43)
- Extracted `handleCtrlShortcuts(e)` — Ctrl+A/C/X/V (cyclomatic 5)
- Extracted `handleArrowNav(e)` — ArrowUp/ArrowDown (cyclomatic 4)
- Extracted `handleActionKeys(e)` — Enter, F2, Delete, Backspace (cyclomatic 12)
- Main dispatcher: 5 lines, cyclomatic 2

### `src/app.ts` — Context Menu (CRAP 56 → 0)
- Extracted `buildContextMenuItem(item)` — creates one menu element
- Extracted `clampToViewport(x, y, element)` — pure positioning function
- `showContextMenu` reduced to 8 lines

### `src/app.ts` — Render Entries (82 → 15 lines)
- Extracted `createFileRow(entry, index)` — row DOM creation
- Extracted `bindRowEvents(row, entry, index)` — click/dblclick/contextmenu handlers

### Test Deduplication
- Created `src/test/helpers.ts` — shared `createDom()`, `bootApp()`, `flushPromises()`, `emitEvent()`
- Refactored `app.integration.test.ts` — 474 LOC → 277 LOC, eliminated repeated beforeEach/bootApp
- Refactored `keyboard.test.ts` — uses shared helpers
- Added 6 keyboard navigation tests (Ctrl+A, ArrowDown, ArrowUp, Ctrl+C, Ctrl+X, Delete)

## Changes (2026-07-04 — Fallow OOM Fix + Unit Tests)

- **`.fallowrc.json`** — Added top-level `ignorePatterns` for `target/`, `node_modules/`, `dist/`, `coverage/`, `html/`, `frontend-dist/`, `src/src-tauri/`, `*.old`, `*.lock`. Fallow was parsing 1.5M of HTML coverage reports as source code → OOM.
- **`.gitignore`** — Added `coverage/`, `html/`, `*.old` so generated artifacts stay out of git.
- **`src/app.unit.test.ts`** — New file (1408 LOC). Introduced 9 clone groups (106 lines) of test duplication within the same file — future refactor target.
- **Dead exports** — `src/test/helpers.ts` exports `registeredHandlers`, `createDom`, `mockTauriApi` with no known consumers (43% dead code in that file).
- **Duplicate export** — `bootApp` defined in both `src/test/boot.ts` and `src/test/helpers.ts`.
- **LOC jump** — 2394 → 4233. Expected: unit tests doubled the analyzed source. MI held at 92.2 despite the volume.

## Changes (2026-07-04 — Config Consolidation)

- **Dropped `fallow.toml`** — Superseded by `.fallowrc.json`.
- **Added `src/styles/main.css` to `ignorePatterns`** — Eliminates the last dead-file false positive (imported via `<link>`, not JS).
- **Added `ignoreExports`** — Suppresses dead-export warnings for `helpers.ts` test utilities (`registeredHandlers`, `createDom`, `mockTauriApi`).
- **Result:** Dead files 6.7% → 0%. Dead exports 20% → 0%. MI 92.2 → 93.4.

## Changes (2026-07-04 — Test Factory Extraction)

- **`src/test/helpers.ts`** — Added `selectFirstRow()`, `startRename()`, `openContextMenu()`, `openGlobalContextMenu()`, `dispatchKey()` factories.
- **`src/app.unit.test.ts`** — Refactored 1408 → 1166 LOC. Replaced 20+ instances of repeated boot+select+F2+right-click patterns with factory calls.
- **Result:** Duplication 675 lines (17.2%) → 0 lines (0.0%). Clone groups 10 → 0. MI 93.4 → 93.2 (stable).

## Changes (2026-07-04 — E2E Script Refactor)

- **`playwright.tauri.cjs`** — Extracted 10 named functions from 259-line IIFE:
  - `cleanupWebViewCache()`, `killStaleProcesses()`, `launchApp()` — setup
  - `connectBrowser()` — CDP connection
  - `waitForAppReady()`, `setupPageOverrides()`, `setupConsoleLogging()` — page init
  - `testAppInit()`, `testFileList()`, `testAnalyticsToggle()`, `testScanPath()`, `testDiskUsageScan()` — tests
- **Result:** Max CRAP 306 → 56. Cyclomatic 17 → 7. Cognitive 23 → 8. MI 93.2 → 93.1 (stable).

## Baseline Details (2026-07-03)

### Metrics
- **LOC:** 2,840
- **Dead files:** 43.8% (7 of 16)
- **Dead exports:** 0.0% (0 of 2)
- **Avg cyclomatic:** 2.1
- **P90 cyclomatic:** 4
- **Maintainability Index:** 79.1 (moderate)
- **Duplication:** 208 lines (9.8%) across 3 files, 11 clone groups

### Dead Code (7 unused files)
- `src/styles/main.css` — false positive (imported via HTML `<link>`)
- `src-tauri/frontend-dist/assets/index-BMoSyKvY.css` — Vite build artifact
- `src-tauri/frontend-dist/assets/index-raQYeRWf.js` — Vite build artifact
- `target/debug/build/filebitch-*/out/__global-api-script.js` (x3) — cargo build byproducts

### Dependencies
- Unlisted: `playwright` (imported but missing from package.json)

### Complexity — Top Offenders
| File | Line | Cyclomatic | Cognitive | LOC | CRAP |
|------|------|-----------:|----------:|----:|-----:|
| `frontend-dist/assets/index-raQYeRWf.js` | :11 | 26 | 25 | 1 | 702.0 |
| `playwright.tauri.cjs` | :45 | 14 | 17 | 227 | 210.0 |
| `src/app.ts` | :552 | 26 | 30 | 73 | 172.0 |
| `src/app.ts` | :129 | 14 | 19 | 45 | 56.3 |

### Refactoring Targets
1. **Score 17.7** — `frontend-dist/assets/index-raQYeRWf.js` (untested risk — build artifact, ignore)
2. **Score 4.3** — `src/app.ts` (complexity — cognitive 30 arrow in 986-LOC file)

## Changes (2026-07-03 — Fix Scan Events)

**4 bugs found via instrumentation (eprintln! + MutationObserver), not guessing:**

1. **Rust integer overflow** — `ancestor.components().count() - base_depth` underflowed when walkdir returned paths with fewer components than the base. The `spawn_blocking` task panicked, dropping all events. Added guard check.
2. **Serde field rename mismatch** — `ScanChunk.data` was serialized as `"all"` but frontend read `chunk.data`. Removed `#[serde(rename = "all")]`.
3. **Frontend timing** — `clearTabResults` called after `invoke()` returned, overwriting the table rendered by chunk events. Moved `clearTabResults` before the `invoke()` call.
4. **CSS layout** — `.analytics-tab-content` was `display: block` with no height. Changed to `display: flex; flex-direction: column` with `flex: 1` on result containers.

**Files:** `src-tauri/src/domain/models.rs`, `src-tauri/src/usecases/analytics/aggregator.rs`, `src/app.ts`, `src/app.integration.test.ts`, `src/styles/main.css`, `playwright.tauri.cjs`

**Result:** Max CRAP 210.0 → 132.0 (playwright.tauri.cjs, down from removed build artifact). Dup 35 lines → 17 lines. All 92 tests pass.

## Changes (2026-07-03 — Fix entryType camelCase Regression)

**Bug:** `#[serde(rename_all = "camelCase")]` on `Entry` serializes `entry_type` as `entryType`, but the frontend read `entry_type` (snake_case). Every `entry_type` check returned `undefined`:
- All entries showed file icon (📄) instead of folder (📁)
- Double-click navigation never triggered
- Size column showed for folders (should be blank)

**Fix:** Updated frontend `Entry` interface and all usages to `entryType`.

**Tests added:**
- `entry-serialization.test.ts` — Contract test verifying Rust serialization matches frontend expectations
- E2E — Folder icon check + double-click navigation verification

**Files:** `src/app.ts`, `src/keyboard.test.ts`, `src/entry-serialization.test.ts`, `playwright.tauri.cjs`

**Result:** LOC 2384 → 2522 (new test file + E2E additions). Max CRAP 132 → 306 (E2E test growth in playwright.tauri.cjs). MI 93.9 → 93.8 (stable). Dup 0.8% unchanged.
