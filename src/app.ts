// FileBitch — Main Application Entry Point
// Tauri 2 + Vanilla TypeScript

// @ts-ignore - Tauri API is injected at runtime by the webview
import { invoke } from '@tauri-apps/api/core';
// @ts-ignore - Tauri API is injected at runtime by the webview
import { listen } from '@tauri-apps/api/event';
import { formatSize, formatDate, entryIcon } from './utils';

// ─── DOM References ───

const fileListEl = document.getElementById('file-list')!;
const breadcrumbEl = document.getElementById('breadcrumb')!;
const drivesEl = document.getElementById('drives')!;
const statusInfoEl = document.getElementById('status-info')!;
const statusPathEl = document.getElementById('status-path')!;
const btnBack = document.getElementById('btn-back')! as HTMLButtonElement;
const btnForward = document.getElementById('btn-forward')! as HTMLButtonElement;
const btnUp = document.getElementById('btn-up')! as HTMLButtonElement;

// ─── Types ───

interface Entry {
  name: string;
  path: string;
  size: number;
  modified: string;
  entryType: 'File' | 'Folder' | 'Symlink' | 'Drive';
  extension: string | null;
}

interface Volume {
  name: string;
  path: string;
}

// ─── Usage Tree Types (NodeId-based) ───

/// One node in the tree store — keyed by NodeId.
/// Children are pulled on demand via get_scan_tree_children.
interface TreeNodeData {
    childCount: number;
    children?: Array<{ id: number; name: string }>;
    stats?: { size: number; fileCount: number; folderCount: number };
}

/// Path info for a NodeId — resolved lazily from backend or during scan.
interface PathInfo {
    path: string;
    name: string;
}

// ─── State ───

const history: string[] = [];
let historyIndex = -1;
let currentPath: string | null = null;
let currentEntries: Entry[] = [];

// Multi-select: Set of selected indices
const selectedIndices = new Set<number>();
let lastSelectedIndex = -1;       // For Shift+click range selection

// Clipboard for copy/move
let clipboard: { paths: string[]; mode: 'copy' | 'move' } | null = null;

// Rename state
let renamingRow: HTMLElement | null = null;
let renameInput: HTMLInputElement | null = null;

// ─── Analytics State ───

let analyticsVisible = false;
let activeScanId: string | null = null;
let currentTab: string = 'usage';
let scanResults: {
    largeFiles: Entry[];
    duplicates: Array<{ hash: string; sizeEach: number; files: string[]; wastedSpace: number }>;
} = { largeFiles: [], duplicates: [] };

// Tree expansion state — keyed by NodeId
const expandedPaths = new Set<number>();

// Single tree store — keyed by NodeId.
// Populated by scan:children_ready (childCount) and scan:chunk (stats).
// Children pulled on demand via get_scan_tree_children.
const treeStore = new Map<number, TreeNodeData>();

// Path resolution — NodeId → {path, name}. Populated during scan events.
const pathMap = new Map<number, PathInfo>();

// Observability — running max folder size (avoids O(n²) scan on every patch).
let maxFolderSize = 0;
let foldersSized = 0;

// ─── Event Batching ───
// Events arrive faster than the main thread can process them.
// Buffer them and flush once per animation frame to keep the UI responsive.

interface PendingChildren {
    parentId: number;
    childCount: number;
}

interface PendingStats {
    nodeId: number;
    size: number;
    fileCount: number;
    folderCount: number;
}

let pendingChildren: PendingChildren[] = [];
let pendingStats: PendingStats[] = [];
let pendingProgress: { percentage: number; message: string } | null = null;
let flushScheduled = false;

/// Update a single size bar and recalculate all siblings using the running total
/// of sized children. Bars grow progressively as siblings get sized, then settle
/// when the parent is fully sized (parentSize === runningSize).
function updateBar(bar: HTMLElement, nameCell: HTMLElement) {
    const size = parseFloat(bar.dataset.size || '0');
    if (size <= 0) {
        bar.style.width = '0%';
        return;
    }
    bar.dataset.size = String(size);
    recalcSiblings(siblingsContainerOf(nameCell));
}

/// Recalculate all bars in a children container.
/// Uses parentSize if known (scan complete for this folder),
/// otherwise uses runningSize (sum of sized children so far).
function recalcSiblings(container: HTMLElement) {
    const parentSize = parseFloat(container.dataset.parentSize || '0');
    const denom = parentSize > 0 ? parentSize : parseFloat(container.dataset.runningSize || '0');
    if (denom <= 0) return;
    const bars = container.querySelectorAll('.tree-size-bar') as NodeListOf<HTMLElement>;
    for (let i = 0; i < bars.length; i++) {
        const s = parseFloat(bars[i].dataset.size || '0');
        bars[i].style.width = `${Math.min(100, Math.max(0, (s / denom) * 100))}%`;
    }
}

/// Find the children container that holds this row and its siblings.
/// DOM: nameCell → row → wrapper → childrenContainer (sibling rows live here).
function siblingsContainerOf(nameCell: HTMLElement): HTMLElement {
    const row = nameCell.parentElement ?? nameCell.closest('.usage-tree-row');
    const wrapper = row?.parentElement;
    return wrapper?.parentElement ?? nameCell;
}

/// Flush all pending events in a single batch.
/// Called once per animation frame to minimize DOM thrashing.
function flushPendingEvents() {
    const start = performance.now();

    // Flush children_ready events (thin: parentId + childCount only)
    const childrenToFlush = pendingChildren;
    pendingChildren = [];

    for (const item of childrenToFlush) {
        const existing = treeStore.get(item.parentId);
        treeStore.set(item.parentId, {
            childCount: item.childCount,
            children: existing?.children,
            stats: existing?.stats,
        });
        enableExpandButton(item.parentId, item.childCount);
    }

    // Flush stats events (NodeId-based)
    const statsToFlush = pendingStats;
    pendingStats = [];

    for (const usage of statsToFlush) {
        if (usage.size > maxFolderSize) {
            maxFolderSize = usage.size;
        }
        foldersSized += 1;

        const existing = treeStore.get(usage.nodeId);
        treeStore.set(usage.nodeId, {
            childCount: existing?.childCount ?? 0,
            children: existing?.children,
            stats: { size: usage.size, fileCount: usage.fileCount, folderCount: usage.folderCount },
        });

        // Find row by data-node-id attribute (O(1), no path normalization)
        const row = document.querySelector(`.usage-tree-row[data-node-id="${usage.nodeId}"]`) as HTMLElement;
        if (!row) continue;

        const sizeEl = row.querySelector('.tree-size') as HTMLElement;
        const barEl = row.querySelector('.tree-size-bar') as HTMLElement;
        const filesEl = row.querySelector('.tree-files') as HTMLElement;
        const foldersEl = row.querySelector('.tree-folders') as HTMLElement;

        if (sizeEl) sizeEl.textContent = formatSize(usage.size);
        if (barEl) {
            barEl.dataset.size = String(usage.size);
            const nameCell = row.querySelector('.tree-name-cell') as HTMLElement;

            // Update runningSize on the container that holds this row (siblings container)
            const siblingContainer = siblingsContainerOf(nameCell);
            const currentRunning = parseFloat(siblingContainer.dataset.runningSize || '0');
            siblingContainer.dataset.runningSize = String(currentRunning + usage.size);

            updateBar(barEl, nameCell);
        }

        // Highlight the row while being processed
        row.classList.add('scanning');

        // Update children container so children bars can recalculate
        const childrenContainer = row.nextElementSibling as HTMLElement | null;
        if (childrenContainer) {
            childrenContainer.dataset.parentSize = String(usage.size);
            recalcSiblings(childrenContainer);
            // Remove scanning from children now that parent is fully sized
            const childRows = childrenContainer.querySelectorAll('.usage-tree-row.scanning');
            for (let i = 0; i < childRows.length; i++) {
                (childRows[i] as HTMLElement).classList.remove('scanning');
            }
        }
        if (filesEl) filesEl.textContent = usage.fileCount.toLocaleString();
        if (foldersEl) foldersEl.textContent = usage.folderCount.toLocaleString();
    }

    // Flush progress
    if (pendingProgress) {
        progressFill.style.width = `${pendingProgress.percentage}%`;
        progressText.textContent = pendingProgress.message;
        statusInfoEl.textContent = pendingProgress.message;
        pendingProgress = null;
    }

    const elapsed = performance.now() - start;
    updatePerfOverlay({
        childrenFlushed: childrenToFlush.length,
        statsFlushed: statsToFlush.length,
        flushMs: elapsed.toFixed(1),
        queueDepth: pendingChildren.length + pendingStats.length,
    });
}

/// Schedule a flush on the next animation frame.
function scheduleFlush() {
    if (!flushScheduled) {
        flushScheduled = true;
        requestAnimationFrame(() => {
            flushScheduled = false;
            flushPendingEvents();
        });
    }
}

// ─── Perf Overlay ───

const perfOverlayEl = document.getElementById('perf-overlay') as HTMLElement | null;
let perfFrameCount = 0;
let perfLastFpsTime = performance.now();
let perfCurrentFps = 60;

/// Update the visible performance overlay.
function updatePerfOverlay(data: {
    childrenFlushed: number;
    statsFlushed: number;
    flushMs: string;
    queueDepth: number;
}) {
    if (!perfOverlayEl) return;

    perfFrameCount++;
    const now = performance.now();
    if (now - perfLastFpsTime >= 1000) {
        perfCurrentFps = Math.round(perfFrameCount * 1000 / (now - perfLastFpsTime));
        perfFrameCount = 0;
        perfLastFpsTime = now;
    }

    const color = perfCurrentFps < 20 ? '#f44' : perfCurrentFps < 40 ? '#fa0' : '#888';
    perfOverlayEl.innerHTML = `
        <span style="color:${color};font-size:11px;font-family:monospace">
        FPS:${perfCurrentFps} | Q:${data.queueDepth} | C:${data.childrenFlushed} | S:${data.statsFlushed} | ${data.flushMs}ms
        </span>
    `;
}

function getSelectedPaths(): string[] {
  const paths: string[] = [];
  for (const idx of selectedIndices) {
    if (currentEntries[idx]) {
      paths.push(currentEntries[idx].path);
    }
  }
  return paths;
}

function getSelectedEntries(): Entry[] {
  const entries: Entry[] = [];
  for (const idx of selectedIndices) {
    if (currentEntries[idx]) {
      entries.push(currentEntries[idx]);
    }
  }
  return entries;
}

// ─── Context Menu ───

let contextMenu: HTMLElement | null = null;

function createContextMenu() {
  const menu = document.createElement('div');
  menu.id = 'context-menu';
  menu.style.display = 'none';
  document.body.appendChild(menu);
  contextMenu = menu;
  return menu;
}

function buildContextMenuItem(item: ContextMenuItem): HTMLElement {
  const el = document.createElement('div');
  el.className = 'ctx-item';

  if (item.disabled) el.classList.add('disabled');
  if (item.danger) el.classList.add('danger');

  if (item.separator) {
    el.classList.add('separator');
    el.textContent = '';
    return el;
  }

  el.textContent = item.label || '';
  if (item.shortcut) {
    el.innerHTML = `${item.label || ''} <span class="ctx-shortcut">${item.shortcut}</span>`;
  }

  if (!item.disabled && item.action) {
    el.addEventListener('click', () => {
      hideContextMenu();
      item.action!();
    });
  }

  return el;
}

function clampToViewport(x: number, y: number, element: HTMLElement): { x: number; y: number } {
  const rect = element.getBoundingClientRect();
  const posX = x + rect.width > window.innerWidth ? window.innerWidth - rect.width - 4 : x;
  const posY = y + rect.height > window.innerHeight ? window.innerHeight - rect.height - 4 : y;
  return { x: posX, y: posY };
}

function showContextMenu(x: number, y: number, items: ContextMenuItem[]) {
  if (!contextMenu) return;

  contextMenu.innerHTML = '';
  items.forEach((item) => contextMenu.appendChild(buildContextMenuItem(item)));

  const pos = clampToViewport(x, y, contextMenu);
  contextMenu.style.left = `${pos.x}px`;
  contextMenu.style.top = `${pos.y}px`;
  contextMenu.style.display = 'block';
}

function hideContextMenu() {
  if (contextMenu) contextMenu.style.display = 'none';
}

interface ContextMenuItem {
  label?: string;
  shortcut?: string;
  disabled?: boolean;
  danger?: boolean;
  separator?: boolean;
  action?: () => void;
}

// ─── Rename (F2 / inline) ───

function createRenameInput(entry: Entry): HTMLInputElement {
  const input = document.createElement('input');
  input.type = 'text';
  input.value = entry.name;
  input.style.cssText = `
    flex: 1; border: 2px solid var(--accent); outline: none;
    font: inherit; padding: 0 4px; border-radius: 2px;
  `;
  return input;
}

function setupRenameEvents(
  input: HTMLInputElement,
  entry: Entry,
  oldName: string,
) {
  async function finishRename() {
    const newName = input.value.trim();
    if (newName && newName !== oldName) {
      try {
        await invoke('rename', { path: entry.path, newName });
        if (currentPath) await navigateTo(currentPath);
      } catch (err) {
        statusInfoEl.textContent = `Rename error: ${err}`;
        if (currentPath) await navigateTo(currentPath);
      }
    } else {
      if (currentPath) await navigateTo(currentPath);
    }
  }

  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') { e.preventDefault(); finishRename(); }
    else if (e.key === 'Escape') { if (currentPath) navigateTo(currentPath); }
  });

  input.addEventListener('blur', finishRename);
}

async function startRename(index: number) {
  const entry = currentEntries[index];
  if (!entry) return;

  const row = fileListEl.querySelectorAll('.file-item')[index];
  if (!row) return;

  const nameSpan = row.querySelector('.col-name')!;
  const iconSpan = nameSpan.querySelector('.icon')!;

  const input = createRenameInput(entry);
  nameSpan.innerHTML = '';
  nameSpan.appendChild(iconSpan);
  nameSpan.appendChild(input);
  row.classList.add('renaming');

  renamingRow = row as HTMLElement;
  renameInput = input;

  const dotIdx = entry.entryType === 'File' ? entry.name.lastIndexOf('.') : -1;
  input.setSelectionRange(0, dotIdx >= 0 ? dotIdx : entry.name.length);
  input.focus();

  setupRenameEvents(input, entry, entry.name);
}

// ─── File Operations ───

async function handleDelete() {
  const paths = getSelectedPaths();
  if (paths.length === 0) return;

  try {
    await invoke('delete', { paths });
    if (currentPath) await navigateTo(currentPath);
  } catch (err) {
    statusInfoEl.textContent = `Delete error: ${err}`;
  }
}

async function handleCopy() {
  const paths = getSelectedPaths();
  if (paths.length === 0) return;
  clipboard = { paths, mode: 'copy' };
  statusInfoEl.textContent = `Copied ${paths.length} item${paths.length > 1 ? 's' : ''}`;
}

async function handleCut() {
  const paths = getSelectedPaths();
  if (paths.length === 0) return;
  clipboard = { paths, mode: 'move' };
  statusInfoEl.textContent = `Cut ${paths.length} item${paths.length > 1 ? 's' : ''}`;
}

async function handlePaste() {
  if (!clipboard || !currentPath) return;

  try {
    if (clipboard.mode === 'copy') {
      await invoke('copy_items', { sources: clipboard.paths, destDir: currentPath });
    } else {
      await invoke('move_items', { sources: clipboard.paths, destDir: currentPath });
    }
    clipboard = null;
    if (currentPath) await navigateTo(currentPath);
  } catch (err) {
    statusInfoEl.textContent = `Paste error: ${err}`;
  }
}

async function handleNewFolder() {
  if (!currentPath) return;

  try {
    await invoke('createFolder', { parentPath: currentPath, name: 'New Folder' });
    // Start renaming the new folder immediately
    await navigateTo(currentPath);
    // Find the new folder and start renaming
    const idx = currentEntries.findIndex((e) => e.name === 'New Folder');
    if (idx >= 0) {
      selectedIndices.clear();
      selectedIndices.add(idx);
      renderSelection();
      startRename(idx);
    }
  } catch (err) {
    statusInfoEl.textContent = `Create folder error: ${err}`;
  }
}

async function handleOpenWith() {
  const entries = getSelectedEntries();
  if (entries.length === 0) return;
  // TODO: Open with default app via tauri-plugin-shell
  statusInfoEl.textContent = 'Open with — coming soon';
}

// ─── Render ───

function createFileRow(entry: Entry, index: number): HTMLElement {
  const row = document.createElement('div');
  row.className = 'file-item';
  row.dataset.index = String(index);
  row.dataset.path = entry.path;
  row.dataset.type = entry.entryType;

  row.innerHTML = `
    <span class="col-name">
      <span class="icon">${entryIcon(entry.entryType)}</span>
      ${entry.name}
    </span>
    <span class="col-size">${entry.entryType === 'Folder' ? '' : formatSize(entry.size)}</span>
    <span class="col-date">${formatDate(entry.modified)}</span>
  `;

  return row;
}

function bindRowEvents(row: HTMLElement, entry: Entry, index: number) {
  // Click — selection
  row.addEventListener('click', (e) => {
    if (renamingRow) return;

    if (e.ctrlKey || e.metaKey) {
      toggleSelect(index);
    } else if (e.shiftKey && lastSelectedIndex >= 0) {
      rangeSelect(Math.min(lastSelectedIndex, index), Math.max(lastSelectedIndex, index));
    } else {
      singleSelect(index);
    }
  });

  // Double click — open
  row.addEventListener('dblclick', () => {
    if (entry.entryType === 'Folder' || entry.entryType === 'Drive') {
      navigateTo(entry.path);
    }
  });

  // Right click — context menu
  row.addEventListener('contextmenu', (e) => {
    e.preventDefault();
    if (!selectedIndices.has(index)) singleSelect(index);

    const selected = getSelectedEntries();
    const allFolders = selected.length > 0 && selected.every((s) => s.entryType === 'Folder');

    const items: ContextMenuItem[] = [
      { label: 'Open', disabled: !allFolders, action: handleOpenWith },
      { separator: true },
      { label: 'Rename', shortcut: 'F2', action: () => startRename(index) },
      { label: 'Copy', shortcut: 'Ctrl+C', action: handleCopy },
      { label: 'Cut', shortcut: 'Ctrl+X', action: handleCut },
      { separator: true },
      { label: 'Delete', shortcut: 'Del', danger: true, action: handleDelete },
    ];

    showContextMenu(e.clientX, e.clientY, items);
  });
}

function renderEntries(entries: Entry[]) {
  currentEntries = entries;
  fileListEl.innerHTML = '';

  if (entries.length === 0) {
    fileListEl.innerHTML = '<div class="empty-state">This folder is empty</div>';
    statusInfoEl.textContent = '0 items';
    renderSelection();
    return;
  }

  for (let i = 0; i < entries.length; i++) {
    const row = createFileRow(entries[i], i);
    bindRowEvents(row, entries[i], i);
    fileListEl.appendChild(row);
  }

  renderSelection();
  updateStatus(entries);
}

function renderSelection() {
  const rows = fileListEl.querySelectorAll('.file-item');
  rows.forEach((row, i) => {
    if (selectedIndices.has(i)) {
      row.classList.add('selected');
    } else {
      row.classList.remove('selected');
    }
  });

  const count = selectedIndices.size;
  if (count > 0 && currentEntries.length > 0) {
    const totalSize = getSelectedEntries()
      .filter((e) => e.entryType === 'File')
      .reduce((sum, e) => sum + e.size, 0);
    statusInfoEl.textContent =
      `${count} item${count !== 1 ? 's' : ''} selected · ${formatSize(totalSize)}`;
  }
}

function updateStatus(entries: Entry[]) {
  if (selectedIndices.size === 0) {
    const fileCount = entries.filter((e) => e.entryType === 'File').length;
    const folderCount = entries.length - fileCount;
    statusInfoEl.textContent =
      `${entries.length} item${entries.length !== 1 ? 's' : ''} · ${fileCount} file${fileCount !== 1 ? 's' : ''} · ${folderCount} folder${folderCount !== 1 ? 's' : ''}`;
  }
}

// ─── Selection ───

function singleSelect(index: number) {
  selectedIndices.clear();
  selectedIndices.add(index);
  lastSelectedIndex = index;
  renderSelection();
}

function toggleSelect(index: number) {
  if (selectedIndices.has(index)) {
    selectedIndices.delete(index);
  } else {
    selectedIndices.add(index);
  }
  lastSelectedIndex = index;
  renderSelection();
}

function rangeSelect(from: number, to: number) {
  selectedIndices.clear();
  for (let i = from; i <= to; i++) {
    selectedIndices.add(i);
  }
  lastSelectedIndex = to;
  renderSelection();
}

function selectAll() {
  for (let i = 0; i < currentEntries.length; i++) {
    selectedIndices.add(i);
  }
  renderSelection();
}

// ─── Navigation ───

async function navigateTo(path: string) {
  try {
    const entries: Entry[] = await invoke('list_dir', { path });

    // Trim forward history if we branched
    if (historyIndex < history.length - 1) {
      history.splice(historyIndex + 1);
    }
    history.push(path);
    historyIndex = history.length - 1;
    currentPath = path;

    selectedIndices.clear();
    lastSelectedIndex = -1;

    renderEntries(entries);
    breadcrumbEl.textContent = path;
    statusPathEl.textContent = path;
    updateNavButtons();
  } catch (err) {
    statusInfoEl.textContent = `Error: ${err}`;
    console.error('Navigation error:', err);
  }
}

function updateNavButtons() {
  btnBack.disabled = historyIndex <= 0;
  btnForward.disabled = historyIndex >= history.length - 1;
}

btnBack.addEventListener('click', () => {
  if (historyIndex > 0) {
    historyIndex--;
    const path = history[historyIndex];
    currentPath = path;
    breadcrumbEl.textContent = path;
    statusPathEl.textContent = path;
    selectedIndices.clear();
    listPath(path);
    updateNavButtons();
  }
});

btnForward.addEventListener('click', () => {
  if (historyIndex < history.length - 1) {
    historyIndex++;
    const path = history[historyIndex];
    currentPath = path;
    breadcrumbEl.textContent = path;
    statusPathEl.textContent = path;
    selectedIndices.clear();
    listPath(path);
    updateNavButtons();
  }
});

btnUp.addEventListener('click', () => {
  if (currentPath) {
    const parent = currentPath.slice(0, currentPath.lastIndexOf('\\'));
    if (parent) navigateTo(parent);
  }
});

async function listPath(path: string) {
  try {
    const entries: Entry[] = await invoke('list_dir', { path });
    selectedIndices.clear();
    renderEntries(entries);
    updateNavButtons();
  } catch (err) {
    statusInfoEl.textContent = `Error: ${err}`;
  }
}

// ─── Keyboard Navigation ───

function handleCtrlShortcuts(e: KeyboardEvent) {
  switch (e.key.toLowerCase()) {
    case 'a':
      e.preventDefault();
      selectAll();
      return true;
    case 'c':
      e.preventDefault();
      handleCopy();
      return true;
    case 'x':
      e.preventDefault();
      handleCut();
      return true;
    case 'v':
      e.preventDefault();
      handlePaste();
      return true;
  }
  return false;
}

function handleArrowNav(e: KeyboardEvent) {
  const rows = fileListEl.querySelectorAll('.file-item');
  if (rows.length === 0) return false;

  if (e.key === 'ArrowDown') {
    e.preventDefault();
    const next = Math.min(
      selectedIndices.size > 0 ? Math.max(...selectedIndices) + 1 : 0,
      rows.length - 1,
    );
    singleSelect(next);
    return true;
  }

  if (e.key === 'ArrowUp') {
    e.preventDefault();
    const prev = Math.max(
      selectedIndices.size > 0 ? Math.min(...selectedIndices) - 1 : 0,
      0,
    );
    singleSelect(prev);
    return true;
  }

  return false;
}

function handleEnterKey() {
  if (selectedIndices.size === 0) return;
  const idx = Math.min(...selectedIndices);
  const entry = currentEntries[idx];
  if (entry && (entry.entryType === 'Folder' || entry.entryType === 'Drive')) {
    navigateTo(entry.path);
  }
}

function handleBackspaceKey() {
  if (!currentPath) return;
  const parent = currentPath.slice(0, currentPath.lastIndexOf('\\'));
  if (parent) navigateTo(parent);
}

function handleActionKeys(e: KeyboardEvent) {
  switch (e.key) {
    case 'Enter':
      e.preventDefault();
      handleEnterKey();
      return true;
    case 'F2':
      e.preventDefault();
      if (selectedIndices.size > 0) startRename(Math.min(...selectedIndices));
      return true;
    case 'Delete':
      e.preventDefault();
      handleDelete();
      return true;
    case 'Backspace':
      e.preventDefault();
      handleBackspaceKey();
      return true;
  }
  return false;
}

document.addEventListener('keydown', (e) => {
  if (document.activeElement?.tagName === 'INPUT') return;

  if (e.ctrlKey || e.metaKey) {
    handleCtrlShortcuts(e);
    return;
  }

  handleArrowNav(e) || handleActionKeys(e);
});

// ─── Global Context Menu (on empty area) ───

fileListEl.addEventListener('contextmenu', (e) => {
  if (e.target === fileListEl || (e.target as HTMLElement).classList.contains('empty-state')) {
    e.preventDefault();

    const items: ContextMenuItem[] = [
      { label: 'Paste', shortcut: 'Ctrl+V', disabled: !clipboard, action: handlePaste },
      { separator: true },
      { label: 'New Folder', action: handleNewFolder },
      { separator: true },
      { label: 'Refresh', action: () => { if (currentPath) navigateTo(currentPath); } },
    ];

    showContextMenu(e.clientX, e.clientY, items);
  }
});

// Close context menu on click elsewhere
document.addEventListener('click', hideContextMenu);
document.addEventListener('contextmenu', (e) => {
  // Only prevent default on file list, allow browser default elsewhere
});

// ─── Initialize ───

createContextMenu();

async function init() {
  try {
    const volumes: Volume[] = await invoke('get_volumes');
    renderDrives(volumes);

    if (volumes.length > 0) {
      await navigateTo(volumes[0].path);
    }
  } catch (err) {
    statusInfoEl.textContent = `Startup error: ${err}`;
    console.error('Init error:', err);
  }
}

function renderDrives(volumes: Volume[]) {
  drivesEl.innerHTML = '';
  for (const vol of volumes) {
    const item = document.createElement('div');
    item.className = 'sidebar-item';
    item.innerHTML = `<span class="icon">💾</span> ${vol.name}`;
    item.addEventListener('click', () => navigateTo(vol.path));
    drivesEl.appendChild(item);
  }
}

// ─── Analytics ───

const btnAnalytics = document.getElementById('btn-analytics')! as HTMLButtonElement;
const analyticsPanel = document.getElementById('analytics-panel')! as HTMLElement;
const fileListContainer = document.getElementById('file-list-container')! as HTMLElement;
const scanPathInput = document.getElementById('scan-path')! as HTMLInputElement;
const btnScan = document.getElementById('btn-scan')! as HTMLButtonElement;
const btnCancelScan = document.getElementById('btn-cancel-scan')! as HTMLButtonElement;
const btnSaveSnapshot = document.getElementById('btn-save-snapshot')! as HTMLButtonElement;
const analyticsProgress = document.getElementById('analytics-progress')! as HTMLElement;
const progressFill = document.getElementById('progress-fill')! as HTMLElement;
const progressText = document.getElementById('progress-text')! as HTMLElement;
const analyticsSummary = document.getElementById('analytics-summary')! as HTMLElement;
const summaryText = document.getElementById('summary-text')! as HTMLElement;

// Toggle analytics panel
btnAnalytics.addEventListener('click', () => {
    analyticsVisible = !analyticsVisible;
    if (analyticsVisible) {
        analyticsPanel.classList.remove('hidden');
        fileListContainer.classList.add('hidden');
        btnAnalytics.classList.add('active');
        // Pre-populate scan path with current path
        if (currentPath) {
            scanPathInput.value = currentPath;
        }
    } else {
        analyticsPanel.classList.add('hidden');
        fileListContainer.classList.remove('hidden');
        btnAnalytics.classList.remove('active');
    }
});

// Tab switching
document.querySelectorAll('.analytics-tab').forEach((tab) => {
    tab.addEventListener('click', () => {
        document.querySelectorAll('.analytics-tab').forEach((t) => t.classList.remove('active'));
        document.querySelectorAll('.analytics-tab-content').forEach((c) => c.classList.remove('active'));
        tab.classList.add('active');
        const tabName = tab.getAttribute('data-tab')!;
        currentTab = tabName;
        document.getElementById(`tab-${tabName}`)!.classList.add('active');
    });
});

// Scan button
btnScan.addEventListener('click', startScan);
btnCancelScan.addEventListener('click', cancelScan);
btnSaveSnapshot.addEventListener('click', saveSnapshot);

async function startScan() {
    const path = scanPathInput.value.trim() || currentPath;
    if (!path) {
        statusInfoEl.textContent = 'Please enter a path to scan';
        return;
    }

    // Reset results + observability + batching
    scanResults = { largeFiles: [], duplicates: [] };
    activeScanId = null;
    expandedPaths.clear();
    treeStore.clear();
    pathMap.clear();
    maxFolderSize = 0;
    foldersSized = 0;
    pendingChildren = [];
    pendingStats = [];
    pendingProgress = null;
    flushScheduled = false;

    // Show progress
    analyticsProgress.classList.remove('hidden');
    btnCancelScan.classList.remove('hidden');
    btnScan.disabled = true;
    progressFill.style.width = '0%';
    progressText.textContent = 'Starting scan...';
    analyticsSummary.classList.add('hidden');

    try {
        // Clear results BEFORE the invoke — the command awaits the full scan,
        // so chunk events arrive DURING the invoke and render the table.
        // If we clear AFTER, we'd overwrite the rendered results.
        if (currentTab === 'usage') {
            clearTabResults('usage-results', 'Scanning disk usage...');
        } else if (currentTab === 'large-files') {
            clearTabResults('large-files-results', 'Finding large files...');
        } else if (currentTab === 'duplicates') {
            clearTabResults('duplicates-results', 'Finding duplicates...');
        } else {
            statusInfoEl.textContent = 'No scan configured for this tab';
            return;
        }

        let scanId: string;
        if (currentTab === 'usage') {
            scanId = await invoke('start_scan_usage', { path, maxDepth: 0 });
        } else if (currentTab === 'large-files') {
            scanId = await invoke('start_find_large_files', {
                path,
                minSize: 100 * 1024 * 1024, // 100MB default
                maxResults: 100,
            });
        } else if (currentTab === 'duplicates') {
            scanId = await invoke('start_find_duplicates', { path });
        } else {
            statusInfoEl.textContent = 'No scan configured for this tab';
            return;
        }

        activeScanId = scanId;
        progressText.textContent = 'Scanning...';
    } catch (err) {
        statusInfoEl.textContent = `Scan error: ${err}`;
        resetScanUI();
    }
}

function cancelScan() {
    if (activeScanId) {
        invoke('cancel_scan', { scanId: activeScanId });
        activeScanId = null;
        progressText.textContent = 'Cancelled';
    }
}

function resetScanUI() {
    analyticsProgress.classList.add('hidden');
    btnCancelScan.classList.add('hidden');
    btnScan.disabled = false;
    activeScanId = null;

    // Clear scanning highlights
    document.querySelectorAll('.usage-tree-row.scanning').forEach(row => {
        row.classList.remove('scanning');
    });
}

function clearTabResults(containerId: string, message: string) {
    const container = document.getElementById(containerId)!;
    container.innerHTML = `<div class="empty-state">${message}</div>`;
}

async function saveSnapshot() {
    // Collect usage data from the tree, resolving paths from pathMap
    const usageData: Array<{ path: string; size: number; fileCount: number; folderCount: number }> = [];
    for (const [nodeId, node] of treeStore) {
        if (node.stats) {
            const pathInfo = pathMap.get(nodeId);
            const path = pathInfo?.path ?? `node_${nodeId}`;
            usageData.push({ path, ...node.stats });
        }
    }

    if (usageData.length === 0) {
        statusInfoEl.textContent = 'No usage data to snapshot. Run a disk usage scan first.';
        return;
    }

    // Sort by size descending for top folders
    usageData.sort((a, b) => b.size - a.size);

    try {
        const topFolders = JSON.stringify(usageData.slice(0, 10));
        // Root folder is the largest — use it for totals
        const root = usageData[0];

        await invoke('snapshot_usage', {
            path: scanPathInput.value,
            totalSize: root.size,
            fileCount: root.fileCount,
            folderCount: root.folderCount,
            topFolders: topFolders,
        });

        statusInfoEl.textContent = 'Snapshot saved!';
        loadHistory();
    } catch (err) {
        statusInfoEl.textContent = `Snapshot error: ${err}`;
    }
}

async function loadHistory() {
    try {
        const start = new Date(Date.now() - 30 * 24 * 60 * 60 * 1000).toISOString();
        const end = new Date().toISOString();
        const path = scanPathInput.value || currentPath || '';

        const history: Array<{
            id: number;
            path: string;
            totalSize: number;
            fileCount: number;
            folderCount: number;
            scannedAt: string;
        }> = await invoke('usage_history', { path, start, end });

        const container = document.getElementById('history-results')!;
        if (history.length === 0) {
            container.innerHTML = '<div class="empty-state">No snapshots found</div>';
            return;
        }

        let html = '<table class="analytics-table">';
        html += '<tr><th>Date</th><th>Path</th><th>Size</th><th>Files</th><th>Folders</th></tr>';
        for (const snap of history) {
            html += `<tr>
                <td>${formatDate(snap.scanned_at)}</td>
                <td>${snap.path}</td>
                <td>${formatSize(snap.totalSize)}</td>
                <td>${snap.fileCount.toLocaleString()}</td>
                <td>${snap.folderCount.toLocaleString()}</td>
            </tr>`;
        }
        html += '</table>';
        container.innerHTML = html;
    } catch (err) {
        statusInfoEl.textContent = `History error: ${err}`;
    }
}

// ─── Event Listeners ───

// Listen for scan events — silently skip if not in Tauri webview
function setupScanListeners() {
    // Tree started — render root row (not batched, fires once)
    listen('scan:tree_started', (event) => {
        const info = event.payload as { scanId: string; rootId: number; rootPath: string; rootName: string };
        treeStore.clear();
        pathMap.clear();
        expandedPaths.clear();
        pendingChildren = [];
        pendingStats = [];
        maxFolderSize = 0;
        foldersSized = 0;

        // Store root path info for resolution
        pathMap.set(info.rootId, { path: info.rootPath, name: info.rootName });
        treeStore.set(info.rootId, { childCount: 0 });

        // Set activeScanId so children_ready can pull children
        activeScanId = info.scanId;

        renderTreeRoot(info.rootId, info.rootPath, info.rootName);
    }).catch(err => console.error('Failed to register scan:tree_started listener:', err));

    // Children ready — thin event: just parentId + childCount
    listen('scan:children_ready', (event) => {
        const data = event.payload as { scanId: string; parentId: number; childCount: number };
        pendingChildren.push({ parentId: data.parentId, childCount: data.childCount });
        scheduleFlush();

        // If the parent is expanded and children aren't loaded yet, fetch them
        // Auto-expand so more rows exist for incoming stats to land on.
        const node = treeStore.get(data.parentId);
        const isExpanded = expandedPaths.has(data.parentId);
        const needsFetch = !node || !node.children;
        if (data.parentId === 0) {
            console.log('[DEBUG] children_ready root: expanded=', isExpanded, 'node=', node, 'needsFetch=', needsFetch, 'childCount=', data.childCount);
        }
        if (isExpanded && needsFetch) {
            const row = document.querySelector(`.usage-tree-row[data-node-id="${data.parentId}"]`);
            const depth = getDepthFromRow(row);
            if (data.parentId === 0) console.log('[DEBUG] fetchAndRenderChildren root, row=', !!row, 'depth=', depth);
            fetchAndRenderChildren(data.parentId, depth, true);
        }
    }).catch(err => console.error('Failed to register scan:children_ready listener:', err));

    // Progress — buffer for batched DOM update
    listen('scan:progress', (event) => {
        const data = event.payload as { percentage: number; message: string };
        pendingProgress = data;
        scheduleFlush();
    }).catch(err => console.error('Failed to register scan:progress listener:', err));

    // Phase 2: Chunk — buffer stats for batched DOM update
    let _chunkDebugCount = 0;
    listen('scan:chunk', (event) => {
        const chunk = event.payload as any;
        const data = chunk.data;

        if (++_chunkDebugCount === 1) {
            console.log('[DEBUG] first chunk payload:', JSON.stringify(chunk).substring(0, 300));
        }

        if (data.type === 'folder_usage') {
            // usage.nodeId is a number (NodeId serializes as u32)
            pendingStats.push({
                nodeId: data.usage.nodeId,
                size: data.usage.size,
                fileCount: data.usage.fileCount,
                folderCount: data.usage.folderCount,
            });
            scheduleFlush();
        } else if (data.type === 'large_file') {
            scanResults.largeFiles.push(data.entry);
            renderLargeFilesResults();
        } else if (data.type === 'duplicate_group') {
            scanResults.duplicates.push(data.group);
            renderDuplicatesResults();
        }
    }).catch(err => console.error('Failed to register scan:chunk listener:', err));

    listen('scan:complete', (event) => {
        const data = event.payload as { totalItems: number; totalSize: number; durationMs: number };

        // Apply root stats from summary if they haven't arrived via chunk events.
        // The root row may still show placeholders if the last flush hasn't run yet.
        if (activeScanId) {
            const rootInStore = treeStore.get(0);
            if (!rootInStore?.stats) {
                // Root stats missing — apply from summary data
                treeStore.set(0, {
                    childCount: rootInStore?.childCount ?? 0,
                    children: rootInStore?.children,
                    stats: { size: data.totalSize, fileCount: data.totalItems, folderCount: 0 },
                });
                // Update the root row in the DOM directly
                const rootRow = document.querySelector('.usage-tree-row[data-node-id="0"]') as HTMLElement;
                if (rootRow) {
                    const sizeEl = rootRow.querySelector('.tree-size') as HTMLElement;
                    const filesEl = rootRow.querySelector('.tree-files') as HTMLElement;
                    const foldersEl = rootRow.querySelector('.tree-folders') as HTMLElement;
                    if (sizeEl) sizeEl.textContent = formatSize(data.totalSize);
                    if (filesEl) filesEl.textContent = data.totalItems.toLocaleString();
                    if (foldersEl) foldersEl.textContent = '0';
                }
            }
        }

        resetScanUI();
        analyticsSummary.classList.remove('hidden');
        summaryText.textContent = `Scan complete: ${data.totalItems.toLocaleString()} items · ${formatSize(data.totalSize)} · ${(data.durationMs / 1000).toFixed(1)}s`;
        statusInfoEl.textContent = summaryText.textContent;

        // Observability — log memory state after scan
        logMemoryState();
    }).catch(err => console.error('Failed to register scan:complete listener:', err));

    listen('scan:error', (event) => {
        const data = event.payload as { message: string };
        resetScanUI();
        statusInfoEl.textContent = data.message;
    }).catch(err => console.error('Failed to register scan:error listener:', err));
}

setupScanListeners();

// ─── Observability ───

/// Log memory state to console for debugging.
/// Uses performance.memory when available (Chrome/Edge), falls back to tree stats.
function logMemoryState() {
    const info = {
        treeNodes: treeStore.size,
        pathMapEntries: pathMap.size,
        foldersSized,
        maxFolderSize: formatSize(maxFolderSize),
        expandedPaths: expandedPaths.size,
        largeFiles: scanResults.largeFiles.length,
        duplicates: scanResults.duplicates.length,
    };

    // performance.memory is available in Chrome/Edge (not Firefox)
    if ('memory' in performance) {
        const mem = (performance as any).memory as {
            usedJSHeapSize: number;
            totalJSHeapSize: number;
            jsHeapSizeLimit: number;
        } | undefined;
        if (mem) {
            console.log('[MEMORY]', {
                ...info,
                usedHeap: formatSize(mem.usedJSHeapSize),
                totalHeap: formatSize(mem.totalJSHeapSize),
                heapLimit: formatSize(mem.jsHeapSizeLimit),
                heapPct: ((mem.usedJSHeapSize / mem.jsHeapSizeLimit) * 100).toFixed(1) + '%',
            });
        } else {
            console.log('[MEMORY]', info);
        }
    } else {
        console.log('[MEMORY]', info);
    }
}

// ─── Render Functions ───

// ─── Usage Tree (Pull-Based, NodeId-Driven) ───

/// Render the root folder row when scan starts.
function renderTreeRoot(rootId: number, rootPath: string, rootName: string) {
    const container = document.getElementById('usage-results')!;

    // Remove empty-state placeholder
    const emptyState = container.querySelector('.empty-state');
    if (emptyState) emptyState.remove();

    // Ensure header exists
    if (!container.querySelector('.usage-tree-header')) {
        const header = document.createElement('div');
        header.className = 'usage-tree-header';
        header.innerHTML = `
            <span class="tree-col-name">Name</span>
            <span class="tree-col-size">Size</span>
            <span class="tree-col-bar"></span>
            <span class="tree-col-files">Files</span>
            <span class="tree-col-folders">Folders</span>
        `;
        container.appendChild(header);
    }

    // Ensure body exists
    let body = container.querySelector('.usage-tree-body') as HTMLElement;
    if (!body) {
        body = document.createElement('div');
        body.className = 'usage-tree-body';
        container.appendChild(body);
    }

    // Clear body and render root
    body.innerHTML = '';
    expandedPaths.add(rootId); // Auto-expand root so children render when pulled
    body.appendChild(renderTreeRow(rootId, 0));
}

/// Render a single tree row by NodeId. Toggle is disabled until children are discovered.
/// parentSize: if provided, the size bar shows percentage of parent (root = 100%).
function renderTreeRow(nodeId: number, depth: number, parentSize?: number): HTMLElement {
    const node = treeStore.get(nodeId);
    const hasChildren = node?.childCount !== undefined && node.childCount > 0;
    const isExpanded = expandedPaths.has(nodeId);
    const stats = node?.stats;
    const pathInfo = pathMap.get(nodeId);
    const displayName = pathInfo?.name ?? `node_${nodeId}`;

    // Row
    const row = document.createElement('div');
    row.className = 'usage-tree-row';
    row.setAttribute('data-node-id', String(nodeId));

    // Indent
    const indent = document.createElement('span');
    indent.className = 'tree-indent';
    indent.style.width = `${depth * 16}px`;

    // Toggle
    const toggle = document.createElement('span');
    toggle.className = 'tree-toggle' + (isExpanded ? ' expanded' : '') + (!hasChildren ? ' disabled' : '');
    toggle.textContent = hasChildren ? '▶' : '';

    // Icon — disk for drive roots, folder for everything else
    const icon = document.createElement('span');
    icon.className = 'tree-icon';
    const isDrive = pathInfo && /^[A-Za-z]:$/.test(pathInfo.path.replace(/\/$/, ''));
    icon.textContent = isDrive ? '💾' : '📁';

    // Name
    const name = document.createElement('span');
    name.className = 'tree-name';
    name.textContent = displayName;

    // Stats — apply stored stats if available, otherwise placeholders
    const size = document.createElement('span');
    size.className = 'tree-size';
    size.textContent = stats ? formatSize(stats.size) : '—';

    const bar = document.createElement('span');
    bar.className = 'tree-size-bar';
    bar.style.width = '0%';
    if (stats && stats.size > 0) {
        bar.dataset.size = String(stats.size);
    }

    const files = document.createElement('span');
    files.className = 'tree-files';
    files.textContent = stats ? stats.fileCount.toLocaleString() : '—';

    const folders = document.createElement('span');
    folders.className = 'tree-folders';
    folders.textContent = stats ? stats.folderCount.toLocaleString() : '—';

    // Name cell
    const nameCell = document.createElement('span');
    nameCell.className = 'tree-name-cell';
    nameCell.appendChild(indent);
    nameCell.appendChild(toggle);
    nameCell.appendChild(icon);
    nameCell.appendChild(name);

    row.appendChild(nameCell);
    row.appendChild(size);
    row.appendChild(bar);
    row.appendChild(files);
    row.appendChild(folders);

    // Click handler — expand/collapse or fetch children
    row.addEventListener('click', () => handleTreeExpand(nodeId, row, depth));

    // Children container — store parent size for bar calculations
    const childrenContainer = document.createElement('div');
    childrenContainer.className = 'usage-tree-children' + (isExpanded ? ' expanded' : '');
    childrenContainer.dataset.runningSize = '0';
    if (stats?.size) {
        childrenContainer.dataset.parentSize = String(stats.size);
    }

    // If already expanded and children loaded, render them
    if (isExpanded && node?.children) {
        for (const child of node.children) {
            childrenContainer.appendChild(renderTreeRow(child.id, depth + 1, stats?.size));
        }
        // Calculate bar widths now that all children are in the container
        recalcSiblings(childrenContainer);
    }

    const wrapper = document.createElement('div');
    wrapper.appendChild(row);
    wrapper.appendChild(childrenContainer);

    return wrapper;
}

/// Handle expand/collapse click on a tree row.
/// On expand, pulls children from backend via get_scan_tree_children.
async function handleTreeExpand(nodeId: number, row: HTMLElement, depth: number) {
    const node = treeStore.get(nodeId);
    if (!node) return; // Children not yet discovered

    const isExpanded = expandedPaths.has(nodeId);
    const childrenContainer = row.nextElementSibling as HTMLElement | null;

    if (isExpanded) {
        // Collapse
        expandedPaths.delete(nodeId);
        if (childrenContainer) childrenContainer.classList.remove('expanded');
        const toggle = row.querySelector('.tree-toggle') as HTMLElement;
        if (toggle) toggle.classList.remove('expanded');
    } else {
        // Expand
        expandedPaths.add(nodeId);
        if (childrenContainer) childrenContainer.classList.add('expanded');
        const toggle = row.querySelector('.tree-toggle') as HTMLElement;
        if (toggle) toggle.classList.add('expanded');

        // If children already loaded, render them
        if (childrenContainer && node.children && childrenContainer.children.length === 0) {
            const parentSize = node.stats?.size;
            for (const child of node.children) {
                childrenContainer.appendChild(renderTreeRow(child.id, depth + 1, parentSize));
            }
            recalcSiblings(childrenContainer);
        }

        // If children not loaded yet, pull from backend
        if (childrenContainer && !node.children && node.childCount > 0 && activeScanId) {
            await fetchAndRenderChildren(nodeId, depth);
        }
    }
}

/// Get the depth of a tree row from its indent element.
function getDepthFromRow(row: HTMLElement | null): number {
    if (!row) return 0;
    const nameCell = row.querySelector('.tree-name-cell') as HTMLElement;
    const indent = nameCell?.querySelector('.tree-indent') as HTMLElement;
    return indent ? parseInt(indent.style.width.replace('px', '')) / 16 : 0;
}

/// Fetch children from backend and render them into the DOM.
/// If autoExpand is true (called during scan), also expand each child one level deeper
/// so more rows exist for incoming stats to land on.
async function fetchAndRenderChildren(parentId: number, depth: number, autoExpand = false) {
    if (!activeScanId) {
        if (parentId === 0) console.log('[DEBUG] fetchAndRenderChildren: no activeScanId');
        return;
    }

    try {
        if (parentId === 0) console.log('[DEBUG] fetchAndRenderChildren root: invoking, scanId=', activeScanId);
        const children: Array<{ id: number; name: string; path?: string }> =
            await invoke('get_scan_tree_children', {
                scanId: activeScanId,
                parentId,
            });

        if (parentId === 0) console.log('[DEBUG] fetchAndRenderChildren root: got', children.length, 'children');
        if (children.length === 0) return;

        // Store path info for each child
        for (const child of children) {
            pathMap.set(child.id, {
                path: child.path ?? `node_${child.id}`,
                name: child.name,
            });
        }

        // Store children in tree store
        const existing = treeStore.get(parentId);
        treeStore.set(parentId, {
            childCount: existing?.childCount ?? children.length,
            children,
            stats: existing?.stats,
        });

        // Find the children container in the DOM
        const row = document.querySelector(`.usage-tree-row[data-node-id="${parentId}"]`) as HTMLElement | null;
        if (!row) return;
        const childrenContainer = row.nextElementSibling as HTMLElement | null;
        if (!childrenContainer) return;

        // Store parent size for bar calculations
        childrenContainer.dataset.runningSize = '0';
        const parentSize = existing?.stats?.size;
        if (parentSize) {
            childrenContainer.dataset.parentSize = String(parentSize);
        }

        // Render each child with parent size for bar calculation
        for (const child of children) {
            childrenContainer.appendChild(renderTreeRow(child.id, depth + 1, parentSize));
        }

        // Calculate bar widths now that all children are in the container
        recalcSiblings(childrenContainer);

        // Auto-expand one more level during scan so stats have rows to land on
        if (autoExpand && depth < 2) {
            for (const child of children) {
                expandedPaths.add(child.id);
                fetchAndRenderChildren(child.id, depth + 1, true);
            }
        }
    } catch (err) {
        console.error('Failed to load children:', err);
    }
}

/// Enable the expand button for a folder when children are discovered.
function enableExpandButton(nodeId: number, childCount: number) {
    const row = document.querySelector(`.usage-tree-row[data-node-id="${nodeId}"]`) as HTMLElement;
    if (!row) return;

    const toggle = row.querySelector('.tree-toggle') as HTMLElement;
    if (toggle) {
        toggle.textContent = '▶';
        toggle.classList.remove('disabled');
    }
}


function renderLargeFilesResults() {
    const container = document.getElementById('large-files-results')!;
    if (scanResults.largeFiles.length === 0) return;

    // Sort by size descending
    scanResults.largeFiles.sort((a, b) => b.size - a.size);

    let html = '<table class="analytics-table">';
    html += '<tr><th>Name</th><th>Size</th><th>Modified</th><th>Path</th></tr>';
    for (const file of scanResults.largeFiles) {
        html += `<tr>
            <td>${file.name}</td>
            <td>${formatSize(file.size)}</td>
            <td>${formatDate(file.modified)}</td>
            <td>${file.path}</td>
        </tr>`;
    }
    html += '</table>';
    container.innerHTML = html;
}

function renderDuplicatesResults() {
    const container = document.getElementById('duplicates-results')!;
    if (scanResults.duplicates.length === 0) return;

    let totalWasted = scanResults.duplicates.reduce((sum, g) => sum + g.wastedSpace, 0);
    let html = `<div style="padding: 8px; font-weight: 500;">Found ${scanResults.duplicates.length} duplicate groups · ${formatSize(totalWasted)} wasted</div>`;
    html += '<table class="analytics-table">';
    html += '<tr><th>Files</th><th>Size Each</th><th>Count</th><th>Wasted</th></tr>';
    for (const group of scanResults.duplicates) {
        html += `<tr>
            <td>${group.files.map(f => f.split('\\').pop()).join(', ')}</td>
            <td>${formatSize(group.sizeEach)}</td>
            <td>${group.files.length}</td>
            <td>${formatSize(group.wastedSpace)}</td>
        </tr>`;
    }
    html += '</table>';
    container.innerHTML = html;
}

init();
