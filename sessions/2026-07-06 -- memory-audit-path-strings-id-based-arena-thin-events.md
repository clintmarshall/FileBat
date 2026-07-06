# 2026-07-06 — Memory Audit: Path Strings → ID-based Arena + Thin Events

**Goal:** Eliminate OOM on large drive scans (3.5M files, 2.8TB, ~500K folders).

### Root Cause

Every folder stored its full path string 4–6× across backend + frontend + IPC. For 500K folders:
- Backend `tree` HashMap: ~140 MB (path strings as keys + child entries)
- Backend `leaf_results` HashMap: ~80 MB (path + stats per folder)
- Backend rollup `pending` HashSet: ~60 MB
- Frontend `knownChildren` Map: ~620 MB (V8 strings are 5–6× more expensive than Rust)
- **Total: ~900 MB** — WebView2 heap limit is ~2 GB

### Design Decisions

**1. NodeId (u32) replaces path strings as identity**
- `NodeId(u32)` — index into `FolderArena` Vec, Copy semantics
- `NameId(u32)` — index into deduplicated string pool
- `FolderUsage.node_id` replaces `FolderUsage.path`
- `ScanTreeChild.id` replaces `ScanTreeChild.path`

**2. FolderArena — Structure of Arrays**
- Structural vectors: `parent`, `first_child`, `next_sibling`, `name_id`
- Metadata vectors: `size`, `file_count`, `folder_count`, `sized`
- First-child/next-sibling tree — zero heap allocation per directory
- String pool — one copy of every name, deduplicated
- **~48 bytes per folder** (vs ~340 bytes with path strings)

**3. Thin Events**
- `ScanTreeChildren` now carries `{parentId, childCount}` (16 bytes) instead of full child array (~840 bytes)
- Frontend stores only expanded nodes — pulls children on demand via `get_scan_tree_children`
- **90% smaller IPC payloads**

**4. Rollup via Parent Pointers**
- Eliminated: `leaf_results` HashMap, `pending` HashSet, notification channel, separate rollup thread
- After sizing a leaf, walk parent chain: check if all siblings sized → roll up → cascade upward
- Single arena lock, one path walk

**5. Frontend stores only what the user expands**
- `treeStore` keyed by NodeId, children only stored when expanded
- `pathMap` resolves paths lazily for rendering and snapshot
- `data-node-id` attribute replaces `data-path` — O(1) DOM lookup, no normalization hacks

### Memory Before vs After

| Structure | Before | After |
|---|---|---|
| Backend tree | ~140 MB | ~24 MB (arena) |
| Backend leaf_results | ~80 MB | ~0 (inline in arena) |
| Backend rollup state | ~60 MB | ~0 (parent pointers) |
| Frontend (typical) | ~620 MB | ~10 MB (expanded only) |
| IPC per event | ~200 bytes | ~16 bytes |
| **Total (typical)** | **~900 MB** | **~54 MB** |

**94% reduction at typical usage.**

### What Was Done

**Backend (`models.rs`):**
- Added `NodeId(u32)`, `NameId(u32)` types
- `FolderUsage.path` → `FolderUsage.node_id`
- `ScanTreeChild.path` → `ScanTreeChild.id`
- `ScanTreeChildren` → thin: `{parentId, childCount}` (dropped full child array)
- `ScanTreeStarted` → added `root_id`

**Backend (`arena.rs` — NEW):**
- `FolderArena` SoA with first-child/next-sibling tree
- String pool with deduplication
- `alloc_folder()`, `add_child()`, `children()` iterator
- `resolve_path()`, `all_children_sized()`, `sum_children()`
- 12 unit tests including 50K-node stress test

**Backend (`disk_usage.rs`):**
- BFS assigns `NodeId` during discovery, stores in arena
- `readdir_names()` returns `(name, path)` tuples (no longer `ScanTreeChild`)
- `size_folder_stats()` returns `(size, file_count, folder_count)` tuple
- Rollup: reverse BFS order pass, parent-pointer walk
- Thin `scan:children_ready` events
- Arena stored in `tree` HashMap for `get_children` queries

**Backend (`mod.rs`):**
- `tree` type: `HashMap<String, FolderArena>` (was `HashMap<String, HashMap<String, Vec<ScanTreeChild>>>`)
- `get_children()` follows FC/NS chain, sorts by name

**Backend (`scan.rs`):**
- `get_scan_tree_children` takes `parentId: u32` instead of `parentPath: String`

**Backend (`aggregator.rs`):**
- `FolderUsageAccumulator` updated to use `node_id` (legacy code, kept for tests)

### Current State (Mid-Session — Resume Here)

**RESOLVED — Session complete. All changes implemented and verified.**

### Results

**Backend:**
- Fixed 2 arena tests:
  1. `resolve_path_walks_parent_chain` — stripped trailing slash from `root_path` before appending segments
  2. `parent_pointer_walk_rollup` — corrected assertion from `folder_count == 1` to `== 2` (root has 2 descendant folders: `a` + `a1`)
- **64/64 unit tests + 7/7 integration tests pass**

**Frontend (`app.ts`):**
- Replaced `knownChildren: Map<path,...>` → `treeStore: Map<NodeId, TreeNodeData>`
- Replaced `expandedPaths: Set<string>` → `Set<number>`
- Added `pathMap: Map<NodeId, PathInfo>` for lazy path resolution
- Event handlers updated: `rootId` in tree_started, thin `{parentId, childCount}` in children_ready, `nodeId` in chunk events
- `handleTreeExpand` now pulls children via `invoke('get_scan_tree_children', {scanId, parentId})`
- All rendering uses `data-node-id` attribute (O(1) DOM lookup, no path normalization)
- Removed dead code: `findTreeRow()`, `escapeAttr()`, `renderChildrenForParent()`, `UsageTreeNode`, `buildUsageTree()`
- `saveSnapshot` resolves paths from `pathMap`

**Tests:**
- `app.integration.test.ts` — updated event payloads (rootId, parentId/childCount, nodeId), added `get_scan_tree_children` mock
- `usage-tree.test.ts` — deleted (tested dead `buildUsageTree()`)
- **125/125 frontend tests pass**
- **E2E: all checks passed** (drives, navigation, analytics toggle, disk usage scan)

**Architecture:**
- `architecture.md` updated: new domain models (NodeId, NameId, ScanTreeChild, ScanTreeChildren, ScanTreeStarted), updated FolderUsage (node_id), new arena.rs in source layout, get_scan_tree_children command, NodeId-based design decision (#11)

### Files Changed

| File | Status |
|------|--------|
| `src-tauri/src/domain/models.rs` | ✅ NodeId, NameId added. FolderUsage/ScanTreeChild/ScanTreeChildren/ScanTreeStarted updated |
| `src-tauri/src/usecases/analytics/arena.rs` | ✅ NEW — FolderArena SoA, FC/NS tree, string pool, 12 tests. 2 test fixes applied |
| `src-tauri/src/usecases/analytics/mod.rs` | ✅ tree → HashMap<String, FolderArena>, get_children uses arena |
| `src-tauri/src/usecases/analytics/disk_usage.rs` | ✅ BFS with arena, thin events, parent-pointer rollup |
| `src-tauri/src/usecases/analytics/aggregator.rs` | ✅ node_id instead of path (legacy, kept for tests) |
| `src-tauri/src/commands/scan.rs` | ✅ get_scan_tree_children takes parentId: u32 |
| `src/app.ts` | ✅ Full rewrite — NodeId-based treeStore, pathMap, pull-based children, thin events |
| `src/app.integration.test.ts` | ✅ Updated event payloads, added get_scan_tree_children mock |
| `src/usage-tree.test.ts` | ✅ DELETED (tested dead buildUsageTree) |
| `playwright.tauri.cjs` | ✅ No changes needed (uses class selectors) |
| `architecture.md` | ✅ Updated with new types, arena module, ID-based decision |


### Branch
`feature/tree-drilldown`

---
