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
    usage: Array<{ path: string; size: number; fileCount: number; folderCount: number }>;
    largeFiles: Entry[];
    duplicates: Array<{ hash: string; sizeEach: number; files: string[]; wastedSpace: number }>;
} = { usage: [], largeFiles: [], duplicates: [] };

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

    // Reset results
    scanResults = { usage: [], largeFiles: [], duplicates: [] };
    activeScanId = null;

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
            scanId = await invoke('start_scan_usage', { path, maxDepth: 2 });
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
}

function clearTabResults(containerId: string, message: string) {
    const container = document.getElementById(containerId)!;
    container.innerHTML = `<div class="empty-state">${message}</div>`;
}

async function saveSnapshot() {
    if (scanResults.usage.length === 0) {
        statusInfoEl.textContent = 'No usage data to snapshot. Run a disk usage scan first.';
        return;
    }

    try {
        const topFolders = JSON.stringify(scanResults.usage.slice(0, 10));
        const totalSize = scanResults.usage.reduce((sum, u) => sum + u.size, 0);
        const totalFiles = scanResults.usage.reduce((sum, u) => sum + u.fileCount, 0);
        const totalFolders = scanResults.usage.reduce((sum, u) => sum + u.folderCount, 0);

        await invoke('snapshot_usage', {
            path: scanPathInput.value,
            totalSize,
            fileCount: totalFiles,
            folderCount: totalFolders,
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

// ─── Event Listeners ─── (refresh 2)

// Listen for scan events — silently skip if not in Tauri webview
function setupScanListeners() {
    console.log('Setting up scan event listeners...');
    listen('scan:progress', (event) => {
        console.log('scan:progress received', event.payload);
        const data = event.payload as { percentage: number; message: string };
        progressFill.style.width = `${data.percentage}%`;
        progressText.textContent = data.message;
        statusInfoEl.textContent = data.message;
    }).then(() => console.log('scan:progress listener registered'))
      .catch(err => console.error('Failed to register scan:progress listener:', err));

    listen('scan:chunk', (event) => {
        console.log('scan:chunk received', event.payload);
        const chunk = event.payload as any;
        const data = chunk.data;

        if (data.type === 'folder_usage') {
            scanResults.usage.push(data.usage);
            renderUsageResults();
        } else if (data.type === 'large_file') {
            scanResults.largeFiles.push(data.entry);
            renderLargeFilesResults();
        } else if (data.type === 'duplicate_group') {
            scanResults.duplicates.push(data.group);
            renderDuplicatesResults();
        }
    }).then(() => console.log('scan:chunk listener registered'))
      .catch(err => console.error('Failed to register scan:chunk listener:', err));

    listen('scan:complete', (event) => {
        console.log('scan:complete received', event.payload);
        const data = event.payload as { totalItems: number; totalSize: number; durationMs: number };
        resetScanUI();
        analyticsSummary.classList.remove('hidden');
        summaryText.textContent = `Scan complete: ${data.totalItems.toLocaleString()} items · ${formatSize(data.totalSize)} · ${(data.durationMs / 1000).toFixed(1)}s`;
        statusInfoEl.textContent = summaryText.textContent;
    }).then(() => console.log('scan:complete listener registered'))
      .catch(err => console.error('Failed to register scan:complete listener:', err));

    listen('scan:error', (event) => {
        console.log('scan:error received', event.payload);
        const data = event.payload as { message: string };
        resetScanUI();
        statusInfoEl.textContent = data.message;
    }).then(() => console.log('scan:error listener registered'))
      .catch(err => console.error('Failed to register scan:error listener:', err));
}

setupScanListeners();

// ─── Render Functions ───

function renderUsageResults() {
    const container = document.getElementById('usage-results')!;
    if (scanResults.usage.length === 0) return;

    // Sort by size descending
    scanResults.usage.sort((a, b) => b.size - a.size);
    const maxSize = scanResults.usage[0].size;

    let html = '<table class="analytics-table">';
    html += '<tr><th>Path</th><th>Size</th><th>Files</th><th>Folders</th></tr>';
    for (const usage of scanResults.usage) {
        const barWidth = (usage.size / maxSize) * 100;
        html += `<tr>
            <td>${usage.path}</td>
            <td>
                ${formatSize(usage.size)}
                <span class="size-bar" style="width: ${barWidth}px;"></span>
            </td>
            <td>${usage.fileCount.toLocaleString()}</td>
            <td>${usage.folderCount.toLocaleString()}</td>
        </tr>`;
    }
    html += '</table>';
    container.innerHTML = html;
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
