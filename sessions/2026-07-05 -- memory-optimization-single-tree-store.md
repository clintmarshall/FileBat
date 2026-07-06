# 2026-07-05 — Memory Optimization: Single Tree Store

**Goal:** Eliminate duplicate tree storage that caused OOM crashes on large drives. Six data structures held the same tree.

### The Problem

Every folder was stored in **six** places across backend and frontend:
- Backend: `parent_children` (Vec\<String\>), `tree_state` (Vec\<ScanTreeChild\>), `leaf_results` (FolderUsage)
- Frontend: `knownChildren` (Vec\<{path,name}\>), `folderStats`, `scanResults.usage` array

For 500K folders → ~3M strings + 6 maps/arrays. The 16MB IPC allocation failure was the symptom.

### What Was Done

**Backend (`disk_usage.rs`, `mod.rs`):**
- Merged `parent_children` + `tree_state` into one: `tree: HashMap<scan_id, HashMap<path, Vec<ScanTreeChild>>>`
- `parent_children` no longer exists — BFS writes directly to `tree`
- `readdir_children` now returns `Vec<ScanTreeChild>` (dropped `DiscoveredFolder` struct)
- Rollup thread reads from `tree` via `tree[scan_id][parent].iter().map(|c| &c.path)`
- `get_children` command reads from `tree` directly
- Removed `emitted_children` HashSet (unused)
- Renamed `tree_state` → `tree` in `AnalyticsUseCase`

**Frontend (`app.ts`):**
- Merged `folderStats` into `knownChildren` — now `Map<path, {children, stats?}>`
- Dropped `scanResults.usage` array — stats live in the tree nodes
- `patchUsageRow` stores stats in `knownChildren`, creates entry if children not yet discovered
- `saveSnapshot` collects usage data from `knownChildren` instead of flat array
- `getMaxFolderSize()` scans `knownChildren` values for size-bar scaling
- Removed all debug `console.log` calls

### Memory Before vs After

| Structure | Before | After |
|---|---|---|
| Backend tree | 3 copies (parent_children, tree_state, leaf_results) | 2 (tree + leaf_results) |
| Frontend tree | 3 copies (knownChildren, folderStats, scanResults.usage) | 1 (knownChildren with inline stats) |
| **Total** | **6 data structures** | **3 data structures** |

### Tests
- All 52 Rust unit tests pass
- All 7 Rust integration tests pass
- All 135 frontend tests pass
- E2E passes (scan E:\projects\filebitch\src → 6 folders, 85 items, 434KB)

### Branch
`feature/tree-drilldown`

---
