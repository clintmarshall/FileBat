# 2026-07-07 — ignore::WalkParallel Prototype: Single-Pass Scan

**Goal:** Replace custom BFS + crossbeam + WalkDir with `ignore::WalkParallel` for faster disk usage scans.

### Why

The original two-phase scan (BFS discovery + parallel sizing) was slow:
- Phase 1: BFS discovers folders via `readdir`
- Phase 2: 10 worker threads each run `WalkDir` on their leaf folder
- Files in nested subdirectories walked multiple times
- `E:\projects` scan timed out after 120s

### Design Decisions

**1. `ignore::WalkParallel` — single-pass parallel walk**
- Walks every entry exactly once
- Metadata is free from `readdir` — no second walk needed
- Internal work-stealing thread pool — no manual thread management
- `.ignore(false)` — we're scanning disk usage, not source code

**2. Feature-gated behind `ignore-walker` (REMOVED)**
- ~~`cargo build` — original implementation (default)~~
- ~~`cargo build --features ignore-walker` — new implementation~~
- The feature flag was removed — jwalk is always-on. `ignore` crate remains in Cargo.toml but the cfg-gate in mod.rs was removed.

**3. Post-walk parent linking**
- Parallel traversal doesn't guarantee parent-first order
- Collect all folders during walk, link parents after walk completes
- Path-based parent resolution: `rsplit_once('/')` to find parent path

**4. Arena reuse**
- Both implementations use `FolderArena` for tree storage
- Frontend queries via `get_scan_tree_children` unchanged

### Results

| Metric | Original | `ignore` |
|--------|---------|----------|
| Scan time | >120s (timeout) | **1.3s** |
| Folders in tree | N/A | **1,510** |
| Files found | N/A | **91,756** |
| Size | N/A | **11.4 GB** |

**~100x faster** for E:\projects scan.

### Bugs Fixed

1. **Root directory overwrite** — `ignore` walker visits the root directory itself, allocating a new nodeId and overwriting the root's entry in folders_map. Fixed by skipping root in walker closure.

2. **CDP race condition** — `playwright.tauri.cjs` had a race where HTTP endpoint came up before WebSocket was ready. Added retry loop around actual CDP connection.

3. **Event ordering** — All events fire at once after walk completes. Added 500ms pause after `children_ready` events so frontend can fetch children before `scan:complete` fires.

### Current State

- **All E2E tests pass** with `--features ignore-walker`
- **66/66 unit tests + 7/7 integration tests pass**
- Original implementation still works (default, no feature flag)
- Parent folder counts not incremented (was a no-op loop, removed)

### Files Changed

| File | Status |
|------|--------|
| `src-tauri/Cargo.toml` | ✅ Added `ignore` optional dependency, `ignore-walker` feature |
| `src-tauri/src/usecases/analytics/disk_usage_ignore.rs` | ✅ NEW — single-pass WalkParallel implementation |
| `src-tauri/src/usecases/analytics/mod.rs` | ✅ Feature switch for scan_usage |
| `playwright.tauri.cjs` | ✅ CDP retry fix, 1s wait after scan completion |

### Left for Next Time

- Increment parent folder counts during post-walk linking
- Remove 500ms pause — stream events instead of batch-firing
- Add unit tests for `disk_usage_ignore.rs`
- Benchmark on larger datasets (full drive scans)
- Consider making `ignore` the default implementation

### Branch

`feature/tree-drilldown`

---
