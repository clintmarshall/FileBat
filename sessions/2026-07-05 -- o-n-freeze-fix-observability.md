# 2026-07-05 — O(n²) Freeze Fix + Observability

**Goal:** Fix the window freeze after scan completion.

### Root Cause

`getMaxFolderSize()` iterated over ALL `knownChildren` to find the largest folder. It was called from `patchUsageRow()` (runs once per folder) and `renderTreeRow()` (runs per rendered row).

For N folders: N × (N+1)/2 iterations. **500K folders → 125 billion operations.** The JS event loop locked up solid.

### Fix

- `maxFolderSize` running variable — updated in O(1) inside `patchUsageRow`
- `foldersSized` counter for observability
- `logMemoryState()` — logs heap usage + tree stats on `scan:complete`
- Backend `println!` with tree_folders and sized_folders at scan completion

### Observability Added

**Frontend (console):**
```
[MEMORY] { treeNodes, foldersSized, maxFolderSize, expandedPaths, usedHeap, heapPct }
```

**Backend (stdout):**
```
[BACKEND MEMORY] scan=scan_1_123 | tree_folders=42 | sized_folders=42 | total_files=85 | total_size=434.2 KB
```

### Tests
- All 52 Rust unit tests pass
- All 7 Rust integration tests pass
- All 135 frontend tests pass

### Branch
`feature/tree-drilldown`

---
