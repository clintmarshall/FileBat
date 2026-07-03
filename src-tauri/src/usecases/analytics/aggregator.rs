use crate::domain::FolderUsage;
use std::collections::HashMap;
use std::path::Path;

/// Normalize a path to forward slashes for consistent HashMap keys.
///
/// `walkdir` on Windows returns paths with `/` separators, but `PathBuf::to_string_lossy()`
/// uses `\`. Without normalization, the same folder gets two entries in the map.
fn normalize_path(path: &std::path::Path) -> String {
    path.to_string_lossy().to_string().replace('\\', "/")
}

/// Pure accumulation logic for disk usage analytics.
///
/// This struct owns the state math (HashMap updates, depth calculations,
/// ancestor traversal) so the Use Cases stay focused on orchestration
/// (walkdir iteration, cancel checks, event emission).
pub struct FolderUsageAccumulator {
    /// Per-folder accumulated stats, keyed by normalized path string.
    folder_map: HashMap<String, FolderUsage>,
    /// Total files seen across all folders.
    total_files: u64,
    /// Maximum ancestor depth to attribute file sizes to.
    max_depth: u32,
    /// Base path component count (for depth calculation).
    base_depth: u32,
}

impl FolderUsageAccumulator {
    pub fn new(base_path: &str, max_depth: u32) -> Self {
        let base = Path::new(base_path);
        let base_depth = base.components().count() as u32;

        // Seed the root folder with normalized path
        let root_key = normalize_path(base);
        let mut folder_map = HashMap::new();
        folder_map.insert(root_key, FolderUsage {
            path: normalize_path(base),
            size: 0,
            file_count: 0,
            folder_count: 1,
            depth: 0,
        });

        Self {
            folder_map,
            total_files: 0,
            max_depth,
            base_depth,
        }
    }

    /// Record a file at the given path with the given size.
    ///
    /// Attributes the file size to every ancestor **folder** up to max_depth.
    /// Starts from the file's parent directory, not the file itself.
    pub fn record_file(&mut self, path: &std::path::Path, size: u64) {
        self.total_files += 1;

        // Start from the file's parent directory, not the file itself
        let mut current = path.parent().map(|p| p.to_path_buf());
        for _d in 0..=self.max_depth {
            let ancestor = match current {
                Some(ref p) => p,
                None => break,
            };

            let ancestor_count = ancestor.components().count() as u32;
            if ancestor_count < self.base_depth {
                // Ancestor is above the base path (e.g., symlink target outside the tree)
                break;
            }
            let key = normalize_path(ancestor);
            let folder_depth = ancestor_count - self.base_depth;

            let entry = self
                .folder_map
                .entry(key)
                .or_insert_with(|| FolderUsage {
                    path: normalize_path(ancestor),
                    size: 0,
                    file_count: 0,
                    folder_count: 0,
                    depth: folder_depth,
                });

            entry.size += size;
            entry.file_count += 1;

            // Move to parent for next iteration
            current = ancestor.parent().map(|p| p.to_path_buf());
        }
    }

    /// Record a directory at the given path.
    ///
    /// Creates a FolderUsage entry if one doesn't already exist for this path.
    pub fn record_directory(&mut self, path: &std::path::Path) {
        let key = normalize_path(path);
        let path_count = path.components().count() as u32;
        if path_count < self.base_depth {
            // Path is above the base (e.g., symlink target outside the tree)
            return;
        }
        let folder_depth = path_count - self.base_depth;

        self.folder_map.entry(key).or_insert_with(|| FolderUsage {
            path: normalize_path(path),
            size: 0,
            file_count: 0,
            folder_count: 1,
            depth: folder_depth,
        });
    }

    /// Finalize: sort by size descending, then path ascending for stability.
    /// Returns the sorted Vec and total files seen.
    pub fn finalize(self) -> (Vec<FolderUsage>, u64) {
        let mut folders: Vec<FolderUsage> = self.folder_map.into_values().collect();
        folders.sort_by(|a, b| b.size.cmp(&a.size).then(a.path.cmp(&b.path)));
        (folders, self.total_files)
    }

    #[cfg(test)]
    pub fn folder_count(&self) -> usize {
        self.folder_map.len()
    }
}

/// Groups file paths by their exact size (Stage 1 of duplicate detection).
///
/// Pure state — no filesystem access.
pub struct SizeGrouping {
    /// Size → list of file paths with that exact size.
    size_map: HashMap<u64, Vec<String>>,
    /// Total files processed.
    total_files: u64,
}

impl SizeGrouping {
    pub fn new() -> Self {
        Self {
            size_map: HashMap::new(),
            total_files: 0,
        }
    }

    /// Record a file path and its size.
    pub fn record(&mut self, path: String, size: u64) {
        self.total_files += 1;
        self.size_map.entry(size).or_default().push(path);
    }

    /// Extract groups where more than one file shares the same size.
    /// These are the candidates for Stage 2 (prefix hash).
    pub fn into_candidates(self) -> (Vec<(u64, Vec<String>)>, u64) {
        let candidates: Vec<(u64, Vec<String>)> = self
            .size_map
            .into_iter()
            .filter(|(_, paths)| paths.len() > 1)
            .collect();
        (candidates, self.total_files)
    }

    /// Count the total number of files in candidate groups (for progress reporting).
    pub fn candidate_file_count(candidates: &[(u64, Vec<String>)]) -> usize {
        candidates.iter().map(|(_, p)| p.len()).sum()
    }
}

/// Groups file paths by a hash value (Stages 2 and 3 of duplicate detection).
///
/// Pure state — hashing is done externally, only the hash string is passed in.
pub struct HashGrouping {
    /// Hash → list of file paths with that hash.
    hash_map: HashMap<String, Vec<String>>,
}

impl HashGrouping {
    pub fn new() -> Self {
        Self {
            hash_map: HashMap::new(),
        }
    }

    /// Record a file path under the given hash.
    pub fn record(&mut self, hash: String, path: String) {
        self.hash_map.entry(hash).or_default().push(path);
    }

    /// Extract groups where more than one file shares the same hash.
    pub fn into_matches(self) -> Vec<(String, Vec<String>)> {
        self.hash_map
            .into_iter()
            .filter(|(_, paths)| paths.len() > 1)
            .collect()
    }

    /// Count the total number of files in matching groups (for progress reporting).
    pub fn match_file_count(matches: &[(String, Vec<String>)]) -> usize {
        matches.iter().map(|(_, p)| p.len()).sum()
    }
}

/// Calculates wasted space for a duplicate group.
pub fn calculate_wasted_space(size_each: u64, file_count: usize) -> u64 {
    if file_count <= 1 {
        0
    } else {
        size_each * (file_count as u64 - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // ─── FolderUsageAccumulator ───

    #[test]
    fn seeds_root_folder_on_creation() {
        let acc = FolderUsageAccumulator::new("/root", 2);
        assert_eq!(acc.folder_count(), 1);
    }

    #[test]
    fn records_file_and_attributes_to_ancestors() {
        let temp = TempDir::new().unwrap();
        let base = temp.path();

        // Create structure: base/a/file.txt (100 bytes) — depth 1 from root
        let subdir = base.join("a");
        fs::create_dir_all(&subdir).unwrap();
        let file_path = subdir.join("file.txt");
        fs::write(&file_path, &[0u8; 100]).unwrap();

        // max_depth=1 is enough to reach root from depth 1 (parent + 1 ancestor)
        let mut acc = FolderUsageAccumulator::new(base.to_str().unwrap(), 1);
        acc.record_file(&file_path, 100);

        let (folders, total_files) = acc.finalize();
        assert_eq!(total_files, 1);

        // File attributed to: base/a (parent) and base (root) = 2 folders
        let sizes: Vec<(String, u64)> = folders.into_iter().map(|f| (f.path, f.size)).collect();

        // Both ancestors should have size 100
        for (path, size) in &sizes {
            assert_eq!(*size, 100, "Folder {} should have size 100", path);
        }
        assert_eq!(sizes.len(), 2, "Expected 2 ancestor folders");
    }

    #[test]
    fn respects_max_depth() {
        let temp = TempDir::new().unwrap();
        let base = temp.path();

        // File directly in base — depth 0
        let file_path = base.join("file.txt");
        fs::write(&file_path, &[0u8; 50]).unwrap();

        // max_depth=0 → file size attributed only to immediate parent (base)
        let mut acc = FolderUsageAccumulator::new(base.to_str().unwrap(), 0);
        acc.record_file(&file_path, 50);

        let (folders, _) = acc.finalize();

        // Root folder should have the file size (it IS the immediate parent)
        let base_normalized = normalize_path(base);
        let root = folders.iter().find(|f| f.path == base_normalized);
        assert!(root.is_some(), "Root folder should exist");
        assert_eq!(root.unwrap().size, 50, "Root should have file size");

        // Only 1 folder (root) since file is directly in base
        assert_eq!(folders.len(), 1);
    }

    #[test]
    fn records_directory() {
        let temp = TempDir::new().unwrap();
        let base = temp.path();
        let dir_path = base.join("new_dir");
        fs::create_dir(&dir_path).unwrap();

        let mut acc = FolderUsageAccumulator::new(base.to_str().unwrap(), 2);
        acc.record_directory(&dir_path);

        let (folders, _) = acc.finalize();
        let dir_normalized = normalize_path(&dir_path);
        let dir_entry = folders.iter().find(|f| f.path == dir_normalized);
        assert!(dir_entry.is_some());
        assert_eq!(dir_entry.unwrap().folder_count, 1);
    }

 
    #[test]
    fn sorts_by_size_descending() {
        let mut acc = FolderUsageAccumulator::new("/root", 2);
        // Manually inject for simplicity
        acc.folder_map.insert(
            "/small".into(),
            FolderUsage {
                path: "/small".into(),
                size: 10,
                file_count: 1,
                folder_count: 0,
                depth: 1,
            },
        );
        acc.folder_map.insert(
            "/large".into(),
            FolderUsage {
                path: "/large".into(),
                size: 1000,
                file_count: 5,
                folder_count: 0,
                depth: 1,
            },
        );

        let (folders, _) = acc.finalize();
        assert!(folders[0].size >= folders[1].size);
    }

    // ─── SizeGrouping ───

    #[test]
    fn groups_files_by_size() {
        let mut grouping = SizeGrouping::new();
        grouping.record("/a.txt".into(), 100);
        grouping.record("/b.txt".into(), 100);
        grouping.record("/c.txt".into(), 200);

        let (candidates, total) = grouping.into_candidates();
        assert_eq!(total, 3);
        assert_eq!(candidates.len(), 1); // Only size 100 has duplicates
        assert_eq!(candidates[0].1.len(), 2);
    }

    #[test]
    fn no_candidates_when_all_unique_sizes() {
        let mut grouping = SizeGrouping::new();
        grouping.record("/a.txt".into(), 100);
        grouping.record("/b.txt".into(), 200);
        grouping.record("/c.txt".into(), 300);

        let (candidates, _) = grouping.into_candidates();
        assert!(candidates.is_empty());
    }

    #[test]
    fn candidate_file_count() {
        let candidates = vec![
            (100, vec!["/a".into(), "/b".into()]),
            (200, vec!["/c".into(), "/d".into(), "/e".into()]),
        ];
        assert_eq!(SizeGrouping::candidate_file_count(&candidates), 5);
    }

    // ─── HashGrouping ───

    #[test]
    fn groups_files_by_hash() {
        let mut grouping = HashGrouping::new();
        grouping.record("abc123".into(), "/a.txt".into());
        grouping.record("abc123".into(), "/b.txt".into());
        grouping.record("def456".into(), "/c.txt".into());

        let matches = grouping.into_matches();
        assert_eq!(matches.len(), 1); // Only abc123 has duplicates
        assert_eq!(matches[0].1.len(), 2);
    }

    #[test]
    fn no_matches_when_all_unique_hashes() {
        let mut grouping = HashGrouping::new();
        grouping.record("hash1".into(), "/a.txt".into());
        grouping.record("hash2".into(), "/b.txt".into());

        let matches = grouping.into_matches();
        assert!(matches.is_empty());
    }

    // ─── calculate_wasted_space ───

    #[test]
    fn wasted_space_for_two_files() {
        assert_eq!(calculate_wasted_space(1000, 2), 1000);
    }

    #[test]
    fn wasted_space_for_five_files() {
        assert_eq!(calculate_wasted_space(1000, 5), 4000);
    }

    #[test]
    fn wasted_space_for_single_file_is_zero() {
        assert_eq!(calculate_wasted_space(1000, 1), 0);
    }
}
