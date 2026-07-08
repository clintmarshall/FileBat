# 2026-07-04 — Disk Usage Tree + Two-Phase Parallel Scan

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
