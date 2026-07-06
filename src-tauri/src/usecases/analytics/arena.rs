use crate::domain::{NameId, NodeId};
use std::collections::HashMap;

/// Structure-of-arrays arena for the folder tree.
///
/// Only folders are stored — files are counted during sizing, not persisted.
/// First-child/next-sibling representation: zero heap allocation per directory.
///
/// Memory: ~48 bytes per folder + one string pool allocation per unique name.
pub struct FolderArena {
    // Structural — accessed during BFS and tree traversal
    pub parent: Vec<Option<NodeId>>,
    pub first_child: Vec<Option<NodeId>>,
    pub next_sibling: Vec<Option<NodeId>>,
    pub name_id: Vec<NameId>,

    // Metadata — separate so rollup doesn't pollute structural cache lines
    pub size: Vec<u64>,
    pub file_count: Vec<u64>,
    pub folder_count: Vec<u64>,
    pub sized: Vec<bool>,

    // String pool — one copy of every name, deduplicated
    pub name_to_id: HashMap<String, NameId>,
    pub names: Vec<String>,
}

impl FolderArena {
    pub fn new() -> Self {
        Self {
            parent: Vec::new(),
            first_child: Vec::new(),
            next_sibling: Vec::new(),
            name_id: Vec::new(),
            size: Vec::new(),
            file_count: Vec::new(),
            folder_count: Vec::new(),
            sized: Vec::new(),
            name_to_id: HashMap::new(),
            names: Vec::new(),
        }
    }

    /// Total allocated nodes.
    pub fn len(&self) -> usize {
        self.parent.len()
    }

    pub fn is_empty(&self) -> bool {
        self.parent.is_empty()
    }

    /// Intern a name into the string pool. Returns existing or new NameId.
    fn intern_name(&mut self, name: &str) -> NameId {
        if let Some(&id) = self.name_to_id.get(name) {
            return id;
        }
        let id = NameId(self.names.len() as u32);
        self.name_to_id.insert(name.to_string(), id);
        self.names.push(name.to_string());
        id
    }

    /// Allocate a folder node. Returns its NodeId.
    /// Name is interned into the string pool (deduplicated).
    pub fn alloc_folder(&mut self, name: &str) -> NodeId {
        let id = NodeId(self.len() as u32);
        let name_id = self.intern_name(name);

        self.parent.push(None);
        self.first_child.push(None);
        self.next_sibling.push(None);
        self.name_id.push(name_id);
        self.size.push(0);
        self.file_count.push(0);
        self.folder_count.push(0);
        self.sized.push(false);

        id
    }

    /// Add a child to a parent. Uses first-child/next-sibling insertion.
    /// O(1) — inserts at front of sibling list.
    pub fn add_child(&mut self, parent: NodeId, child: NodeId) {
        let p = parent.0 as usize;
        let c = child.0 as usize;

        // New child becomes first child, old first child becomes next sibling
        self.next_sibling[c] = self.first_child[p];
        self.first_child[p] = Some(child);
        self.parent[c] = Some(parent);
    }

    /// Iterate children of a parent (follows first_child → next_sibling chain).
    /// Note: iteration order is reverse insertion order (LIFO due to front insertion).
    pub fn children(&self, parent: NodeId) -> FolderChildrenIter<'_> {
        FolderChildrenIter {
            current: self.first_child[parent.0 as usize],
            arena: self,
        }
    }

    /// Resolve a NodeId to its display name.
    pub fn name(&self, id: NodeId) -> &str {
        &self.names[self.name_id[id.0 as usize].0 as usize]
    }

    /// Build the full path for a NodeId by walking parent pointers.
    pub fn resolve_path(&self, id: NodeId, root_path: &str) -> String {
        // Collect path segments by walking up
        let mut segments: Vec<&str> = Vec::new();
        let mut current = id;
        loop {
            segments.push(self.name(current));
            if let Some(parent) = self.parent[current.0 as usize] {
                current = parent;
            } else {
                break;
            }
        }
        // segments are [child, ..., root_name], reverse to [root_name, ..., child]
        segments.reverse();
        // Skip the root name (first segment) — use root_path as base
        if segments.len() <= 1 {
            return root_path.to_string();
        }
        // Strip trailing slash from root_path to avoid double slashes
        let base = root_path.trim_end_matches('/');
        let mut path = base.to_string();
        for segment in &segments[1..] {
            path.push('/');
            path.push_str(segment);
        }
        path
    }

    /// Check if all children of a parent are sized.
    pub fn all_children_sized(&self, parent: NodeId) -> bool {
        for child in self.children(parent) {
            if !self.sized[child.0 as usize] {
                return false;
            }
        }
        true
    }

    /// Sum children's stats for rollup.
    pub fn sum_children(&self, parent: NodeId) -> (u64, u64, u64) {
        let mut total_size: u64 = 0;
        let mut total_files: u64 = 0;
        let mut total_folders: u64 = 0;

        for child in self.children(parent) {
            total_size += self.size[child.0 as usize];
            total_files += self.file_count[child.0 as usize];
            total_folders += 1 + self.folder_count[child.0 as usize];
        }

        (total_size, total_files, total_folders)
    }
}

impl Default for FolderArena {
    fn default() -> Self {
        Self::new()
    }
}

/// Iterator over children of a parent node (first-child / next-sibling).
pub struct FolderChildrenIter<'a> {
    current: Option<NodeId>,
    arena: &'a FolderArena,
}

impl<'a> Iterator for FolderChildrenIter<'a> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        let id = self.current?;
        self.current = self.arena.next_sibling[id.0 as usize];
        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_arena() -> FolderArena {
        // Build: root
        //   ├── a
        //   │   ├── a1
        //   │   └── a2
        //   └── b
        let mut arena = FolderArena::new();
        let root = arena.alloc_folder("root");
        let a = arena.alloc_folder("a");
        let a1 = arena.alloc_folder("a1");
        let a2 = arena.alloc_folder("a2");
        let b = arena.alloc_folder("b");

        arena.add_child(root, a);
        arena.add_child(root, b);
        arena.add_child(a, a1);
        arena.add_child(a, a2);

        arena
    }

    #[test]
    fn alloc_folder_returns_sequential_ids() {
        let mut arena = FolderArena::new();
        assert_eq!(arena.alloc_folder("x"), NodeId(0));
        assert_eq!(arena.alloc_folder("y"), NodeId(1));
        assert_eq!(arena.alloc_folder("z"), NodeId(2));
        assert_eq!(arena.len(), 3);
    }

    #[test]
    fn string_pool_deduplicates_names() {
        let mut arena = FolderArena::new();
        let id1 = arena.alloc_folder("Documents");
        let id2 = arena.alloc_folder("Documents");
        let id3 = arena.alloc_folder("Other");

        // Names should be interned — same NameId for "Documents"
        assert_eq!(arena.name_id[id1.0 as usize], arena.name_id[id2.0 as usize]);
        assert_ne!(arena.name_id[id1.0 as usize], arena.name_id[id3.0 as usize]);

        // Only 2 unique names in the pool
        assert_eq!(arena.names.len(), 2);
        assert_eq!(arena.name_to_id.len(), 2);
    }

    #[test]
    fn add_child_links_via_first_child_and_next_sibling() {
        let mut arena = FolderArena::new();
        let parent = arena.alloc_folder("parent");
        let c1 = arena.alloc_folder("c1");
        let c2 = arena.alloc_folder("c2");
        let c3 = arena.alloc_folder("c3");

        arena.add_child(parent, c1);
        arena.add_child(parent, c2);
        arena.add_child(parent, c3);

        // First child is c3 (last added, front insertion)
        assert_eq!(arena.first_child[parent.0 as usize], Some(c3));

        // c3 → c2 → c1 → None (reverse insertion order)
        assert_eq!(arena.next_sibling[c3.0 as usize], Some(c2));
        assert_eq!(arena.next_sibling[c2.0 as usize], Some(c1));
        assert_eq!(arena.next_sibling[c1.0 as usize], None);

        // All parents set correctly
        assert_eq!(arena.parent[c1.0 as usize], Some(parent));
        assert_eq!(arena.parent[c2.0 as usize], Some(parent));
        assert_eq!(arena.parent[c3.0 as usize], Some(parent));
    }

    #[test]
    fn children_iterates_all_siblings() {
        let arena = test_arena();
        let root = NodeId(0);

        let children: Vec<NodeId> = arena.children(root).collect();
        assert_eq!(children.len(), 2);
        assert!(children.contains(&NodeId(1))); // a
        assert!(children.contains(&NodeId(4))); // b
    }

    #[test]
    fn children_of_leaf_is_empty() {
        let arena = test_arena();
        let a1 = NodeId(2); // leaf node

        let children: Vec<NodeId> = arena.children(a1).collect();
        assert!(children.is_empty());
    }

    #[test]
    fn name_resolves_correctly() {
        let arena = test_arena();
        assert_eq!(arena.name(NodeId(0)), "root");
        assert_eq!(arena.name(NodeId(1)), "a");
        assert_eq!(arena.name(NodeId(2)), "a1");
    }

    #[test]
    fn resolve_path_walks_parent_chain() {
        let arena = test_arena();
        // root = NodeId(0), a = NodeId(1), a1 = NodeId(2)
        assert_eq!(arena.resolve_path(NodeId(0), "E:/"), "E:/");
        assert_eq!(arena.resolve_path(NodeId(1), "E:/"), "E:/a");
        assert_eq!(arena.resolve_path(NodeId(2), "E:/"), "E:/a/a1");
    }

    #[test]
    fn sized_flag_tracks_completion() {
        let mut arena = FolderArena::new();
        let leaf = arena.alloc_folder("leaf");

        assert!(!arena.sized[leaf.0 as usize]);
        arena.sized[leaf.0 as usize] = true;
        assert!(arena.sized[leaf.0 as usize]);
    }

    #[test]
    fn all_children_sized_returns_correct_result() {
        let mut arena = FolderArena::new();
        let parent = arena.alloc_folder("parent");
        let c1 = arena.alloc_folder("c1");
        let c2 = arena.alloc_folder("c2");

        arena.add_child(parent, c1);
        arena.add_child(parent, c2);

        // Neither sized
        assert!(!arena.all_children_sized(parent));

        // One sized
        arena.sized[c1.0 as usize] = true;
        assert!(!arena.all_children_sized(parent));

        // Both sized
        arena.sized[c2.0 as usize] = true;
        assert!(arena.all_children_sized(parent));
    }

    #[test]
    fn sum_children_aggregates_stats() {
        let mut arena = FolderArena::new();
        let parent = arena.alloc_folder("parent");
        let c1 = arena.alloc_folder("c1");
        let c2 = arena.alloc_folder("c2");

        arena.add_child(parent, c1);
        arena.add_child(parent, c2);

        // Set child stats
        arena.size[c1.0 as usize] = 100;
        arena.file_count[c1.0 as usize] = 5;
        arena.folder_count[c1.0 as usize] = 2;

        arena.size[c2.0 as usize] = 200;
        arena.file_count[c2.0 as usize] = 10;
        arena.folder_count[c2.0 as usize] = 3;

        let (total_size, total_files, total_folders) = arena.sum_children(parent);
        assert_eq!(total_size, 300);
        assert_eq!(total_files, 15);
        // total_folders = (1 + c1.folder_count) + (1 + c2.folder_count) = 3 + 4 = 7
        assert_eq!(total_folders, 7);
    }

    #[test]
    fn large_arena_capacity() {
        let mut arena = FolderArena::new();
        let root = arena.alloc_folder("root");

        // Allocate 50K nodes — check no panic, reasonable memory
        for i in 0..50_000 {
            let name = format!("folder_{:04}", i);
            let child = arena.alloc_folder(&name);
            arena.add_child(root, child);
        }

        assert_eq!(arena.len(), 50_001);

        // All children reachable (though iteration order is reversed)
        let count = arena.children(root).count();
        assert_eq!(count, 50_000);
    }

    #[test]
    fn parent_pointer_walk_rollup() {
        // Simulate: root → a → a1 (leaf)
        let mut arena = FolderArena::new();
        let root = arena.alloc_folder("root");
        let a = arena.alloc_folder("a");
        let a1 = arena.alloc_folder("a1");

        arena.add_child(root, a);
        arena.add_child(a, a1);

        // Size leaf
        arena.size[a1.0 as usize] = 100;
        arena.file_count[a1.0 as usize] = 5;
        arena.folder_count[a1.0 as usize] = 0;
        arena.sized[a1.0 as usize] = true;

        // Rollup: walk parent chain from leaf
        let mut current = arena.parent[a1.0 as usize]; // Some(a)
        while let Some(parent_id) = current {
            if arena.all_children_sized(parent_id) {
                let (s, f, d) = arena.sum_children(parent_id);
                arena.size[parent_id.0 as usize] = s;
                arena.file_count[parent_id.0 as usize] = f;
                arena.folder_count[parent_id.0 as usize] = d;
                arena.sized[parent_id.0 as usize] = true;
                current = arena.parent[parent_id.0 as usize];
            } else {
                break;
            }
        }

        // a should be rolled up
        assert!(arena.sized[a.0 as usize]);
        assert_eq!(arena.size[a.0 as usize], 100);
        assert_eq!(arena.file_count[a.0 as usize], 5);

        // root should be rolled up (a is its only child, now sized)
        assert!(arena.sized[root.0 as usize]);
        assert_eq!(arena.size[root.0 as usize], 100);
        assert_eq!(arena.file_count[root.0 as usize], 5);
        assert_eq!(arena.folder_count[root.0 as usize], 2); // a + a1 = 2 descendant folders
    }
}
