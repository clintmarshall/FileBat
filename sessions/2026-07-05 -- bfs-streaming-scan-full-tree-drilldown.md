# 2026-07-05 — BFS Streaming Scan + Full Tree Drilldown

**Goal:** Enable tree drilldown at any depth. Replace O(depth × files) redundant walks with leaf-only sizing + rollup.

### What Was Done
- Rewrote `disk_usage.rs` as BFS streaming loop:
  - Phase 1: BFS discovers folders in batches of 200
  - Phase 2: Leaf folders sized in parallel (10 threads) as discovered
  - Phase 3: Rollup propagates totals bottom-up after discovery completes
- Frontend: removed depth selector, cleaned debug console.logs
- 7 Rust unit tests for `readdir_children`, `size_folder`
- All 54 Rust tests pass, 135 frontend tests pass, E2E passes

### Crash — Memory Allocation Failure
Scanning E: drive crashed with `memory allocation of 16777216 bytes failed` / `STATUS_STACK_BUFFER_OVERRUN`.

**Root cause:** Every `scan:structure` event emits the ENTIRE accumulated folder list. On a large drive with thousands of folders, each batch sends a growing JSON payload. The 16MB allocation failure is likely the IPC bridge trying to serialize a massive folder list.

**Resolved:** Structure is now emitted incrementally after each batch. Frontend merges incrementally (no full re-render). Payload grows with each batch but the tree renders immediately — no single massive emission.

### Remaining Work

**1. Progress bar**
- Currently only updates at 100%. Add incremental progress per completed folder.

**2. Cancel between phases**
- Test cancel during structure phase vs sizing phase

**3. Deeper-level rendering on incremental updates**
- When a previously-rendered folder gains new children (discovered in a later batch), the children container isn't updated until the user expands the folder. Minor visual issue — tree functions correctly.

### Branch
`feature/tree-drilldown`

---
