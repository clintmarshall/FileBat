# Future Ideas

## File Manager Feature Blueprint (TreeSize Inspired)

Building a file manager is a fantastic project because the core mechanics (CRUD operations) are well-known, leaving you free to innovate on user experience and data visualization. Following the **TreeSize Free/Pro** philosophy means transforming a passive file viewer into an active data-analytics and storage-optimisation tool.

Here are several high-utility, actionable feature suggestions to make your file manager stand out:

### 📊 Advanced Storage Analytics

* **Visual TreeMap Cushions:** Implement interactive, nested treemaps where block sizes represent file sizes.
* **Age and History Heatmaps:** Color-code directories by "last modified" or "last accessed" dates to visually spot ancient, forgotten data.
* **Growth Tracking:** Take scheduled snapshots of the file system to show users exactly which folders grew the most since last week.
* **File Type Breakdown:** Display dynamic pie or bar charts showing what percentage of space is consumed by video, audio, code, or system files.

### 🔍 Intelligent Cleanup Tools

* **Byte-Level Duplicate Finder:** Scan for duplicates using fast cryptographic hashing (like BLAKE3) rather than just matching file names.
* **Zero-Byte & Empty Folder Sweeper:** Identify and safely purge empty directory structures that clutter navigation.
* **"Orphaned" File Detector:** Highlight temporary files, cache folders, and broken shortcuts left behind by uninstalled software.

### ⚡ Power-User Navigation

* **Dual-Pane with Flat View:** Allow users to toggle a "Flat View" that strips away directory hierarchies, listing every single file inside subfolders in one master list.
* **Fuzzy Search & Command Palette:** Integrate a lightning-fast search index with a command palette (`Ctrl+K`) for keyboard-driven navigation.
* **Virtual Collections:** Let users group files from completely different physical drives into virtual folders without moving the actual data.

### 🛡️ Security & Integrity Inspection

* **Checksum Verification:** Build native tools to calculate and verify MD5, SHA-256, or SHA-512 hashes with a single click.
* **Deep Permissions Viewer:** Create a clear visual breakdown of inherited folder permissions, owner identities, and security flags.

---

## Architectural Architecture & Features for Tauri/Rust File Manager

Combining Rust with Tauri and TypeScript gives you a massive advantage: Rust provides bare-metal speed for heavy disk operations, while TypeScript handles complex UI rendering.

Because transferring massive file trees across the Tauri IPC (Inter-Process Communication) bridge can cause performance bottlenecks, you must balance where the work happens.

### 🏗️ Optimized Tauri Architecture

* **Parallel Disk Walker (Rust):** Use the `ignore` or `jwalk` crates in Rust. They use multi-threading to scan directories much faster than the standard library.
* **Streaming IPC Events:** Do not wait for a folder scan to finish before sending data to the UI. Stream directory chunks or individual file updates from Rust to TypeScript using Tauri's `emit` event system.
* **On-Demand UI Nodes:** Only send the top-level directories to TypeScript initially. Fetch subdirectories lazily from Rust when a user expands a folder tree node.
* **Window Pooling:** Store the heavy file metadata arrays inside a Rust global state using `tauri::State` wrapped in an `Arc<RwLock<T>>` to keep the frontend memory footprint low.

### 📊 Frontend Visualization (TypeScript)

* **Virtualised Lists:** Use libraries like `@tanstack/react-virtual` or `vue-virtual-scroller` to render thousands of files seamlessly without crashing the DOM.
* **D3.js / Treemap Component:** Use D3.js inside your TypeScript code to map the file-size weights into an interactive, zoomable HTML Canvas or SVG treemap.

### ⚡ Rust-Powered Features to Implement

* **BLAKE3 Duplicate Hashing:** Leverage the `blake3` crate. It is exceptionally fast and can hash files at disk-speed limits to find duplicate content instantly.
* **Cross-Drive Virtual File Systems:** Use Rust structs to create "virtual symlinks" so users can organize scattered files without modifying the actual disk structure.
* **Low-Level Metadata Extraction:** Use platform-specific crates (like `winapi` for Windows or `nix` for Unix) to grab deep file attributes, security descriptors, and hard link counts that standard tools miss.

## Refactoring Task: Break Down Large app.ts File

### Objective

The current `app.ts` file is 1200 lines long and handles too many responsibilities. Refactor this file into a modular, scalable TypeScript project structure following Clean Architecture principles.

### Target Project Structure

Create the following directory structure inside the `src/` folder:

* `src/routes/` – API route definitions and endpoint mappings.
* `src/controllers/` – HTTP request handlers (handles req, res, and status codes).
* `src/services/` – Core business logic and data processing rules.
* `src/models/` – Database schemas, types, and TypeScript interfaces.
* `src/middlewares/` – Authentication, validation, logging, and error handlers.
* `src/utils/` – Pure helper functions and configuration utilities.

### Refactoring Guidelines

1. **Keep app.ts Lean:** The final `app.ts` should only initialize Express, apply global middleware, register top-level routes, and export or start the server.
2. **Decouple Logic:** Controllers must not write directly to the database; they must call Services. Services must not handle HTTP `req` or `res` objects.
3. **Preserve Functionality:** Do not alter the existing runtime logic, variable names, database queries, or API endpoints. Only move them to their appropriate modules.
4. **Type Safety:** Ensure all extracted modules maintain strict TypeScript types and explicit imports/exports.

### Output Requirements

Provide the refactored code step-by-step:

1. List the files to be created.
2. Provide the complete code for each new module.
3. Provide the final, slimmed-down version of `app.ts`.
