# 2026-07-04 — Fallow OOM Fix, Test Dedup, E2E Refactor, Chart Overhaul

**Goal:** Fix fallow running out of memory, eliminate test duplication, refactor E2E script, improve chart readability

### Fallow OOM Fix
- Fallow was parsing 1.5M of HTML coverage reports as source code → OOM
- Created `.fallowrc.json` with top-level `ignorePatterns` (coverage/, html/, frontend-dist/, target/, *.old, *.lock)
- Updated `.gitignore` to exclude generated artifacts (coverage/, html/, *.old)
- Dropped `fallow.toml` — consolidated all config into `.fallowrc.json`

### Test Factory Extraction
- Extracted 5 factories into `helpers.ts`: `selectFirstRow`, `startRename`, `openContextMenu`, `openGlobalContextMenu`, `dispatchKey`
- Refactored `app.unit.test.ts`: 1408 → 1166 LOC
- Duplication: 17.2% (675 lines, 10 clone groups) → 0%

### E2E Script Refactor
- Split 259-line IIFE in `playwright.tauri.cjs` into 10 named functions
- CRAP: 306 → 56 (cyclomatic 17→7, cognitive 23→8)
- Functions: `cleanupWebViewCache`, `killStaleProcesses`, `launchApp`, `connectBrowser`, `waitForAppReady`, `setupPageOverrides`, `setupConsoleLogging`, `testAppInit`, `testFileList`, `testAnalyticsToggle`, `testScanPath`, `testDiskUsageScan`

### Chart Overhaul
- All lines green by default, segments turn red when thresholds breached (MI < 85, CRAP ≥ 30, dead > 0, dup > 0)
- Test code lines rendered dotted
- Split CRAP tracking: CRAP (app) vs CRAP (test) columns in table and chart
- Labels below points to prevent overflow

### Stash Recovery
- Recovered stashed WIP from `fix/scan-events` branch (CLAUDE.md workflow, coverage deps, expanded integration tests)
- Merged main into branch, resolved conflicts in progress tracking files
- Created PR #2, merged to main

### Running the App
```bash
# Terminal 1 — Vite dev server (compiles TS, serves on :1420)
npx vite

# Terminal 2 — Tauri dev (connects to Vite, launches window)
npx tauri dev
```

**Metrics:**
| MI | CRAP (app) | CRAP (test) | Dup % | Dead Files | Dead Exports |
|----|-----------|-------------|-------|------------|-------------|
| 92.2 → 93.1 | 43 → 10.2 | 306 → 56 | 17.2% → 0% | 6.7% → 0% | 20% → 0% |

**Decisions:**
- Green-by-default chart lines, red reserved for threshold breaches
- Test code lines dotted to distinguish from app code
- Split CRAP tracking (app vs test) — test code obscures real code signal
- `ignorePatterns` at top-level in `.fallowrc.json` (not just under `duplicates`)
- Consolidate fallow config to single file (`.fallowrc.json` > `fallow.toml`)

**Left for next time:**
- `testFileList` at 56 CRAP (E2E test — acceptable, hard to reduce further)
- Run full e2e scan test to completion
- Consider adding coverage data for accurate CRAP scores

**Files touched:** `.fallowrc.json`, `.gitignore`, `fallow.toml` (deleted), `fallow-progress.md`, `fallow-chart.html`, `playwright.tauri.cjs`, `src/app.unit.test.ts`, `src/test/helpers.ts`, `CLAUDE.md`, `package.json`, `package-lock.json`, `src/app.integration.test.ts`, `src/keyboard.test.ts`

**Commits:**
| Hash | Message |
|------|---------|
| `b0a5093` | Fix fallow OOM — ignorePatterns, tighten .gitignore, update progress |
| `dfa15a3` | Restore WIP from stash — CLAUDE.md workflow, coverage deps, test updates |
| `813b7dc` | Migrate fallow config — drop fallow.toml, consolidate to .fallowrc.json |
| `187f9b3` | Update fallow progress — config consolidation row (MI 93.4, dead files/exports 0%) |
| `d3f0228` | Extract test factories — eliminate all duplication (17.2%→0%) |
| `f4915c3` | Refactor playwright.tauri.cjs — extract 10 named functions (CRAP 306→56) |
| `86fd110` | Split CRAP tracking — separate app and test code in table and chart |
| `6eeae90` | Chart colours — CRAP lines teal, reserve red for attention |
| `dbf2baf` | Chart overhaul — green default, red on threshold breach, dotted test lines |
| `2600fc0` | Fix chart label overflow — move values below points, increase top padding |
