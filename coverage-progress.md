# Coverage Progression

## How to Maintain This File

- **One row per meaningful test addition.** Every time we add tests that shift coverage, add a row. Pull metrics from `npm test -- --coverage` (clover.xml). Don't estimate — use the real numbers.
- **Run coverage after the change.** The row tells the story at a glance.
- **Notes column = the "what" in one line.** What did we test? "Context menu — buildContextMenuItem, clampToViewport, showContextMenu", "Rename flow — startRename, handleDelete".
- **Baseline row captures the full picture.** The very first run gets a detailed breakdown below (which functions are uncovered). Subsequent runs only need the row.
- **Track trajectory, not perfection.** The goal is the arrow pointing up. A single plateau doesn't matter — the trend does.
- **Statements first, conditionals second.** Statement coverage tells you "is it run?". Conditional coverage tells you "are the branches exercised?". Both matter.
- **Distinguish testable from untestable.** Tauri IPC (`invoke`, `event`) requires mocking the window.__TAURI__ bridge. Lines that only run inside the real app (E2E territory) are signal for what's missing from unit tests, not a reason to skip them.
- **When in doubt, keep it brief.** The table is the hero. Details are reference material.

## Metric Sources (where each column comes from)

| Column | Source |
|--------|--------|
| Statements | `clover.xml` → `<metrics statements="X" coveredstatements="Y">` |
| Statements % | `coveredstatements / statements * 100` |
| Branches | `clover.xml` → `<metrics conditionals="X" coveredconditionals="Y">` |
| Branches % | `coveredconditionals / conditionals * 100` |
| Methods | `clover.xml` → `<metrics methods="X" coveredmethods="Y">` |
| Methods % | `coveredmethods / methods * 100` |
| app.ts % | `clover.xml` → file `app.ts` → `coveredstatements / statements * 100` |

---

## Visualisation

Open [`coverage-chart.html`](coverage-chart.html) in any browser for live charts (Statements %, Branches %, Methods %, app.ts %). The chart reads the same data as the table below — update both when adding rows.

## Progression Table

| Date | Statements | Stmt % | Branches | Branch % | Methods | Meth % | app.ts % | Notes |
|------------|----------:|-------:|---------:|---------:|-------:|-------:|---------:|-------|
| 2026-07-03 | 307 / 547 | 56% | 81 / 232 | 35% | 61 / 110 | 55% | 53% | Baseline — utils 100%, helpers 92%, app.ts 271/509 |
| 2026-07-03 | 414 / 547 | 76% | 140 / 232 | 60% | 79 / 110 | 72% | 74% | Unit tests — context menu, selection, rename, file ops, action keys, nav, init error |
| 2026-07-03 | 565 / 605 | 93% | 188 / 232 | 81% | 98 / 110 | 89% | 93% | Round 2 — large/dup scans, paste move, newFolder, snapshot, history, error paths |

## Changes (2026-07-03 — Baseline Details)

### Covered
- **`src/utils.ts`** — 13/13 statements (100%). `formatSize`, `formatDate`, `entryIcon` all exercised.
- **`src/test/helpers.ts`** — 23/25 statements (92%). Test doubles and boot helpers. 2 uncovered lines are an unhit conditional in `MockFileSystem.emit`.

### Uncovered in `app.ts` (238 lines at count 0)

| Lines | Function / Region | Why Uncovered |
|-------|-------------------|---------------|
| 100–144 | Context menu (`buildContextMenuItem`, `clampToViewport`, `showContextMenu`, `hideContextMenu`) | Requires Tauri `window.__TAURI__.menu` — not mocked |
| 162–223 | Rename flow (`createRenameInput`, `setupRenameEvents`, `startRename`, `handleDelete`, `handleCopy`, `handleCut`, `handlePaste`, `handleNewFolder`, `handleOpenWith`) | IPC-heavy — `invoke()` for rename/delete/mkdir |
| 320–358 | `bindRowEvents` callback branches | Right-click context menu, drag-start, double-click rename — need real DOM events |
| 421–436 | `getSelectedPaths`, `getSelectedEntries` | Tauri IPC commands — thin bridges, hard to isolate |
| 468–469 | `init` error branch | Error path in app bootstrap |
| 478–518 | `scan`, `analyze`, `exportResults` | IPC commands — invoke + event listening |
| 574–607 | Analytics rendering | Requires scan events from backend |
| 623–635 | `updateFallowMetrics` | Fallow calculation — pure function, easy to test |
| 777–865 | Fallow/metrics rendering | DOM manipulation after metrics calc |
| 880, 890–896, 908, 916 | Init callback error paths | Error branches in startup |

### Priority Ranking (effort → impact)

1. **Fallow metrics** (lines 623–635, 777–865) — pure functions, no Tauri deps. **Lowest hanging fruit.**
2. **Context menu** (lines 100–144) — `clampToViewport` is pure math. `buildContextMenuItem` creates DOM. Mock `window.__TAURI__.menu`.
3. **Row event binding** (lines 320–358) — dispatch real DOM events on created rows.
4. **Rename/file ops** (lines 162–223) — mock `invoke()`, check state changes.
5. **IPC commands** (lines 421–518) — thinnest value. Just bridges. E2E covers the real path.

## Changes (2026-07-03 — Unit Test Suite)

Created `src/app.unit.test.ts` — 40 tests covering the biggest uncovered areas:

### Context Menu (lines 99–148)
- `buildContextMenuItem` — basic item, disabled, danger, separator, shortcut variants
- `clampToViewport` — within viewport, clamp-x, clamp-y
- `showContextMenu` / `hideContextMenu` — menu visibility

### Selection via DOM Events (lines 318–358)
- Single click, Ctrl+click toggle, Shift+click range
- Double-click folder navigation

### Rename (lines 161–224)
- `createRenameInput` — input with entry name
- `startRename` via F2 — file selects to extension dot, folder selects full name

### File Operations (lines 228–295)
- Copy/cut singular and plural messaging
- Multiple selection before copy/cut

### Action Keys (lines 574–607)
- Enter on folder navigates, Backspace to parent

### Navigation (lines 478–509)
- Back/Forward button states, history traversal, Up button

### Init Error Path (lines 657–659)
- `get_volumes` failure shows startup error

### Render (lines 299–397)
- Empty state, folder/file size columns, icons, data-type attributes, status bar

**Result:** Statements 56% → 76%, Branches 35% → 60%, Methods 55% → 72%, app.ts 53% → 74%. 93 tests pass.

## Changes (2026-07-03 — Round 2: Analytics & Error Paths)

### Integration Tests (`app.integration.test.ts`)
- **Large Files Scan** — tab switch + `start_find_large_files` invoke, `scan:chunk` with `large_file` type renders results
- **Duplicates Scan** — tab switch + `start_find_duplicates` invoke, `scan:chunk` with `duplicate_group` type renders results
- **Paste** — `copy_items` invoke after Ctrl+C then Ctrl+V
- **Cancel Scan** — `cancel_scan` invoke, `scan:error` resets UI
- **Forward Navigation** — Back then Forward restores path
- **Scan Validation** — error when no path set

### Unit Tests (`app.unit.test.ts`)
- **Row Right-Click Context Menu** — menu items, row selection, Open enabled/disabled by entry type, shortcuts
- **Context Menu Action Callback** — click hides menu, disabled items have no listener
- **Rename finishRename** — Enter completes rename (invoke checked), Escape cancels
- **Rename Error Path** — rename invoke throws, error handled
- **Delete Error Path** — delete invoke throws, error shown
- **Paste Move Branch** — cut then paste invokes `move_items`
- **handleNewFolder** — global context menu New Folder invokes `createFolder`
- **handleOpenWith** — context menu has Open item
- **Save Snapshot** — `snapshot_usage` invoke with scan data, empty data message
- **Load History** — history table rendered after snapshot, empty state when no history
- **Unknown Tab** — "No scan configured" message for history tab
- **Ctrl+V Paste Shortcut** — paste triggered by keyboard
- **Navigation Error Path** — `list_dir` failure shows error
- **handleActionKeys** — unknown key returns false
- **Global Context Menu Refresh** — refresh action navigates

**Result:** Statements 76% → 93%, Branches 60% → 81%, Methods 72% → 89%, app.ts 74% → 93%. 125 tests pass.

### Remaining Gaps (defensive, not business logic)
- Lines 880, 898, 908, 916 — `.catch` branches on `listen()` registration (only hit if Tauri event API rejects)
- Line 864 — `loadHistory` catch block (requires `usage_history` invoke to throw)
- Lines 114-115 in `helpers.ts` — `emitEvent` throw path (only hit when no handler registered)
