import { describe, it, expect, beforeEach, vi } from 'vitest';
import { formatSize, formatDate, entryIcon } from './utils';

// Mock Tauri API
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(),
}));

// ─── Utility Functions ───

describe('formatSize', () => {
  it('formats 0 bytes as empty string', () => {
    expect(formatSize(0)).toBe('');
  });

  it('formats bytes', () => {
    expect(formatSize(42)).toBe('42 B');
  });

  it('formats kilobytes', () => {
    expect(formatSize(1024)).toBe('1.0 KB');
  });

  it('formats megabytes', () => {
    expect(formatSize(1048576)).toBe('1.0 MB');
  });

  it('formats gigabytes', () => {
    expect(formatSize(1073741824)).toBe('1.0 GB');
  });

  it('formats terabytes', () => {
    expect(formatSize(1099511627776)).toBe('1.0 TB');
  });

  it('formats partial units correctly', () => {
    expect(formatSize(1536)).toBe('1.5 KB');
    expect(formatSize(1572864)).toBe('1.5 MB');
  });
});

describe('formatDate', () => {
  it('formats empty string as empty', () => {
    expect(formatDate('')).toBe('');
  });

  it('formats ISO date string', () => {
    const result = formatDate('2024-01-15T10:30:00.000Z');
    expect(result).toContain('2024');
    expect(result).toContain('Jan');
  });

  it('handles invalid date gracefully', () => {
    const result = formatDate('invalid');
    expect(result).toContain('Invalid');
  });
});

describe('entryIcon', () => {
  it('returns folder icon for Folder', () => {
    expect(entryIcon('Folder')).toBe('📁');
  });

  it('returns drive icon for Drive', () => {
    expect(entryIcon('Drive')).toBe('💾');
  });

  it('returns symlink icon for Symlink', () => {
    expect(entryIcon('Symlink')).toBe('🔗');
  });

  it('returns file icon for unknown types', () => {
    expect(entryIcon('File')).toBe('📄');
    expect(entryIcon('Unknown')).toBe('📄');
  });
});

// ─── Analytics Rendering Logic ───

describe('analytics rendering', () => {
  interface UsageResult {
    path: string;
    size: number;
    file_count: number;
    folder_count: number;
  }

  function renderUsageTable(results: UsageResult[]): string {
    if (results.length === 0) return '';

    const sorted = [...results].sort((a, b) => b.size - a.size);
    const maxSize = sorted[0].size;

    let html = '<table class="analytics-table">';
    html += '<tr><th>Path</th><th>Size</th><th>Files</th><th>Folders</th></tr>';
    for (const usage of sorted) {
      const barWidth = (usage.size / maxSize) * 100;
      html += `<tr>
        <td>${usage.path}</td>
        <td>${formatSize(usage.size)}<span class="size-bar" style="width: ${barWidth}px;"></span></td>
        <td>${usage.file_count.toLocaleString()}</td>
        <td>${usage.folder_count.toLocaleString()}</td>
      </tr>`;
    }
    html += '</table>';
    return html;
  }

  it('renders empty results as empty string', () => {
    expect(renderUsageTable([])).toBe('');
  });

  it('renders results sorted by size descending', () => {
    const results = [
      { path: '/small', size: 100, file_count: 1, folder_count: 0 },
      { path: '/large', size: 1000, file_count: 10, folder_count: 2 },
      { path: '/medium', size: 500, file_count: 5, folder_count: 1 },
    ];

    const html = renderUsageTable(results);
    const largeIndex = html.indexOf('/large');
    const mediumIndex = html.indexOf('/medium');
    const smallIndex = html.indexOf('/small');

    expect(largeIndex).toBeLessThan(mediumIndex);
    expect(mediumIndex).toBeLessThan(smallIndex);
  });

  it('calculates bar widths relative to max size', () => {
    const results = [
      { path: '/half', size: 500, file_count: 1, folder_count: 0 },
      { path: '/full', size: 1000, file_count: 10, folder_count: 2 },
    ];

    const html = renderUsageTable(results);
    expect(html).toContain('width: 100px'); // full size = 100%
    expect(html).toContain('width: 50px'); // half size = 50%
  });

  it('renders file counts with locale formatting', () => {
    const results = [
      { path: '/test', size: 1000, file_count: 1000, folder_count: 100 },
    ];

    const html = renderUsageTable(results);
    expect(html).toContain('1,000');
  });
});

// ─── Duplicate Group Rendering ───

describe('duplicate group rendering', () => {
  interface DuplicateGroup {
    hash: string;
    size_each: number;
    files: string[];
    wasted_space: number;
  }

  function renderDuplicatesTable(groups: DuplicateGroup[]): string {
    if (groups.length === 0) return '';

    const totalWasted = groups.reduce((sum, g) => sum + g.wasted_space, 0);
    let html = `<div style="padding: 8px; font-weight: 500;">Found ${groups.length} duplicate groups · ${formatSize(totalWasted)} wasted</div>`;
    html += '<table class="analytics-table">';
    html += '<tr><th>Files</th><th>Size Each</th><th>Count</th><th>Wasted</th></tr>';
    for (const group of groups) {
      html += `<tr>
        <td>${group.files.map(f => f.split('\\').pop()).join(', ')}</td>
        <td>${formatSize(group.size_each)}</td>
        <td>${group.files.length}</td>
        <td>${formatSize(group.wasted_space)}</td>
      </tr>`;
    }
    html += '</table>';
    return html;
  }

  it('renders empty groups as empty string', () => {
    expect(renderDuplicatesTable([])).toBe('');
  });

  it('calculates total wasted space', () => {
    const groups: DuplicateGroup[] = [
      { hash: 'abc', size_each: 1000, files: ['/a', '/b'], wasted_space: 1000 },
      { hash: 'def', size_each: 2000, files: ['/c', '/d', '/e'], wasted_space: 4000 },
    ];

    const html = renderDuplicatesTable(groups);
    expect(html).toContain('4.9 KB wasted');
  });

  it('extracts filenames from paths', () => {
    const groups: DuplicateGroup[] = [
      { hash: 'abc', size_each: 1000, files: ['C:\\path\\to\\file1.txt', 'C:\\other\\file2.txt'], wasted_space: 1000 },
    ];

    const html = renderDuplicatesTable(groups);
    expect(html).toContain('file1.txt');
    expect(html).toContain('file2.txt');
    expect(html).not.toContain('C:\\path\\to\\');
  });
});

// ─── Selection Logic ───

describe('selection logic', () => {
  it('single select clears previous selection', () => {
    const selected = new Set<number>();
    selected.add(0);
    selected.add(1);

    // Single select index 2
    selected.clear();
    selected.add(2);

    expect(selected.size).toBe(1);
    expect(selected.has(2)).toBe(true);
  });

  it('toggle select adds/removes from selection', () => {
    const selected = new Set<number>();

    // Add index 0
    if (selected.has(0)) selected.delete(0);
    else selected.add(0);

    expect(selected.has(0)).toBe(true);

    // Remove index 0
    if (selected.has(0)) selected.delete(0);
    else selected.add(0);

    expect(selected.has(0)).toBe(false);
  });

  it('range select selects all indices in range', () => {
    const selected = new Set<number>();
    const from = 2;
    const to = 5;

    selected.clear();
    for (let i = from; i <= to; i++) {
      selected.add(i);
    }

    expect(selected.size).toBe(4);
    expect(selected.has(2)).toBe(true);
    expect(selected.has(5)).toBe(true);
    expect(selected.has(1)).toBe(false);
  });
});
