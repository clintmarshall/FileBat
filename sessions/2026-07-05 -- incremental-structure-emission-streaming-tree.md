# 2026-07-05 — Incremental Structure Emission (Streaming Tree)

**Goal:** Emit the tree structure immediately after discovering the first level, then continue discovering in the background. Tree renders instantly and grows.

### What Was Done

**Backend (`disk_usage.rs`):**
- Moved `ScanStep::Structure` emission INSIDE the BFS loop (after each batch of 200 folders)
- Previously: structure emitted ONCE after full discovery (minutes on large drives)
- Now: structure emitted after every batch — frontend gets root + level 1 immediately
- Added `emit_structure_batch()` helper — populates `children` from `parent_children` map before emission
- Removed the single post-discovery structure emission

**Frontend (`app.ts`):**
- Refactored `renderUsageTreeSkeleton()` to support incremental updates
- Previously: cleared container and re-rendered from scratch on every structure event
- Now: merges new folders into existing DOM — skips already-rendered rows via `data-path`
- Header and body created once (first call), subsequent calls append only new roots
- Auto-expands roots on every incremental update

### How It Works

**Batch 1** (root + level 1 discovered):
- Backend emits structure with root + immediate children
- Frontend renders root (expanded) + level 1 rows
- User sees the tree immediately

**Batch 2+** (deeper levels discovered):
- Backend emits structure with all discovered folders so far
- Frontend merges — skips existing rows, adds new ones
- Existing rows retain their expansion state

**Sizing** (concurrent with discovery):
- Leaf folders sized in parallel (10 threads) as discovered
- `scan:chunk` events patch individual rows with size/file count/folder count

### Tests
- All 135 frontend tests pass
- All 47 Rust unit tests pass
- All 7 integration tests pass

### Branch
`feature/tree-drilldown`

---
