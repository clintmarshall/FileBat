# 2026-07-07 — Interactive SDLC, Size Bar Fix, E2E Overhaul

**Goal:** Fix the SDLC so I can actually verify changes interactively, stop going in circles.

### Root Cause

Two independent problems:

1. **Size bars stayed at 0%** — `updateBar(bar, nameCell)` was called in `renderTreeRow` at line 1356, but the row wasn't inside the wrapper yet (wrapper created 3 lines later). `siblingsContainerOf` traverses up from nameCell → row → parent, found nothing, bars stayed at 0%.

2. **No interactive verify loop** — I kept killing the app before connecting to it (taskkill → launch → repeat loop). The Chrome DevTools MCP was already connected to the running app via CDP port 9222 the whole time. The "going around in circles" wasn't a tooling issue — it was process discipline.

### What Was Done

**1. Interactive Workflow Established**

- Chrome DevTools MCP connects to running Tauri app via `--remote-debugging-port=9222`
- `list_pages` → find FileBitch page
- `take_snapshot` → full DOM with uids
- `click`, `fill`, `screenshot`, `evaluate_script` → interact normally
- **DO NOT kill the app before connecting** — just connect to what's running
- **DO NOT run the E2E script** for interactive verification — use Chrome DevTools MCP directly

**2. Size Bar Bug Fix (`app.ts`)**

- Removed `updateBar(bar, nameCell)` from `renderTreeRow` (row not in DOM yet)
- Added `recalcSiblings(childrenContainer)` after children are appended in three places:
  - `renderTreeRow` — pre-expanded children
  - `fetchAndRenderChildren` — lazy-loaded children
  - `handleTreeExpand` — user clicks to expand
- `flushPendingEvents` path was already fine — row IS in DOM when events arrive

**3. E2E Test Overhaul (`playwright.tauri.cjs`)**

- Scan path: `src` (29 items, 144 KB) → `E:\projects` (1.8M items, 273 GB)
- Timeout: 60s → 120s for larger scans
- File added to git (was untracked)

**4. SDLC Documentation (`sdlc.md` — NEW)**

- Quick iterate loop documented
- Automated verification section
- Definition of done
- Error recovery protocol (three failure states):
  - State 1: Stale UIDs → fresh snapshot
  - State 2: Target closed → restart, reconnect
  - State 3: Click does nothing → check console errors, fix the bug
- GitHub CLI auth workaround: `unset GH_TOKEN` before `gh` commands (env var is invalid, keyring token works)

**5. Memory Records**

- `filebitch-interactive-workflow.md` — how to connect Chrome DevTools MCP to running app
- Added to `MEMORY.md` index

### Results

**Verified interactively:**
- E:\projects scan: 1.8M files, 273.1 GB, 37 seconds
- Size bars correct at all nesting levels (root = 100%, children = percentage of parent)
- Nested bars correct after expand (filebitch → target 96%, src-tauri 5.6%, node_modules 0.3%)

**Tests:**
- 125/125 frontend tests pass
- E2E: all checks passed (drives, navigation, analytics toggle, disk usage scan)

**PR:** #3 created at https://github.com/clintmarshall/FileBat/pull/3

### Commits

| Commit | Message |
|--------|---------|
| `d2c2371` | Progressive size bars, alphabetical sort, better E2E test |
| `d6131b9` | Add sdlc.md — document the interactive verify loop |
| `d83ec32` | Fix size bars — calculate widths after children are in the DOM |
| `ec7bf08` | Add error recovery protocol to sdlc.md |
| `67f3984` | Document gh auth workaround in sdlc.md |

### Files Changed

| File | Status |
|------|--------|
| `src/app.ts` | ✅ Size bar fix — recalcSiblings after children in DOM |
| `playwright.tauri.cjs` | ✅ E:\projects scan, 120s timeout, added to git |
| `sdlc.md` | ✅ NEW — interactive workflow, error recovery, gh auth |
| `memory/filebitch-interactive-workflow.md` | ✅ NEW — Chrome DevTools MCP connection guide |
| `memory/MEMORY.md` | ✅ Index updated |

### Branch
`feature/tree-drilldown`

---
