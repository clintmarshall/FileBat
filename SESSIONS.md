# Session Log

## 2026-07-03 — Fallow Health Sweep

**Goal:** Run fallow, remove noise, refactor complexity, set up git

**What we did:**
- Created `fallow.toml` — excluded build artifacts, ignored test helper exports
- Added `playwright` to devDependencies
- Extracted keyboard handler (CRAP 172 → 26) into 5 focused functions
- Extracted context menu helpers (`buildContextMenuItem`, `clampToViewport`)
- Extracted `renderEntries` row creation (`createFileRow`, `bindRowEvents`)
- Extracted `startRename` into `createRenameInput` + `setupRenameEvents`
- Created `src/utils.ts` — shared `formatSize`, `formatDate`, `entryIcon`
- Deduplicated test setup with `src/test/helpers.ts`
- Added 6 keyboard navigation tests
- Set up `.gitignore`, committed, pushed to GitHub

**Metrics:**
| MI | Max CRAP (prod) | Dup % | Clone Groups | Tests |
|----|----------------|-------|--------------|-------|
| 79.1 → 93.9 | 172 → 26 | 9.8% → 1.7% | 11 → 2 | 39 → 45 |

**Decisions:**
- Compare chart cells against previous row (not baseline) — more signal
- Only colour numeric columns — date and notes stay neutral
- `playwright.tauri.cjs` is e2e, not prod — skip its CRAP score
- Per-cell colouring (green/red) instead of whole-row — mixed signals per row

**Left for next time:**
- Run e2e tests (`npm run test:e2e`)
- Consider `handleActionKeys` CRAP 26 (acceptable for now)
- 2 remaining clone groups (18 lines of test boilerplate)

**Files touched:** `app.ts`, `app.test.ts`, `app.integration.test.ts`, `keyboard.test.ts`, `utils.ts`, `test/helpers.ts`, `fallow.toml`, `fallow-progress.md`, `fallow-chart.html`, `package.json`, `CLAUDE.md`, `.gitignore`
