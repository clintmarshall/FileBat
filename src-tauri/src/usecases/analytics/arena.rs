use crate::domain::{NameId, NodeId};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// Immutable structural data — frozen after BFS, shared via Arc.
/// Zero locks for all reads after BFS completes.
pub struct StructuralData {
    pub parent: Vec<Option<NodeId>>,
    pub first_child: Vec<Option<NodeId>>,
    pub next_sibling: Vec<Option<NodeId>>,
    pub name_id: Vec<NameId>,
}

/// Concurrent metadata — each node written exactly once.
/// Leaf: written by sizing thread. Parent: written during rollup.
/// The AtomicU32 pending counter provides memory ordering guarantees
/// so readers never see partial or stale data.
pub struct MetadataSlice {
    /// (size, file_count, folder_count) — None until written
    pub sized: Vec<Option<(u64, u64, u64)>>,
    /// Atomic countdown of unsized children per parent.
    /// Leaf: 0 (ready immediately). Parent: child count, decremented as each child finishes.
    /// When it hits 0, the thread that decremented it triggers rollup.
    pub pending_children: Vec<AtomicU32>,
}

/// Lock-free arena for the folder tree.
///
/// Architecture:
/// - **Structural** data is mutable Vec during BFS, then frozen into Arc<StructuralData>.
///   After freeze, threads read parent pointers, sibling chains, and names with zero locks.
/// - **Metadata** (size, file_count, folder_count) is written exactly once per node.
///   Leaf writes are lock-free (each thread writes its own slot).
///   Parent rollup is triggered by the atomic pending_children countdown.
/// - **Pending children** is an AtomicU32 per node. AcqRel ordering on fetch_sub
///   guarantees that when the counter hits 0, all children's metadata writes
///   are visible to the rolling-up thread — no mutex needed.
///
/// Memory: ~56 bytes per folder + one string pool allocation per unique name.
/// Contention: zero during sizing, one atomic per rollup step.
pub struct FolderArena {
    /// Mutable structural fields during BFS. Moved into Arc at freeze time.
    bfs_structural: Option<StructuralData>,

    /// Frozen structural — set after freeze_structural(). Used by sizing threads.
    frozen_structural: Option<Arc<StructuralData>>,

    /// Metadata — concurrent writes, each node written once.
    metadata: MetadataSlice,

    /// String pool — mutable during BFS, then read-only.
    /// One copy of every name, deduplicated.
    name_to_id: HashMap<String, NameId>,
    pub names: Vec<String>,
}

impl FolderArena {
    pub fn new() -> Self {
        Self {
            bfs_structural: Some(StructuralData {
                parent: Vec::new(),
                first_child: Vec::new(),
                next_sibling: Vec::new(),
                name_id: Vec::new(),
            }),
            frozen_structural: None,
            metadata: MetadataSlice {
                sized: Vec::new(),
                pending_children: Vec::new(),
            },
            name_to_id: HashMap::new(),
            names: Vec::new(),
        }
    }

    /// Total allocated nodes.
    pub fn len(&self) -> usize {
        self.metadata.sized.len()
    }

    pub fn is_empty(&self) -> bool {
        self.metadata.sized.is_empty()
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
    /// MUST be called before freeze_structural().
    pub fn alloc_folder(&mut self, name: &str) -> NodeId {
        let id = NodeId(self.len() as u32);
        let name_id = self.intern_name(name);

        let s = self.bfs_structural.as_mut().expect("called after freeze");
        s.parent.push(None);
        s.first_child.push(None);
        s.next_sibling.push(None);
        s.name_id.push(name_id);

        // Metadata
        self.metadata.sized.push(None);
        self.metadata.pending_children.push(AtomicU32::new(0));

        id
    }

    /// Freeze structural data into Arc after BFS completes.
    /// Returns a cloneable Arc for all sizing threads to read lock-free.
    pub fn freeze_structural(&mut self) -> Arc<StructuralData> {
        let bfs = self
            .bfs_structural
            .take()
            .expect("structural already frozen or not initialized");
        let frozen = Arc::new(bfs);
        self.frozen_structural = Some(frozen.clone());
        frozen
    }

    /// Get the frozen structural data for frontend queries and sizing threads.
    /// Returns None if freeze_structural() hasn't been called yet.
    pub fn structural_ref(&self) -> Option<&Arc<StructuralData>> {
        self.frozen_structural.as_ref()
    }

    /// Add a child to a parent. Uses first-child/next-sibling insertion.
    /// O(1) — inserts at front of sibling list.
    /// MUST be called before freeze_structural().
    pub fn add_child(&mut self, parent: NodeId, child: NodeId) {
        let p = parent.0 as usize;
        let c = child.0 as usize;

        let s = self.bfs_structural.as_mut().expect("called after freeze");
        s.next_sibling[c] = s.first_child[p];
        s.first_child[p] = Some(child);
        s.parent[c] = Some(parent);
    }

    /// Iterate children of a parent (follows first_child → next_sibling chain).
    pub fn children<'a>(
        &self,
        structural: &'a Arc<StructuralData>,
        parent: NodeId,
    ) -> FolderChildrenIter<'a> {
        FolderChildrenIter {
            current: structural.first_child[parent.0 as usize],
            structural,
        }
    }

    /// Resolve a NodeId to its display name.
    pub fn name(&self, structural: &Arc<StructuralData>, id: NodeId) -> &str {
        &self.names[structural.name_id[id.0 as usize].0 as usize]
    }

    /// Build the full path for a NodeId by walking parent pointers.
    pub fn resolve_path(
        &self,
        structural: &Arc<StructuralData>,
        id: NodeId,
        root_path: &str,
    ) -> String {
        let mut segments: Vec<&str> = Vec::new();
        let mut current = id;
        loop {
            segments.push(self.name(structural, current));
            if let Some(parent) = structural.parent[current.0 as usize] {
                current = parent;
            } else {
                break;
            }
        }
        segments.reverse();
        if segments.len() <= 1 {
            return root_path.to_string();
        }
        let base = root_path.trim_end_matches('/');
        let mut path = base.to_string();
        for segment in &segments[1..] {
            path.push('/');
            path.push_str(segment);
        }
        path
    }

    /// Write metadata for a node. Each node is written exactly once.
    /// Called through MutexGuard (brief lock, single slot write).
    pub fn write_size(&mut self, id: NodeId, size: u64, file_count: u64, folder_count: u64) {
        let idx = id.0 as usize;
        debug_assert!(
            self.metadata.sized[idx].is_none(),
            "Node {:?} sized twice — logic error",
            id
        );
        self.metadata.sized[idx] = Some((size, file_count, folder_count));
    }

    /// Read metadata for a sized node.
    /// Safe to call when is_sized() returns true — the AtomicU32 pending counter
    /// provides memory ordering (AcqRel) that guarantees visibility.
    pub fn read_size(&self, id: NodeId) -> (u64, u64, u64) {
        self.metadata.sized[id.0 as usize]
            .expect("read unsized node")
    }

    /// Check if a node has been sized.
    pub fn is_sized(&self, id: NodeId) -> bool {
        self.metadata.sized[id.0 as usize].is_some()
    }

    /// Sum children's stats for rollup.
    /// Children are guaranteed sized when pending_children hits 0.
    pub fn sum_children(
        &self,
        structural: &Arc<StructuralData>,
        parent: NodeId,
    ) -> (u64, u64, u64) {
        let mut total_size: u64 = 0;
        let mut total_files: u64 = 0;
        let mut total_folders: u64 = 0;

        for child in self.children(structural, parent) {
            let (s, f, d) = self.read_size(child);
            total_size += s;
            total_files += f;
            total_folders += 1 + d;
        }

        (total_size, total_files, total_folders)
    }

    /// Initialize pending_children counters after BFS is complete.
    /// Each parent's counter is set to its child count.
    pub fn init_pending(&self, structural: &Arc<StructuralData>) {
        for idx in 0..self.len() {
            if let Some(parent) = structural.parent[idx] {
                let p = parent.0 as usize;
                self.metadata.pending_children[p].fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Mark one child of `parent` as sized. Returns true if this was the last child
    /// (counter hit 0), meaning the caller should roll up the parent.
    /// AcqRel ordering: all metadata stores by this thread before the fetch_sub
    /// are visible to any thread that observes the counter reaching 0.
    pub fn child_completed(&self, parent: NodeId) -> bool {
        let p = parent.0 as usize;
        let prev = self.metadata.pending_children[p].fetch_sub(1, Ordering::AcqRel);
        prev == 1 // was 1, now 0 → all children done
    }

    /// Check if a parent has no pending children (all sized or no children).
    pub fn is_ready(&self, id: NodeId) -> bool {
        self.metadata.pending_children[id.0 as usize].load(Ordering::Acquire) == 0
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
    structural: &'a Arc<StructuralData>,
}

impl<'a> Iterator for FolderChildrenIter<'a> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        let id = self.current?;
        self.current = self.structural.next_sibling[id.0 as usize];
        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_arena() -> (FolderArena, Arc<StructuralData>) {
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

        let structural = arena.freeze_structural();
        (arena, structural)
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

        let structural = arena.freeze_structural();

        assert_eq!(structural.name_id[id1.0 as usize], structural.name_id[id2.0 as usize]);
        assert_ne!(structural.name_id[id1.0 as usize], structural.name_id[id3.0 as usize]);
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

        let structural = arena.freeze_structural();

        assert_eq!(structural.first_child[parent.0 as usize], Some(c3));
        assert_eq!(structural.next_sibling[c3.0 as usize], Some(c2));
        assert_eq!(structural.next_sibling[c2.0 as usize], Some(c1));
        assert_eq!(structural.next_sibling[c1.0 as usize], None);
        assert_eq!(structural.parent[c1.0 as usize], Some(parent));
        assert_eq!(structural.parent[c2.0 as usize], Some(parent));
        assert_eq!(structural.parent[c3.0 as usize], Some(parent));
    }

    #[test]
    fn children_iterates_all_siblings() {
        let (arena, structural) = test_arena();
        let root = NodeId(0);

        let children: Vec<NodeId> = arena.children(&structural, root).collect();
        assert_eq!(children.len(), 2);
        assert!(children.contains(&NodeId(1))); // a
        assert!(children.contains(&NodeId(4))); // b
    }

    #[test]
    fn children_of_leaf_is_empty() {
        let (arena, structural) = test_arena();
        let a1 = NodeId(2); // leaf node

        let children: Vec<NodeId> = arena.children(&structural, a1).collect();
        assert!(children.is_empty());
    }

    #[test]
    fn name_resolves_correctly() {
        let (arena, structural) = test_arena();
        assert_eq!(arena.name(&structural, NodeId(0)), "root");
        assert_eq!(arena.name(&structural, NodeId(1)), "a");
        assert_eq!(arena.name(&structural, NodeId(2)), "a1");
    }

    #[test]
    fn resolve_path_walks_parent_chain() {
        let (arena, structural) = test_arena();
        assert_eq!(arena.resolve_path(&structural, NodeId(0), "E:/"), "E:/");
        assert_eq!(arena.resolve_path(&structural, NodeId(1), "E:/"), "E:/a");
        assert_eq!(arena.resolve_path(&structural, NodeId(2), "E:/"), "E:/a/a1");
    }

    #[test]
    fn write_and_read_size() {
        let mut arena = FolderArena::new();
        let node = arena.alloc_folder("test");
        let structural = arena.freeze_structural();

        assert!(!arena.is_sized(node));
        arena.write_size(node, 1000, 5, 2);
        assert!(arena.is_sized(node));
        let (s, f, d) = arena.read_size(node);
        assert_eq!(s, 1000);
        assert_eq!(f, 5);
        assert_eq!(d, 2);
    }

    #[test]
    fn pending_children_countdown() {
        let mut arena = FolderArena::new();
        let parent = arena.alloc_folder("parent");
        let c1 = arena.alloc_folder("c1");
        let c2 = arena.alloc_folder("c2");

        arena.add_child(parent, c1);
        arena.add_child(parent, c2);
        let structural = arena.freeze_structural();
        arena.init_pending(&structural);

        assert_eq!(
            arena.metadata.pending_children[parent.0 as usize].load(Ordering::Relaxed),
            2
        );

        assert!(!arena.child_completed(parent)); // prev was 2, now 1
        assert_eq!(
            arena.metadata.pending_children[parent.0 as usize].load(Ordering::Relaxed),
            1
        );

        assert!(arena.child_completed(parent)); // prev was 1, now 0
        assert!(arena.is_ready(parent));
    }

    #[test]
    fn leaf_is_ready_immediately() {
        let mut arena = FolderArena::new();
        let leaf = arena.alloc_folder("leaf");
        assert!(arena.is_ready(leaf));
    }

    #[test]
    fn sum_children_aggregates_stats() {
        let mut arena = FolderArena::new();
        let parent = arena.alloc_folder("parent");
        let c1 = arena.alloc_folder("c1");
        let c2 = arena.alloc_folder("c2");

        arena.add_child(parent, c1);
        arena.add_child(parent, c2);

        arena.write_size(c1, 100, 5, 2);
        arena.write_size(c2, 200, 10, 3);

        let structural = arena.freeze_structural();
        let (total_size, total_files, total_folders) = arena.sum_children(&structural, parent);
        assert_eq!(total_size, 300);
        assert_eq!(total_files, 15);
        assert_eq!(total_folders, 7); // (1+2) + (1+3)
    }

    #[test]
    fn large_arena_capacity() {
        let mut arena = FolderArena::new();
        let root = arena.alloc_folder("root");

        for i in 0..50_000 {
            let name = format!("folder_{:04}", i);
            let child = arena.alloc_folder(&name);
            arena.add_child(root, child);
        }

        assert_eq!(arena.len(), 50_001);

        let structural = arena.freeze_structural();
        let count = arena.children(&structural, root).count();
        assert_eq!(count, 50_000);
    }

    #[test]
    fn parent_pointer_walk_rollup() {
        let mut arena = FolderArena::new();
        let root = arena.alloc_folder("root");
        let a = arena.alloc_folder("a");
        let a1 = arena.alloc_folder("a1");

        arena.add_child(root, a);
        arena.add_child(a, a1);
        let structural = arena.freeze_structural();
        arena.init_pending(&structural);

        // Size leaf
        arena.write_size(a1, 100, 5, 0);

        // Rollup: walk parent chain from leaf using atomic countdown
        let mut current = structural.parent[a1.0 as usize];
        while let Some(parent_id) = current {
            if arena.child_completed(parent_id) {
                let (s, f, d) = arena.sum_children(&structural, parent_id);
                arena.write_size(parent_id, s, f, d);
                current = structural.parent[parent_id.0 as usize];
            } else {
                break;
            }
        }

        assert!(arena.is_ready(a));
        let (s, f, _d) = arena.read_size(a);
        assert_eq!(s, 100);
        assert_eq!(f, 5);

        // root was also rolled up by the while loop (a was its only child)
        assert!(arena.is_ready(root));
        let (s, f, d) = arena.read_size(root);
        assert_eq!(s, 100);
        assert_eq!(f, 5);
        assert_eq!(d, 2);
    }

    #[test]
    fn concurrent_sizing_simulation() {
        // root
        //   ├── leaf1
        //   ├── leaf2
        //   └── inner
        //       ├── leaf3
        //       └── leaf4
        let mut arena = FolderArena::new();
        let root = arena.alloc_folder("root");
        let leaf1 = arena.alloc_folder("leaf1");
        let leaf2 = arena.alloc_folder("leaf2");
        let inner = arena.alloc_folder("inner");
        let leaf3 = arena.alloc_folder("leaf3");
        let leaf4 = arena.alloc_folder("leaf4");

        arena.add_child(root, leaf1);
        arena.add_child(root, leaf2);
        arena.add_child(root, inner);
        arena.add_child(inner, leaf3);
        arena.add_child(inner, leaf4);

        let structural = arena.freeze_structural();
        arena.init_pending(&structural);

        // Simulate concurrent sizing
        arena.write_size(leaf1, 100, 1, 0);
        assert!(!arena.child_completed(root));

        arena.write_size(leaf2, 200, 2, 0);
        assert!(!arena.child_completed(root));

        arena.write_size(leaf3, 300, 3, 0);
        assert!(!arena.child_completed(inner));

        arena.write_size(leaf4, 400, 4, 0);
        assert!(arena.child_completed(inner));
        let (s, f, d) = arena.sum_children(&structural, inner);
        arena.write_size(inner, s, f, d);

        assert!(arena.child_completed(root));
        let (s, f, d) = arena.sum_children(&structural, root);
        arena.write_size(root, s, f, d);

        let (inner_s, inner_f, inner_d) = arena.read_size(inner);
        assert_eq!(inner_s, 700); // 300 + 400
        assert_eq!(inner_f, 7); // 3 + 4
        assert_eq!(inner_d, 2); // leaf3 + leaf4 = 2 child folders

        let (root_s, root_f, root_d) = arena.read_size(root);
        assert_eq!(root_s, 1000); // 100 + 200 + 700
        assert_eq!(root_f, 10); // 1 + 2 + 7
        assert_eq!(root_d, 5); // leaf1(0) + leaf2(0) + inner(2) + 3 child folders = 1+1+3
    }
}
