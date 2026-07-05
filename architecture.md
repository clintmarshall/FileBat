# FileBitch — Architecture

> **Maintainer note:** This document must be kept up to date as the project evolves. When adding a feature, changing a layer, or introducing a new dependency, update the relevant section.

---

## What It Is

A fast, Windows-style file explorer with built-in disk analytics (usage over time, large files, duplicates). Built for speed — small binary, direct filesystem access, streaming results.

## Tech Stack

| Layer | Technology | Why |
|-------|-----------|-----|
| Framework | **Tauri 2** | ~5-10MB binary, native windowing, direct Rust FS access |
| Backend | **Rust 1.95** | Performance, memory safety, zero-cost abstractions |
| Frontend | **Vanilla TypeScript + CSS** | Simple state model, no framework overhead |
| Dev Server | **Vite** | Fast HMR, TypeScript compilation, serves on `:1420` |
| Database | **SQLite** (via `sqlx`) | Analytics history queries, lazy-init |

---

## Layered Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                     PRESENTATION LAYER                          │
│  Frontend (HTML/CSS/TypeScript)                                 │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌─────────────────┐    │
│  │ Sidebar  │ │ File List │ │ Toolbar  │ │ Analytics Panel │    │
│  │ (Tree)   │ │ (Virtual) │ │ (Nav)    │ │ (Tabbed)        │    │
│  └──────────┘ └──────────┘ └──────────┘ └─────────────────┘    │
│                                                                  │
│  ← Virtual scrolling for large directories                      │
│  ← Windows 11-inspired CSS (Segoe UI, subtle colors)            │
├─────────────────────────────────────────────────────────────────┤
│                      APPLICATION LAYER                          │
│  Fast paths:   Tauri Command → Use Case → Return                │
│  Slow paths:   Tauri Command → Spawn Thread → Tauri Events      │
│                                                                  │
│  Streaming pattern for heavy ops (scan, duplicates):            │
│    1. Command spawns background thread, returns scan_id          │
│    2. Backend emits: scan:progress, scan:chunk, scan:complete   │
│    3. Frontend subscribes and updates incrementally              │
│    4. User can cancel via command(scan_id)                       │
├─────────────────────────────────────────────────────────────────┤
│                        DOMAIN LAYER                             │
│  Traits:    FileSystemRepo, AnalyticsRepo                       │
│  Models:    Entry, EntryType, Volume                            │
│           : FolderUsage, DuplicateGroup, UsageSnapshot          │
│  Errors:    AppError (PermissionDenied, NotFound, Io, ...)      │
│                                                                  │
│  ← Zero knowledge of Tauri, std::fs, or the UI                  │
│  ← Pure Rust — testable in isolation                            │
├─────────────────────────────────────────────────────────────────┤
│                    INFRASTRUCTURE LAYER                         │
│  FileSystem: StdFileSystem (std::fs + jwalk + notify)           │
│  Analytics : SqliteAnalytics (sqlx + sqlite)                    │
│                                                                  │
│  ← Implements Domain traits using OS/database primitives        │
│  ← SQLite lazy-init — only opens when Analytics panel first used│
│  ← Icon cache: HashMap<PathBuf, IconHandle>                     │
└─────────────────────────────────────────────────────────────────┘
```

### Dependency Flow

```
Commands → UseCases → Domain (traits) ← Infrastructure (impls)
     ↓
   Tauri IPC ←────────────────────────────────┘
```

**Rules:**
- Domain knows nothing about infrastructure or presentation
- Infrastructure implements Domain traits
- UseCases orchestrate repos + apply business rules
- Commands are thin IPC bridges — no business logic

---

## Source Layout

```
filebitch/
├── Cargo.toml                      # Workspace root
├── package.json                    # Node deps (vite, tauri, typescript)
├── tsconfig.json
├── vite.config.ts                  # Vite dev server (:1420)
├── architecture.md                 # ← This file
│
├── src/                            # Frontend
│   ├── index.html                  # 4-pane Explorer layout
│   ├── app.ts                      # State, navigation, keyboard, Tauri bridge
│   ├── styles/main.css             # Windows 11 theme (CSS custom properties)
│
├── src-tauri/                      # Rust Backend
│   ├── Cargo.toml                  # Dependencies
│   ├── tauri.conf.json             # Window config, devUrl, frontendDist
│   ├── build.rs
│   ├── capabilities/default.json   # Tauri 2 permissions
│   ├── icons/icon.ico
│   │
│   └── src/
│       ├── main.rs                 # Bootstrap — wires layers together
│       │
│       ├── domain/                 # ← Pure domain, zero infra deps
│       │   ├── mod.rs
│       │   ├── models.rs           # Entry, EntryType, Volume, FolderUsage, DuplicateGroup, UsageSnapshot, scan models
│       │   └── repos.rs            # FileSystemRepo, AnalyticsRepo traits, AppError enum
│       │
│       ├── infrastructure/         # ← Concrete implementations
│       │   ├── mod.rs
│       │   ├── filesystem.rs       # StdFileSystem : impl FileSystemRepo
│       │   └── analytics_db.rs     # SqliteAnalytics : impl AnalyticsRepo (lazy-init SQLite)
│       │
│       ├── usecases/               # ← Business logic
│       │   ├── mod.rs
│       │   ├── navigation.rs       # NavigationUseCase<R> — sorting, orchestration
│       │   ├── file_operations.rs  # FileOperationsUseCase<R> — rename, delete, copy, move
│       │   └── analytics/          # AnalyticsUseCase — scan orchestration + MPSC streaming
│       │       ├── mod.rs          #   AnalyticsUseCase — scan lifecycle (register/cancel/unregister)
│       │       ├── aggregator.rs   #   FolderUsageAccumulator, SizeGrouping, HashGrouping — pure math
│       │       ├── disk_usage.rs   #   DiskUsageUseCase — walkdir + MPSC channel emit
│       │       ├── large_files.rs  #   LargeFilesUseCase — walkdir + MPSC channel emit
│       │       ├── duplicates.rs   #   DuplicatesUseCase — 3-stage funnel + MPSC channel emit
│       │       └── snapshot.rs     #   SnapshotUseCase — SQLite save/query delegation
│       │
│       └── commands/               # ← Tauri IPC wrappers
│           ├── mod.rs
│           ├── navigation.rs       # list_dir, get_volumes
│           ├── fileops.rs          # rename, delete, create_folder, copy_items, move_items
│           └── scan.rs             # start_scan_usage, start_find_large_files, start_find_duplicates, cancel_scan, snapshot_usage, usage_history
```

---

## Key Design Decisions

### 1. Tauri 2 over egui/Slint
User wants a real file explorer. Tauri gives native windowing, CSS styling freedom, tiny footprint (~5MB vs Electron's 150MB).

### 2. Vanilla TypeScript over React/Vue
The state model is a flat tree + list. No complex reactivity needed. Less dependency surface = faster app.

### 3. jwalk over walkdir
`jwalk` uses a thread pool for parallel directory traversal. Essential for analytics scans on large directory trees. `walkdir` is single-threaded and would freeze on `C:\`.

### 4. Tauri Events for streaming (via MPSC channels)
Heavy ops (scan, duplicates) spawn a blocking thread and return `scan_id` immediately. Results stream as events — no blocking IPC, no massive JSON payloads.

**MPSC Channel Pattern:** `window.emit()` is never called from inside `spawn_blocking`. Instead:
1. Blocking thread walks the filesystem and pushes `ScanStep` variants into a `tokio::sync::mpsc::unbounded_channel`
2. A lightweight `tokio::spawn` async task drains the channel and calls `window.emit()` on the safe executor thread
3. `run()` returns a `oneshot`-backed future that resolves when emission is complete, so `AnalyticsUseCase` can unregister the scan at the right time

This prevents the blocking thread pool from starving the Tokio executor and freezing the IPC bus.

### 5. Three-stage duplicate detection
1. **Group by exact size** — O(1) metadata, eliminates 99% of non-duplicates
2. **Hash first 8KB** — cheap, catches most false positives
3. **Full SHA-256** — expensive but rare by this point

Same approach as `fdupes` and `rdfind`.

### 6. Virtual scrolling
File list only renders visible DOM rows. A folder with 50,000 files renders ~20 rows.

### 7. SQLite lazy-init
Analytics DB opens only when the user first opens the Analytics panel. Keeps cold start fast.

### 8. notify for live updates
Native filesystem watch events update the current folder view without polling.

### 9. Icon caching
`HashMap<PathBuf, IconHandle>` in the backend. Icons are base64-encoded and sent once per file type.

### 10. Three-phase disk usage scanning
**Phase 1 — Structure (BFS readdir, pull-based):** Breadth-first discovery using `readdir` only. Zero file stats, zero `metadata()` calls. Emits `scan:tree_started` (root info) at start, then `scan:children_ready` per folder as children are discovered. Frontend renders the root, then fetches children on expand via `get_scan_tree_children` command. Tree stored in backend memory — O(children) IPC per expand, not O(entire tree).

**Phase 2 — Leaf Sizing (parallel, 10 threads):** Only folders with no subdirectories (leaves) are queued for sizing. Each thread walks one leaf folder with WalkDir, counting files, subfolders, and total size. Every file on disk is visited exactly once. Results are emitted as `scan:chunk` events and stored in a shared HashMap.

**Phase 3 — Rollup (bottom-up):** After all leaves are sized, walk the tree bottom-up. Each parent's `size`, `fileCount`, `folderCount` = sum of its direct children's values. Emit `scan:chunk` for each parent so the frontend patches all rows.

**Why not walk every folder?** The previous approach walked every folder's entire subtree. A file in `E:/Users/Clint/docs/notes.txt` was visited 4+ times (once per ancestor). With leaf-only sizing + rollup, every file is visited exactly once. Parent totals are a cheap in-memory sum.

---

## Dependencies

### Rust (`src-tauri/Cargo.toml`)

| Crate | Purpose |
|-------|---------|
| `tauri = "2"` | Framework — IPC, windowing, events |
| `jwalk = "0.8"` | Multi-threaded directory walking |
| `notify = "6.1"` | Native OS filesystem watching |
| `sqlx = "0.8"` | SQLite analytics DB (`runtime-tokio`, `sqlite`) |
| `serde = "1.0"` | Frontend/Backend serialization |
| `serde_json = "1.0"` | JSON for IPC payloads |
| `sha2 = "0.10"` | Duplicate detection hashing |
| `chrono = "0.4"` | Timestamps (`serde` feature) |
| `thiserror = "2.0"` | Error derive macros |
| `tokio = "1"` | Async runtime (`full` features) |
| `dirs = "5.0"` | Cross-platform data directory resolution |

### Frontend (`package.json`)

| Package | Purpose |
|---------|---------|
| `@tauri-apps/api` | Rust↔JS bridge (`invoke`, `emit`, `listen`) |
| `vite` | Dev server + TypeScript compilation |
| `typescript` | Type safety |
| `@tauri-apps/cli` | Tauri dev/build commands |

---

## Tauri Commands (Current)

| Command | Args | Returns | Layer |
|---------|------|---------|-------|
| `list_dir` | `{ path: String }` | `Result<Vec<Entry>, String>` | navigation |
| `get_volumes` | _(none)_ | `Result<Vec<Volume>, String>` | navigation |
| `rename` | `{ path, newName }` | `Result<(), String>` | fileops |
| `delete` | `{ paths: Vec<String> }` | `Result<Vec<String>, String>` | fileops |
| `create_folder` | `{ parentPath, name }` | `Result<(), String>` | fileops |
| `copy_items` | `{ sources, destDir }` | `Result<Vec<String>, String>` | fileops |
| `move_items` | `{ sources, destDir }` | `Result<Vec<String>, String>` | fileops |
| `start_scan_usage` | `{ path, maxDepth }` | `Result<String, String>` (scan_id) | scan |
| `start_find_large_files` | `{ path, minSize, maxResults }` | `Result<String, String>` (scan_id) | scan |
| `start_find_duplicates` | `{ path }` | `Result<String, String>` (scan_id) | scan |
| `cancel_scan` | `{ scanId }` | `bool` | scan |
| `snapshot_usage` | `{ path, totalSize, fileCount, folderCount, topFolders }` | `Result<UsageSnapshot, String>` | scan |
| `usage_history` | `{ path, start, end }` | `Result<Vec<UsageSnapshot>, String>` | scan |

---

## Domain Models

### Entry
```rust
pub struct Entry {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub modified: String,          // ISO 8601 format
    pub entry_type: EntryType,     // Folder | Drive | Symlink | File
    pub extension: Option<String>,
}
```

### Volume
```rust
pub struct Volume {
    pub name: String,   // "C:"
    pub path: String,   // "C:\\"
}
```

### AppError
```rust
pub enum AppError {
    PermissionDenied(String),
    NotFound(String),
    Io(std::io::Error),
    Database(String),
    ScanCancelled,
    Other(String),
}
```

### FolderUsage
```rust
pub struct FolderUsage {
    pub path: String,
    pub size: u64,
    pub file_count: u64,
    pub folder_count: u64,
}
```

### FolderStructure (Tree Shape)
```rust
pub struct FolderStructure {
    pub path: String,
    pub name: String,
    pub children: Vec<String>,  // Direct child folder paths
}
```

### ScanStructure (Phase 1 Output)
```rust
pub struct ScanStructure {
    pub scan_id: String,
    pub root_path: String,
    pub folders: Vec<FolderStructure>,
    pub total_folders: usize,
}
```

### DuplicateGroup
```rust
pub struct DuplicateGroup {
    pub hash: String,
    pub size_each: u64,
    pub files: Vec<String>,
    pub wasted_space: u64,  // size_each * (files.len() - 1)
}
```

### UsageSnapshot
```rust
pub struct UsageSnapshot {
    pub id: i64,
    pub path: String,
    pub total_size: u64,
    pub file_count: u64,
    pub folder_count: u64,
    pub top_folders: String,  // JSON array of FolderUsage
    pub scanned_at: String,   // ISO 8601 UTC
}
```

### Streaming Models
```rust
pub struct ScanProgress { scan_id, percentage, message }
pub struct ScanChunk    { scan_id, data: ScanChunkData }
pub struct ScanComplete { scan_id, total_items, total_size, duration_ms }
pub struct ScanError    { scan_id, message }

pub enum ScanChunkData {
    FolderUsage { usage: FolderUsage },
    LargeFile { entry: Entry },
    DuplicateGroup { group: DuplicateGroup },
}
```

---

## Frontend Architecture

### State Model
```typescript
interface AppState {
    history: string[];       // Navigation history (paths)
    historyIndex: number;    // Current position in history
    selectedIndex: number;   // Currently selected row index
}
```

### Component Structure
- `app.ts` — Single entry point. Manages state, renders, handles keyboard.
- No component framework — DOM manipulation is straightforward for a file list + tree.
- Future: Extract `sidebar.ts`, `filelist.ts`, `analytics.ts` as the app grows.

### Tauri Bridge
```typescript
import { invoke } from '@tauri-apps/api/core';

// Usage:
const entries: Entry[] = await invoke('list_dir', { path: 'C:\\' });
```

Vite is configured to skip bundling `@tauri-apps/api` — the API is injected by the Tauri webview at runtime.

---

## Dev Workflow

```bash
# Terminal 1: Start Vite (compiles TS, serves on :1420)
npx vite

# Terminal 2: Start Tauri (connects to Vite, launches native window)
npx tauri dev
```

Tauri watches `src-tauri/` for Rust changes and hot-reloads. Vite HMR handles frontend changes.

### Build for Production
```bash
npx tauri build    # Produces installer in src-tauri/target/release/bundle/
```

---

## Testing Strategy

### Domain Layer
Unit tests on models and sorting logic — no dependencies needed.

```rust
#[test]
fn sort_entries_folders_first() {
    let mut entries = vec![file_entry, folder_entry];
    sort_entries(&mut entries);
    assert_eq!(entries[0].entry_type, EntryType::Folder);
}
```

### Use Cases
Mock `FileSystemRepo` → test sorting, filtering, error handling without a real filesystem.

### Commands
Integration tests via Tauri's test harness — verify IPC serialization.

---

## Performance Mandates

| Operation | Target | Strategy |
|-----------|--------|----------|
| Cold start | < 500ms | SQLite lazy-init, minimal startup work |
| List directory (10K files) | < 100ms | Metadata-only, no hashing |
| Scan 100GB drive | < 30s | jwalk parallel traversal |
| Find duplicates (1M files) | < 2min | 3-stage funnel, rayon parallel |
| UI responsiveness | 60fps | Virtual scrolling, streaming events |

---

## Future Considerations

- **File type icons:** Windows shell icons via `windows` crate or embedded SVG set
- **Drag-and-drop:** Between folders within the app
- **Search/filter bar:** Content search within current directory
- **Settings:** View preferences, scan paths, theme toggle
- **Dark/light theme:** CSS custom properties + system preference detection
- **Multi-window:** Multiple explorer windows (Tauri supports this natively)
