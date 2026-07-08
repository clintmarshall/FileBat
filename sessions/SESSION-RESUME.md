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

## Last Session: 2026-07-07

**Branch:** `feature/tree-drilldown`

### Where We Left Off

**Context management system — built, not tested.**

1. **SESSION-CURRENT.md** — living document, created and maintained
2. **SESSION-RESUME.md** — this file, ready for handoff
3. **context_manager.sh** — complete rewrite. Polls llama-server metrics, triggers handoff at 85%, coordinates via sentinel files, resets server, relaunches fresh session
4. **context_state.json** — structured state file written every poll. Dashboard-ready. Contains: current status, token usage, handoff history with timestamps and results

**Status:** Script written. Session files created. Memory entries saved. JSON structure validated. **Not yet tested end-to-end.**

### What to Do Next

**Primary action — run the E2E test to verify the handoff didn't break anything:**
```bash
npm run test:e2e
```

**Then:** continue with whatever work was in progress. See SESSION-CURRENT.md for details.

**Secondary:**
- Tune handoff threshold if 90% proved too aggressive or too lenient
- Build dashboard from `context_state.json` (future work)
- Update this file when the session ends or context gets tight.

### Behavior Rules

- **No assessment theatre.** Summary is fine, but it must be immediately followed by an action. First output = doing something, not reporting.
- **One primary action at a time.** This section always has exactly one top item — do it, don't choose between options.

### Watch Out For

- The user dislikes "magic" — everything must be visible, referenceable, plain files
- The user wants autonomous capability — continue work without prompting when context fills
- Git Bash on Windows — `//F` not `/F` for taskkill, Python for env var propagation
- TDD mandate — tests before implementation, always

### Key Files

| File | Purpose |
|------|---------|
| `sessions/SESSION-CURRENT.md` | Living session state — updated throughout |
| `sessions/SESSION-RESUME.md` | This file — resume brief for next session |
| `sessions/2026-07-*.md` | Archived session records |
| `context_manager.sh` | Watchdog script — polls metrics, triggers handoff |
| `context_state.json` | Structured state — written every poll, dashboard-ready |
| `architecture.md` | Living architecture document |
| `sdlc.md` | Development workflow, error recovery |

---
