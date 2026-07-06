# 2026-07-05 — Event Batching + Visible Observability

**Goal:** Fix UI lock during scans. Window unmovable, tab-switch dead.

### Root Cause

Every `scan:children_ready` and `scan:chunk` event triggered a DOM operation **synchronously** on the main thread. Thousands of events per second → thousands of DOM mutations → main thread saturated → paint/input events starved → UI locked.

### Fix

**Event Batching:**
- Events buffer into `pendingChildren` / `pendingStats` arrays (O(1) push)
- `requestAnimationFrame` flushes all pending work once per frame
- DOM mutations batched in a single pass per frame
- `scheduleFlush()` deduplicates — only one rAF scheduled at a time

**Visible Observability:**
- Perf overlay in statusbar: `FPS | Q:queue_depth | C:children_flushed | S:stats_flushed | flush_ms`
- Color-coded FPS: green (>40), orange (20-40), red (<20)
- Backend `println!` with tree_folders and sized_folders at scan completion

**Test Infrastructure:**
- `requestAnimationFrame` mocked as synchronous queue in test helpers
- `flushRaf()` drains the queue + flushes promises
- Tests call `flushRaf()` after emitting events (replaces `flushPromises`)

### Before vs After

| Metric | Before | After |
|---|---|---|
| DOM ops per event | 1 (immediate) | 0 (buffered) |
| DOM ops per frame | N (unbounded) | 1 batch (once per rAF) |
| Main thread blocked | Every event | ~16ms per frame max |
| UI responsive | ❌ Frozen | ✅ Responsive |

### Tests
- All 135 frontend tests pass
- Rust compiles clean (Windows file-lock prevented test run)

### Branch
`feature/tree-drilldown`

---
