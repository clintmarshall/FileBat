import { describe, it, expect } from 'vitest';

// ─── Types (mirrored from app.ts for testing) ───

interface FolderUsage {
  path: string;
  size: number;
  fileCount: number;
  folderCount: number;
}

interface UsageTreeNode {
  path: string;
  name: string;
  size: number;
  fileCount: number;
  folderCount: number;
  children: UsageTreeNode[];
}

// ─── buildUsageTree (inline for testing — same implementation as app.ts) ───

function buildUsageTree(usages: FolderUsage[], rootPath: string): UsageTreeNode[] {
  if (usages.length === 0) return [];

  // Normalize root path for consistent comparison
  const normalizedRoot = rootPath.replace(/\\/g, '/');

  // Build a map of path -> node
  const nodeMap = new Map<string, UsageTreeNode>();

  for (const usage of usages) {
    const path = usage.path.replace(/\\/g, '/');
    // Name is the last segment of the path
    const name = path.split('/').pop() || path;

    nodeMap.set(path, {
      path: usage.path,
      name,
      size: usage.size,
      fileCount: usage.fileCount,
      folderCount: usage.folderCount,
      children: [],
    });
  }

  // Link children to parents
  const roots: UsageTreeNode[] = [];

  for (const [path, node] of nodeMap) {
    if (path === normalizedRoot) {
      roots.push(node);
      continue;
    }

    // Find parent: the node whose path is the longest prefix of this path
    // Parent path = everything up to (but not including) this node's name
    const lastSlash = path.lastIndexOf('/');
    // slice(0, lastSlash) gives parent, but handle trailing-slash roots:
    // "C:/Windows" -> lastSlash=1 -> "C:" -> check "C:" and "C:/"
    const parentPath = lastSlash > 0 ? path.slice(0, lastSlash) : null;

    let parent: UsageTreeNode | null = null;
    if (parentPath) {
      // Try exact match first, then with trailing slash (for drive roots like C:)
      parent = nodeMap.get(parentPath) ?? null;
      if (parent === null && !parentPath.endsWith('/')) {
        parent = nodeMap.get(parentPath + '/') ?? null;
      }
    }

    if (parent) {
      parent.children.push(node);
    } else {
      // No parent in the map — it's a root node (or orphan)
      roots.push(node);
    }
  }

  // Sort children by size descending at each level
  function sortChildren(nodes: UsageTreeNode[]) {
    for (const node of nodes) {
      node.children.sort((a, b) => b.size - a.size);
      sortChildren(node.children);
    }
  }

  roots.sort((a, b) => b.size - a.size);
  sortChildren(roots);

  return roots;
}

// ─── Tests ───

describe('buildUsageTree', () => {
  it('returns empty array for empty input', () => {
    const result = buildUsageTree([], 'C:/');
    expect(result).toEqual([]);
  });

  it('returns single root node', () => {
    const usages: FolderUsage[] = [
      { path: 'C:/', size: 1000, fileCount: 5, folderCount: 1 },
    ];
    const result = buildUsageTree(usages, 'C:/');
    expect(result).toHaveLength(1);
    expect(result[0].name).toBe('C:/');
    expect(result[0].size).toBe(1000);
    expect(result[0].children).toHaveLength(0);
  });

  it('builds two-level hierarchy', () => {
    const usages: FolderUsage[] = [
      { path: 'C:/', size: 5000, fileCount: 10, folderCount: 2 },
      { path: 'C:/Windows', size: 3000, fileCount: 8, folderCount: 1 },
      { path: 'C:/Users', size: 2000, fileCount: 2, folderCount: 1 },
    ];
    const result = buildUsageTree(usages, 'C:/');

    expect(result).toHaveLength(1);
    expect(result[0].name).toBe('C:/');
    expect(result[0].children).toHaveLength(2);

    // Children sorted by size descending
    expect(result[0].children[0].name).toBe('Windows');
    expect(result[0].children[0].size).toBe(3000);
    expect(result[0].children[1].name).toBe('Users');
    expect(result[0].children[1].size).toBe(2000);
  });

  it('builds three-level hierarchy', () => {
    const usages: FolderUsage[] = [
      { path: 'C:/', size: 10000, fileCount: 20, folderCount: 3 },
      { path: 'C:/Users', size: 5000, fileCount: 10, folderCount: 1 },
      { path: 'C:/Users/Clint', size: 5000, fileCount: 10, folderCount: 1 },
    ];
    const result = buildUsageTree(usages, 'C:/');

    expect(result).toHaveLength(1);
    expect(result[0].children).toHaveLength(1);
    expect(result[0].children[0].name).toBe('Users');
    expect(result[0].children[0].children).toHaveLength(1);
    expect(result[0].children[0].children[0].name).toBe('Clint');
  });

  it('handles sibling folders at the same level', () => {
    const usages: FolderUsage[] = [
      { path: 'C:/', size: 9000, fileCount: 15, folderCount: 3 },
      { path: 'C:/Windows', size: 3000, fileCount: 5, folderCount: 0 },
      { path: 'C:/Users', size: 4000, fileCount: 5, folderCount: 0 },
      { path: 'C:/Program Files', size: 2000, fileCount: 5, folderCount: 0 },
    ];
    const result = buildUsageTree(usages, 'C:/');

    expect(result).toHaveLength(1);
    expect(result[0].children).toHaveLength(3);

    // Sorted by size descending
    expect(result[0].children[0].name).toBe('Users');
    expect(result[0].children[1].name).toBe('Windows');
    expect(result[0].children[2].name).toBe('Program Files');
  });

  it('handles multiple root nodes when scan root is missing', () => {
    // If the root folder is not in the usages, children become roots
    const usages: FolderUsage[] = [
      { path: 'C:/Windows', size: 3000, fileCount: 5, folderCount: 0 },
      { path: 'C:/Users', size: 4000, fileCount: 5, folderCount: 0 },
    ];
    const result = buildUsageTree(usages, 'C:/');

    expect(result).toHaveLength(2);
    // Roots sorted by size descending
    expect(result[0].name).toBe('Users');
    expect(result[1].name).toBe('Windows');
  });

  it('handles backslash paths', () => {
    const usages: FolderUsage[] = [
      { path: 'C:\\', size: 5000, fileCount: 10, folderCount: 2 },
      { path: 'C:\\Windows', size: 3000, fileCount: 8, folderCount: 1 },
      { path: 'C:\\Users', size: 2000, fileCount: 2, folderCount: 1 },
    ];
    const result = buildUsageTree(usages, 'C:\\');

    expect(result).toHaveLength(1);
    expect(result[0].children).toHaveLength(2);
    expect(result[0].children[0].name).toBe('Windows');
    expect(result[0].children[1].name).toBe('Users');
  });

  it('preserves original path in node', () => {
    const usages: FolderUsage[] = [
      { path: 'C:\\', size: 5000, fileCount: 10, folderCount: 1 },
      { path: 'C:\\Windows', size: 3000, fileCount: 8, folderCount: 0 },
    ];
    const result = buildUsageTree(usages, 'C:\\');

    expect(result[0].path).toBe('C:\\');
    expect(result[0].children[0].path).toBe('C:\\Windows');
  });

  it('recursively sorts nested children by size', () => {
    const usages: FolderUsage[] = [
      { path: 'C:/', size: 20000, fileCount: 30, folderCount: 4 },
      { path: 'C:/Users', size: 10000, fileCount: 15, folderCount: 2 },
      { path: 'C:/Users/Clint', size: 6000, fileCount: 10, folderCount: 0 },
      { path: 'C:/Users/Admin', size: 4000, fileCount: 5, folderCount: 0 },
      { path: 'C:/Windows', size: 9000, fileCount: 15, folderCount: 0 },
    ];
    const result = buildUsageTree(usages, 'C:/');

    expect(result).toHaveLength(1);
    expect(result[0].children).toHaveLength(2);

    // Top-level children sorted by size
    expect(result[0].children[0].name).toBe('Users');
    expect(result[0].children[1].name).toBe('Windows');

    // Nested children also sorted by size
    const usersNode = result[0].children[0];
    expect(usersNode.children).toHaveLength(2);
    expect(usersNode.children[0].name).toBe('Clint');
    expect(usersNode.children[1].name).toBe('Admin');
  });

  it('handles deeply nested paths', () => {
    const usages: FolderUsage[] = [
      { path: 'C:/', size: 100, fileCount: 1, folderCount: 1 },
      { path: 'C:/a', size: 100, fileCount: 1, folderCount: 1 },
      { path: 'C:/a/b', size: 100, fileCount: 1, folderCount: 1 },
      { path: 'C:/a/b/c', size: 100, fileCount: 1, folderCount: 1 },
      { path: 'C:/a/b/c/d', size: 100, fileCount: 1, folderCount: 0 },
    ];
    const result = buildUsageTree(usages, 'C:/');

    expect(result).toHaveLength(1);
    expect(result[0].children[0].name).toBe('a');
    expect(result[0].children[0].children[0].name).toBe('b');
    expect(result[0].children[0].children[0].children[0].name).toBe('c');
    expect(result[0].children[0].children[0].children[0].children[0].name).toBe('d');
  });
});
