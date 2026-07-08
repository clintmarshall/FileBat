# Session — Current State

> Living document. Updated throughout the session, not after.

**Date:** 2026-07-08
**Branch:** `feature/tree-drilldown`
**Last Updated:** 2026-07-08 ~12:50

---

## What's Done (Recent)

### Completed & Merged to main
- Core file explorer (sidebar, file list, toolbar, navigation)
- Analytics panel (disk usage, large files, duplicates)
- SQLite analytics history
- Tauri 2 integration, Windows 11 theme

### Completed on `feature/tree-drilldown` (not merged)

#### Jul 8 — Bottom-Up Rollup for Ignore Walker
- **Problem:** Ignore walker accumulators only tracked immediate files per folder. Parent folders showed 0 size because child stats weren't propagated upward.
- **Solution:** Post-order DFS rollup after the walk completes — children's rolled stats propagate to parents. Each folder now shows total subtree size, file count, and folder count.
- **Bug fix:** `final_arena` was moved into `Arc::new(...)` but later borrowed for `structural_ref()`. Fixed by cloning `Arc<StructuralData>` early through the mutex.
- **All tests pass:** 125 frontend, 67 Rust
- **Verified in real app:** E:\projects scan — 609K items, 71.6 GB, 1.4s. All folders show correct subtree sizes (comfyui 6.5GB/81 files/60 folders, filebitch 5.5GB/4262 files/347 folders). Zero console errors.

#### Jul 8 — E2E Scan Assertions & Root Stats Fix
- E2E scan test now asserts:
  - Scan completes in <10s (5.98s for E:\projects)
  - UI updates during scan (polls every 500ms, checks rows appear progressively)
  - First 20 rows have complete data (size, file count, folder count)
  - Fallback completion detection via progress bar
- Root stats fallback in `scan:complete` — if chunk events haven't flushed via rAF, apply from summary data directly
- **All tests pass:** 125 frontend, 67 Rust, E2E green

#### Jul 7 — ignore::WalkParallel Prototype
- Single-pass parallel walk, 100x faster (1.3s vs 120s+)
- Feature-gated behind `ignore-walker` flag (now always-on, feature removed)
- E:\projects scan: 1,510 folders, 91,756 files, 11.4 GB in 1.3s
- **PR #3 created** — https://github.com/clintmarshall/FileBat/pull/3 (OPEN, needs merge)

#### Jul 7 — Interactive SDLC & Size Bars
- Size bars fixed — `recalcSiblings()` called after children are in DOM
- Interactive workflow established — Chrome DevTools MCP + CDP port 9222
- E2E test upgraded — `E:\projects` scan (1.8M files, 273 GB)

#### Jul 6 — Memory Audit (NodeId Arena)
- Full path strings → `NodeId(u32)` identity everywhere
- `FolderArena` — Structure of Arrays, first-child/next-sibling tree
- Thin events — `{parentId, childCount}` (16 bytes vs ~840 bytes)
- **94% memory reduction** (900 MB → 54 MB typical)

#### Jul 5 — Streaming & Tree Features
- BFS streaming scan — incremental structure emission after each batch
- Pull-based tree rendering — children fetched on demand
- N^2 freeze fix — O(1) DOM lookup via `data-node-id`

---

## What's In Progress

- **Merge PR #3** — blocked on permission classifier (needs user approval)

## What's Next (Prioritized)

1. **Merge PR #3** — user needs to approve squash merge
2. **Tree drilldown polish** — any remaining UX issues from NodeId refactor
3. **Clean up** — remove unused `folder_count` field from `FolderAccum`, stale `ignore-walker` feature docs

## Current Blockers

- **PR #3 merge** — permission classifier blocks `gh pr merge` in auto mode

## Key Gotchas

- `taskkill //F //IM` (double slash) for Git Bash
- Python `subprocess.Popen` with `env=` for CDP launch
- `unset GH_TOKEN` before `gh` commands
- rAF flush race: `scan:complete` fires before pending stats flush to DOM
- Vite dev server serves `src/app.ts` directly — no rebuild needed for JS changes
- `ignore-walker` feature was never added to Cargo.toml — `disk_usage_ignore` is always the default now

## Environment

- **Platform:** Windows 11 Home
- **Shell:** Git Bash (PowerShell available)
- **Model:** Qwen3.6-27B-UD-Q4_K_XL (local)
- **CDP Port:** 9222 (for Chrome DevTools MCP)

---
