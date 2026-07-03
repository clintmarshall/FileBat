import { describe, it, expect, beforeEach, vi } from 'vitest';
import {
  resetTauriMocks,
  bootApp,
  flushPromises,
  emitEvent,
  registeredHandlers,
} from './test/helpers';
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
      expect.objectContaining({ path: 'C:\\', maxDepth: 2 }),
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

    emitEvent('scan:chunk', {
      scanId: 'test_scan',
      data: {
        type: 'folder_usage',
        usage: { path: 'C:\\Windows', size: 5368709120, fileCount: 12345, folderCount: 890 },
      },
    });
    await flushPromises();

    const results = document.getElementById('usage-results')!;
    expect(results.innerHTML).toContain('C:\\Windows');
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
