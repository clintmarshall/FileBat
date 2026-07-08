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

**E2E scan assertions implemented and passing.**

1. **E2E test improvements:**
   - Scan completes in <10s (5.98s for E:\projects)
   - UI updates during scan (polls every 500ms)
   - First 20 rows have complete data (size + files + folders)
   - Fallback completion detection via progress bar

2. **Root stats fix:**
   - `scan:complete` handler applies root stats from summary if chunk events haven't flushed
   - Fixes rAF flush race condition

3. **All tests pass:** 125 frontend, 67 Rust, E2E green

### What to Do Next

**Primary action — merge PR #3:**
```bash
unset GH_TOKEN && gh pr merge 3 --squash
```
Note: Permission classifier may block this — user needs to approve.

**Secondary:**
- Consider making `ignore` the default implementation (100x faster)
- Tree drilldown polish if needed

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
| `architecture.md` | Living architecture document |

---
