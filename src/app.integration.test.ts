import { describe, it, expect, beforeEach, vi } from 'vitest';
import {
  resetTauriMocks,
  flushPromises,
  emitEvent,
  registeredHandlers,
} from './test/helpers';
import { bootApp } from './test/boot';
import type { TauriMockInvoke } from './test/helpers';

// ─── Tests ───

describe('Sidebar — Drives', () => {
  beforeEach(() => {
    resetTauriMocks();
  });

  it('populates the sidebar with drives on startup', async () => {
    await bootApp((cmd) => {
      if (cmd === 'get_volumes') return [
        { name: 'C:', path: 'C:\\' },
        { name: 'D:', path: 'D:\\' },
      ];
      return [];
    });

    const items = document.getElementById('drives')!.querySelectorAll('.sidebar-item');
    expect(items.length).toBe(2);
    expect(items[0].textContent).toContain('C:');
    expect(items[1].textContent).toContain('D:');
  });

  it('invokes get_volumes during init', async () => {
    await bootApp();
    const { invoke } = await import('@tauri-apps/api/core');
    expect(invoke).toHaveBeenCalledWith('get_volumes');
  });

  it('navigates to the first drive on startup', async () => {
    await bootApp();
    const { invoke } = await import('@tauri-apps/api/core');
    expect(invoke).toHaveBeenCalledWith('list_dir', { path: 'C:\\' });
  });
});

describe('Analytics Panel — Toggle', () => {
  beforeEach(() => {
    resetTauriMocks();
  });

  it('clicking 📊 shows the Analytics panel and hides the file list', async () => {
    await bootApp();

    const btn = document.getElementById('btn-analytics')!;
    const panel = document.getElementById('analytics-panel')!;
    const fileList = document.getElementById('file-list-container')!;

    expect(panel.classList.contains('hidden')).toBe(true);
    btn.click();
    await flushPromises();

    expect(panel.classList.contains('hidden')).toBe(false);
    expect(fileList.classList.contains('hidden')).toBe(true);
    expect(btn.classList.contains('active')).toBe(true);
  });

  it('clicking 📊 again hides the Analytics panel', async () => {
    await bootApp();

    const btn = document.getElementById('btn-analytics')!;
    const panel = document.getElementById('analytics-panel')!;
    const fileList = document.getElementById('file-list-container')!;

    btn.click(); await flushPromises();
    expect(panel.classList.contains('hidden')).toBe(false);

    btn.click(); await flushPromises();
    expect(panel.classList.contains('hidden')).toBe(true);
    expect(fileList.classList.contains('hidden')).toBe(false);
  });

  it('pre-populates the scan path input with the current path', async () => {
    await bootApp();

    document.getElementById('btn-analytics')!.click();
    await flushPromises();

    const scanPath = document.getElementById('scan-path')! as HTMLInputElement;
    expect(scanPath.value).toBe('C:\\');
  });
});

describe('Analytics — Disk Usage Scan', () => {
  beforeEach(() => {
    resetTauriMocks();
  });

  async function bootWithScan() {
    await bootApp((cmd) => {
      if (cmd === 'get_volumes') return [{ name: 'C:', path: 'C:\\' }];
      if (cmd === 'start_scan_usage') return 'scan-123';
      if (cmd === 'start_find_large_files') return 'scan-large';
      if (cmd === 'start_find_duplicates') return 'scan-dup';
      return [];
    });
  }

  it('clicking Scan starts a scan with snake_case args and shows progress', async () => {
    await bootWithScan();

    document.getElementById('btn-analytics')!.click();
    await flushPromises();
    document.getElementById('btn-scan')!.click();
    await flushPromises();

    const { invoke } = await import('@tauri-apps/api/core');
    expect(invoke).toHaveBeenCalledWith(
      'start_scan_usage',
      expect.objectContaining({ path: 'C:\\', maxDepth: 0 }),
    );

    expect(document.getElementById('analytics-progress')!.classList.contains('hidden')).toBe(false);
    expect(document.getElementById('btn-cancel-scan')!.classList.contains('hidden')).toBe(false);
  });

  it('scan:chunk events render usage results', async () => {
    await bootWithScan();

    document.getElementById('btn-analytics')!.click();
    await flushPromises();
    document.getElementById('btn-scan')!.click();
    await flushPromises();

    // Tree started — renders root row
    emitEvent('scan:tree_started', {
      scanId: 'test_scan',
      rootPath: 'C:/',
      rootName: 'C:/',
    });
    await flushPromises();

    // Children ready — enables expand and stores children
    emitEvent('scan:children_ready', {
      scanId: 'test_scan',
      parentPath: 'C:/',
      children: [
        { path: 'C:/Windows', name: 'Windows' },
        { path: 'C:/Users', name: 'Users' },
      ],
    });
    await flushPromises();

    const results = document.getElementById('usage-results')!;
    expect(results.innerHTML).toContain('usage-tree-header');

    // Expand root to render children
    const rootRow = results.querySelector('.usage-tree-row[data-path="C:/"]')!;
    rootRow.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    await flushPromises();

    // Children should now be rendered
    expect(results.innerHTML).toContain('Windows');
    expect(results.innerHTML).toContain('Users');

    // Phase 2: emit chunk to patch the row
    emitEvent('scan:chunk', {
      scanId: 'test_scan',
      data: {
        type: 'folder_usage',
        usage: { path: 'C:/Windows', size: 5368709120, fileCount: 12345, folderCount: 890 },
      },
    });
    await flushPromises();

    expect(results.innerHTML).toContain('5.0 GB');
  });

  it('scan:complete hides progress and shows summary', async () => {
    await bootWithScan();

    document.getElementById('btn-analytics')!.click();
    await flushPromises();
    document.getElementById('btn-scan')!.click();
    await flushPromises();
    expect(document.getElementById('analytics-progress')!.classList.contains('hidden')).toBe(false);

    emitEvent('scan:complete', { totalItems: 5000, totalSize: 10737418240, durationMs: 3500 });
    await flushPromises();

    expect(document.getElementById('analytics-progress')!.classList.contains('hidden')).toBe(true);
    expect(document.getElementById('analytics-summary')!.classList.contains('hidden')).toBe(false);

    const text = document.getElementById('summary-text')!.textContent!;
    expect(text).toContain('5,000');
    expect(text).toContain('10.0 GB');
    expect(text).toContain('3.5s');
  });

  it('disables Scan button while running', async () => {
    await bootWithScan();

    document.getElementById('btn-analytics')!.click();
    await flushPromises();

    const btn = document.getElementById('btn-scan')! as HTMLButtonElement;
    expect(btn.disabled).toBe(false);
    btn.click(); await flushPromises();
    expect(btn.disabled).toBe(true);
  });

  it('scan:progress updates the progress bar', async () => {
    await bootWithScan();

    document.getElementById('btn-analytics')!.click();
    await flushPromises();
    document.getElementById('btn-scan')!.click(); await flushPromises();

    emitEvent('scan:progress', { percentage: 42, message: 'Scanning C:\\Windows...' });
    await flushPromises();

    expect(document.getElementById('progress-fill')!.style.width).toBe('42%');
    expect(document.getElementById('progress-text')!.textContent).toBe('Scanning C:\\Windows...');
  });

  it('scan:error resets the UI', async () => {
    await bootWithScan();

    document.getElementById('btn-analytics')!.click();
    await flushPromises();
    document.getElementById('btn-scan')!.click(); await flushPromises();

    emitEvent('scan:error', { message: 'Access denied' });
    await flushPromises();

    expect(document.getElementById('analytics-progress')!.classList.contains('hidden')).toBe(true);
    expect((document.getElementById('btn-scan')! as HTMLButtonElement).disabled).toBe(false);
  });
});

async function bootAndScan(tabName?: string) {
  await bootApp((cmd) => {
    if (cmd === 'get_volumes') return [{ name: 'C:', path: 'C:\\' }];
    if (cmd.startsWith('start_')) return 'scan-ok';
    return [];
  });

  document.getElementById('btn-analytics')!.click();
  await flushPromises();

  if (tabName) {
    document.querySelector(`[data-tab="${tabName}"]`)!.dispatchEvent(
      new MouseEvent('click', { bubbles: true }),
    );
    await flushPromises();
  }

  document.getElementById('btn-scan')!.click();
  await flushPromises();
}

async function getInvokeCall(command: string): Promise<Record<string, unknown>> {
  const { invoke } = await import('@tauri-apps/api/core');
  const call = (invoke as unknown as TauriMockInvoke).mock.calls.find(
    (c: string[]) => c[0] === command,
  );
  return call![1] as Record<string, unknown>;
}

describe('Tauri IPC — Argument Casing', () => {
  beforeEach(() => {
    resetTauriMocks();
  });

  it('start_scan_usage → maxDepth (Tauri 2 camelCase)', async () => {
    await bootAndScan();
    const args = await getInvokeCall('start_scan_usage');
    expect(args).toHaveProperty('maxDepth');
    expect(args).not.toHaveProperty('max_depth');
  });

  it('start_find_large_files → minSize, maxResults (Tauri 2 camelCase)', async () => {
    await bootAndScan('large-files');
    const args = await getInvokeCall('start_find_large_files');
    expect(args).toHaveProperty('minSize');
    expect(args).toHaveProperty('maxResults');
    expect(args).not.toHaveProperty('min_size');
    expect(args).not.toHaveProperty('max_results');
  });

  it('start_find_duplicates → path (single word, no casing issue)', async () => {
    await bootAndScan('duplicates');
    const args = await getInvokeCall('start_find_duplicates');
    expect(args).toHaveProperty('path');
  });
});

// ─── Large Files Scan ───

describe('Analytics — Large Files Scan', () => {
  beforeEach(() => {
    resetTauriMocks();
  });

  it('switches to large-files tab and invokes start_find_large_files', async () => {
    await bootApp((cmd) => {
      if (cmd === 'get_volumes') return [{ name: 'C:', path: 'C:\\' }];
      if (cmd === 'start_find_large_files') return 'scan-large';
      return [];
    });

    document.getElementById('btn-analytics')!.click();
    await flushPromises();

    // Switch to large-files tab
    document.querySelector('[data-tab="large-files"]')!.dispatchEvent(
      new MouseEvent('click', { bubbles: true }),
    );
    await flushPromises();

    document.getElementById('btn-scan')!.click();
    await flushPromises();

    const { invoke } = await import('@tauri-apps/api/core');
    expect(invoke).toHaveBeenCalledWith(
      'start_find_large_files',
      expect.objectContaining({ path: 'C:\\', minSize: expect.any(Number), maxResults: 100 }),
    );
  });

  it('scan:chunk with large_file type renders results', async () => {
    await bootApp((cmd) => {
      if (cmd === 'get_volumes') return [{ name: 'C:', path: 'C:\\' }];
      if (cmd === 'start_find_large_files') return 'scan-large';
      return [];
    });

    document.getElementById('btn-analytics')!.click();
    await flushPromises();

    // Switch to large-files tab
    document.querySelector('[data-tab="large-files"]')!.dispatchEvent(
      new MouseEvent('click', { bubbles: true }),
    );
    await flushPromises();

    document.getElementById('btn-scan')!.click();
    await flushPromises();

    emitEvent('scan:chunk', {
      scanId: 'scan-large',
      data: {
        type: 'large_file',
        entry: {
          name: 'video.mp4',
          path: 'C:\\Videos\\video.mp4',
          size: 1073741824,
          modified: '2024-06-15T12:00:00Z',
          entryType: 'File',
          extension: '.mp4',
        },
      },
    });
    await flushPromises();

    const results = document.getElementById('large-files-results')!;
    expect(results.innerHTML).toContain('video.mp4');
    expect(results.innerHTML).toContain('1.0 GB');
  });

  it('scan:chunk with duplicate_group type renders results', async () => {
    await bootApp((cmd) => {
      if (cmd === 'get_volumes') return [{ name: 'C:', path: 'C:\\' }];
      if (cmd === 'start_find_duplicates') return 'scan-dup';
      return [];
    });

    document.getElementById('btn-analytics')!.click();
    await flushPromises();

    // Switch to duplicates tab
    document.querySelector('[data-tab="duplicates"]')!.dispatchEvent(
      new MouseEvent('click', { bubbles: true }),
    );
    await flushPromises();

    document.getElementById('btn-scan')!.click();
    await flushPromises();

    emitEvent('scan:chunk', {
      scanId: 'scan-dup',
      data: {
        type: 'duplicate_group',
        group: {
          hash: 'abc123',
          sizeEach: 1024,
          files: ['C:\\a\\file.txt', 'C:\\b\\file.txt'],
          wastedSpace: 1024,
        },
      },
    });
    await flushPromises();

    const results = document.getElementById('duplicates-results')!;
    expect(results.innerHTML).toContain('file.txt');
    expect(results.innerHTML).toContain('1.0 KB');
  });
});

// ─── Paste / New Folder ───

describe('File Operations — Paste', () => {
  beforeEach(() => {
    resetTauriMocks();
  });

  it('paste after copy invokes copy_items', async () => {
    await bootApp((cmd) => {
      if (cmd === 'get_volumes') return [{ name: 'C:', path: 'C:\\' }];
      if (cmd === 'list_dir') return [{
        name: 'file.txt', path: 'C:\\file.txt', size: 100, modified: '',
        entryType: 'File', extension: '.txt',
      }];
      if (cmd === 'copy_items') return Promise.resolve();
      return [];
    });

    // Select and copy
    const rows = document.querySelectorAll('.file-item');
    rows[0].dispatchEvent(new MouseEvent('click', { bubbles: true }));
    await flushPromises();

    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'c', ctrlKey: true, bubbles: true }));
    await flushPromises();

    // Paste via Ctrl+V
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'v', ctrlKey: true, bubbles: true }));
    await flushPromises();

    const { invoke } = await import('@tauri-apps/api/core');
    expect(invoke).toHaveBeenCalledWith(
      'copy_items',
      expect.objectContaining({ destDir: 'C:\\' }),
    );
  });
});

// ─── Scan Cancel / Reset ───

describe('Analytics — Cancel Scan', () => {
  beforeEach(() => {
    resetTauriMocks();
  });

  it('cancel button invokes cancel_scan', async () => {
    await bootApp((cmd) => {
      if (cmd === 'get_volumes') return [{ name: 'C:', path: 'C:\\' }];
      if (cmd === 'start_scan_usage') return 'scan-123';
      if (cmd === 'cancel_scan') return Promise.resolve();
      return [];
    });

    document.getElementById('btn-analytics')!.click();
    await flushPromises();
    document.getElementById('btn-scan')!.click();
    await flushPromises();

    // Click cancel
    document.getElementById('btn-cancel-scan')!.click();
    await flushPromises();

    const { invoke } = await import('@tauri-apps/api/core');
    expect(invoke).toHaveBeenCalledWith('cancel_scan', { scanId: 'scan-123' });
  });

  it('scan:error resets UI', async () => {
    await bootApp((cmd) => {
      if (cmd === 'get_volumes') return [{ name: 'C:', path: 'C:\\' }];
      if (cmd === 'start_scan_usage') return 'scan-123';
      return [];
    });

    document.getElementById('btn-analytics')!.click();
    await flushPromises();
    document.getElementById('btn-scan')!.click();
    await flushPromises();

    expect(document.getElementById('analytics-progress')!.classList.contains('hidden')).toBe(false);

    emitEvent('scan:error', { message: 'Access denied' });
    await flushPromises();

    expect(document.getElementById('analytics-progress')!.classList.contains('hidden')).toBe(true);
    expect((document.getElementById('btn-scan')! as HTMLButtonElement).disabled).toBe(false);
  });
});

// ─── Forward Navigation ───

describe('Navigation — Forward', () => {
  beforeEach(() => {
    resetTauriMocks();
  });

  it('Back then Forward restores path', async () => {
    await bootApp((cmd) => {
      if (cmd === 'get_volumes') return [{ name: 'C:', path: 'C:\\' }];
      if (cmd === 'list_dir') return [{
        name: 'docs', path: 'C:\\docs', size: 0, modified: '',
        entryType: 'Folder', extension: null,
      }];
      return [];
    });

    const breadcrumb = document.getElementById('breadcrumb')!;

    // Navigate into folder
    const rows = document.querySelectorAll('.file-item');
    rows[0].dispatchEvent(new MouseEvent('dblclick', { bubbles: true }));
    await flushPromises();
    expect(breadcrumb.textContent).toBe('C:\\docs');

    // Go back
    document.getElementById('btn-back')!.click();
    await flushPromises();
    expect(breadcrumb.textContent).toBe('C:\\');

    // Go forward — should restore C:\docs
    document.getElementById('btn-forward')!.click();
    await flushPromises();
    expect(breadcrumb.textContent).toBe('C:\\docs');
  });
});

// ─── Scan — No Path Error ───

describe('Analytics — Scan Validation', () => {
  beforeEach(() => {
    resetTauriMocks();
  });

  it('shows error when no path is set', async () => {
    await bootApp((cmd) => {
      if (cmd === 'get_volumes') return [];
      return [];
    });

    document.getElementById('btn-analytics')!.click();
    await flushPromises();

    // Clear the scan path
    const scanPath = document.getElementById('scan-path')! as HTMLInputElement;
    scanPath.value = '';

    document.getElementById('btn-scan')!.click();
    await flushPromises();

    const status = document.getElementById('status-info')!;
    expect(status.textContent).toContain('enter a path');
  });
});
