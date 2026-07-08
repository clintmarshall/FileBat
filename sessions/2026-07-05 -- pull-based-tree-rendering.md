# 2026-07-05 — Pull-Based Tree Rendering

**Goal:** Replace push-based tree (entire tree over IPC every batch) with pull-based rendering. UI drives what gets rendered, backend stores tree in memory.

### What Was Done

**Backend:**
- Replaced `ScanStep::Structure(ScanStructure)` with `ScanStep::Started` and `ScanStep::ChildrenReady`
- `ScanTreeStarted` emitted at scan start (root path + name only)
- `ScanTreeChildren` emitted per folder as BFS discovers children (path + child names only)
- Tree stored in `AnalyticsUseCase.tree_state` — `HashMap<scan_id, HashMap<parent_path, Vec<ScanTreeChild>>>`
- New command: `get_scan_tree_children(scan_id, parent_path)` — returns children from memory, O(children) IPC
- Removed: `ScanStructure`, `FolderStructure`, `emit_structure_batch`, `all_folders` Vec

**Frontend:**
- Replaced `renderUsageTreeSkeleton` + `createSkeletonRow` with `renderTreeRoot` + `renderTreeRow`
- `scan:tree_started` → renders root row
- `scan:children_ready` → stores children in `knownChildren` Map, enables expand button
- Click → `handleTreeExpand` → renders children from `knownChildren` (no IPC needed, data already there)
- Removed: `expandedPaths`, `parentMap`, incremental merge logic, debug logs

**Tests:**
- Updated `app.integration.test.ts` — uses `scan:tree_started` + `scan:children_ready` instead of `scan:structure`
- All 182 tests pass (47 Rust unit + 7 integration + 135 frontend)

### How It Works

1. Scan starts → `scan:tree_started` with root → frontend renders root row
2. BFS discovers root's children → `scan:children_ready` → frontend enables expand button, stores children
3. User clicks root → children render from `knownChildren` Map (no IPC round-trip)
4. Each child row has disabled toggle until its own `scan:children_ready` fires
5. `get_scan_tree_children` command available for fallback (e.g., re-fetch after page reload)

### Why Better

- **IPC:** ~200 small events (path + child names) vs 1 giant JSON with 3000 folders
- **DOM:** Only renders what the user expands, not the entire tree
- **Simplicity:** No merge logic, no incremental updates, no DOM diffing
- **Memory:** Backend stores tree once, frontend stores children Map

### Branch
`feature/tree-drilldown`

---
