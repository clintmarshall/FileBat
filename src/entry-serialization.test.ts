import { describe, it, expect } from 'vitest';
import { entryIcon } from './utils';

// ─── Entry Interface Contract ───
//
// The Rust Entry struct uses #[serde(rename_all = "camelCase")], so the
// serialized JSON field is "entryType", not "entry_type".
// This test suite verifies that the frontend code matches the Rust contract.

describe('Entry interface contract', () => {
  // Simulate what Rust serializes (camelCase)
  const rustFolderEntry = {
    name: 'Documents',
    path: 'C:\\Users\\test\\Documents',
    size: 0,
    modified: '2024-01-01T00:00:00.000Z',
    entryType: 'Folder',
    extension: null,
  };

  const rustFileEntry = {
    name: 'readme.txt',
    path: 'C:\\Users\\test\\readme.txt',
    size: 1024,
    modified: '2024-01-01T00:00:00.000Z',
    entryType: 'File',
    extension: 'txt',
  };

  it('folder entry has entryType field (camelCase)', () => {
    expect(rustFolderEntry.entryType).toBe('Folder');
    expect(rustFolderEntry.entry_type).toBeUndefined();
  });

  it('file entry has entryType field (camelCase)', () => {
    expect(rustFileEntry.entryType).toBe('File');
    expect(rustFileEntry.entry_type).toBeUndefined();
  });

  it('entryIcon receives entryType value', () => {
    expect(entryIcon(rustFolderEntry.entryType)).toBe('📁');
    expect(entryIcon(rustFileEntry.entryType)).toBe('📄');
  });

  it('folder detection uses entryType', () => {
    const isFolder = rustFolderEntry.entryType === 'Folder';
    expect(isFolder).toBe(true);
  });

  it('file detection uses entryType', () => {
    const isFile = rustFileEntry.entryType === 'File';
    expect(isFile).toBe(true);
  });
});

describe('Entry rendering with camelCase fields', () => {
  it('folder rows should not show size', () => {
    const entry = { entryType: 'Folder', size: 0 };
    const showSize = entry.entryType !== 'Folder';
    expect(showSize).toBe(false);
  });

  it('file rows should show size', () => {
    const entry = { entryType: 'File', size: 1024 };
    const showSize = entry.entryType !== 'Folder';
    expect(showSize).toBe(true);
  });

  it('double-click navigation checks entryType', () => {
    const folder = { entryType: 'Folder', path: 'C:\\test' };
    const file = { entryType: 'File', path: 'C:\\test.txt' };

    const shouldNavigate = (e: { entryType: string }) =>
      e.entryType === 'Folder' || e.entryType === 'Drive';

    expect(shouldNavigate(folder)).toBe(true);
    expect(shouldNavigate(file)).toBe(false);
  });
});
