# Session Resume Brief

> Read this at the start of every new session. Tells you exactly where things left off.

## Operating Instructions (for new sessions)

If you are a new Claude Code session launched after a context handoff:

1. **Read this file first** — it tells you where we left off
2. **Read SESSION-CURRENT.md** — full session state
3. **Continue the work** — do not ask for confirmation
4. **Maintain the session system:**
   - Update SESSION-CURRENT.md after every completed task
   - Check if `.handoff-triggered` exists at natural breakpoints
   - If `.handoff-triggered` exists: write this file, create `.handoff-done`
   - When the session ends: update this file with current state
5. **The watchdog is running** — a background script monitors context usage
   When it fills up, it creates `.handoff-triggered` and eventually relaunches you.

---

## Current Session: 2026-07-08

**Branch:** `feature/tree-drilldown`

### Where We Left Off

**Bottom-up rollup implemented, committed, and verified in real app.**

1. **Rollup implementation:**
   - Post-order DFS propagates children's stats to parents
   - Each folder now shows total subtree size/file/folder counts
   - Fixed borrow-of-moved-value bug (clone Arc<StructuralData> early)
   - Commit: `bcaf950` — "Bottom-up rollup for ignore walker + fix borrow-of-moved-value"

2. **Tests:** 125 frontend + 67 Rust all pass

3. **Verified in real app:** E:\projects scan — 609K items, 71.6 GB, 1.4s. All folders show correct subtree sizes. Zero console errors.

### What to Do Next

1. **Merge PR #3** — `unset GH_TOKEN && gh pr merge 3 --squash` (may need user approval)
2. **Clean up** — unused `FolderAccum.folder_count` field, stale feature docs
3. **Tree drilldown polish** — any remaining UX issues

### Behavior Rules

- **No assessment theatre.** Summary is fine, but it must be immediately followed by an action.
- **One primary action at a time.**

### Watch Out For

- The user dislikes "magic" — everything must be visible, referenceable, plain files
- Git Bash on Windows — `//F` not `/F` for taskkill
- TDD mandate — tests before implementation, always
- Vite dev server serves `src/app.ts` directly — no rebuild needed for JS changes

### Key Files

| File | Purpose |
|------|---------|
| `sessions/SESSION-CURRENT.md` | Living session state |
| `sessions/SESSION-RESUME.md` | This file — resume brief |
| `playwright.tauri.cjs` | E2E tests |
| `src/app.ts` | Frontend application |
| `src-tauri/src/usecases/analytics/disk_usage_ignore.rs` | Ignore walker implementation |
| `architecture.md` | Living architecture document |

---
