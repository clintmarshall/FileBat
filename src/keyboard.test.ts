import { describe, it, expect, beforeEach, vi } from 'vitest';
import { resetTauriMocks, flushPromises } from './test/helpers';
import { bootApp } from './test/boot';

async function bootWithEntries(entries: Array<{
  name: string; path: string; size: number; modified: string;
  entryType: 'File' | 'Folder' | 'Symlink' | 'Drive'; extension: string | null;
}>) {
  await bootApp((cmd) => {
    if (cmd === 'get_volumes') return [{ name: 'C:', path: 'C:\\' }];
    if (cmd === 'list_dir') return entries;
    return [];
  });
}

function dispatchKey(key: string, opts?: { ctrl?: boolean; shift?: boolean }) {
  const event = new KeyboardEvent('keydown', {
    key,
    ctrlKey: opts?.ctrl || false,
    shiftKey: opts?.shift || false,
    bubbles: true,
  });
  document.dispatchEvent(event);
  return flushPromises();
}

describe('Keyboard Navigation', () => {
  beforeEach(() => {
    resetTauriMocks();
  });

  it('Ctrl+A selects all entries', async () => {
    await bootWithEntries([
      { name: 'alpha', path: 'C:\\alpha', size: 0, modified: '', entryType: 'Folder', extension: null },
      { name: 'beta.txt', path: 'C:\\beta.txt', size: 100, modified: '', entryType: 'File', extension: '.txt' },
      { name: 'gamma.txt', path: 'C:\\gamma.txt', size: 200, modified: '', entryType: 'File', extension: '.txt' },
    ]);

    await dispatchKey('a', { ctrl: true });
    const selected = document.querySelectorAll('.file-item.selected');
    expect(selected.length).toBe(3);
  });

  it('ArrowDown moves selection down', async () => {
    await bootWithEntries([
      { name: 'alpha', path: 'C:\\alpha', size: 0, modified: '', entryType: 'Folder', extension: null },
      { name: 'beta.txt', path: 'C:\\beta.txt', size: 100, modified: '', entryType: 'File', extension: '.txt' },
      { name: 'gamma.txt', path: 'C:\\gamma.txt', size: 200, modified: '', entryType: 'File', extension: '.txt' },
    ]);

    // First ArrowDown — select index 0
    await dispatchKey('ArrowDown');
    let selected = document.querySelectorAll('.file-item.selected');
    expect(selected.length).toBe(1);
    expect(selected[0].dataset.type).toBe('Folder');

    // Second ArrowDown — move to index 1
    await dispatchKey('ArrowDown');
    selected = document.querySelectorAll('.file-item.selected');
    expect(selected[0].dataset.type).toBe('File');
  });

  it('ArrowUp moves selection up', async () => {
    await bootWithEntries([
      { name: 'alpha', path: 'C:\\alpha', size: 0, modified: '', entryType: 'Folder', extension: null },
      { name: 'beta.txt', path: 'C:\\beta.txt', size: 100, modified: '', entryType: 'File', extension: '.txt' },
    ]);

    // Select index 0
    await dispatchKey('ArrowDown');
    // Move back up — should stay at 0 (clamped)
    await dispatchKey('ArrowUp');
    const selected = document.querySelectorAll('.file-item.selected');
    expect(selected.length).toBe(1);
    expect(selected[0].dataset.type).toBe('Folder');
  });

  it('Ctrl+C triggers copy', async () => {
    await bootWithEntries([
      { name: 'file.txt', path: 'C:\\file.txt', size: 100, modified: '', entryType: 'File', extension: '.txt' },
    ]);

    await dispatchKey('ArrowDown');
    await dispatchKey('c', { ctrl: true });
    const status = document.getElementById('status-info')!;
    expect(status.textContent).toContain('Copied');
  });

  it('Ctrl+X triggers cut', async () => {
    await bootWithEntries([
      { name: 'file.txt', path: 'C:\\file.txt', size: 100, modified: '', entryType: 'File', extension: '.txt' },
    ]);

    await dispatchKey('ArrowDown');
    await dispatchKey('x', { ctrl: true });
    const status = document.getElementById('status-info')!;
    expect(status.textContent).toContain('Cut');
  });

  it('Delete key triggers delete when items selected', async () => {
    await bootWithEntries([
      { name: 'file.txt', path: 'C:\\file.txt', size: 100, modified: '', entryType: 'File', extension: '.txt' },
    ]);

    await dispatchKey('ArrowDown');
    await dispatchKey('Delete');
    const status = document.getElementById('status-info')!;
    expect(status.textContent).not.toContain('Error');
  });
});
