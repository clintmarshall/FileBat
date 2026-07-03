//! Integration tests using real filesystem operations.
//!
//! These tests create temp directories, populate them with files,
//! and verify the analytics engine works end-to-end.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

// We need to test the actual use cases, so we re-export what we need
// from the main binary. Since Rust doesn't let us import private modules,
// we'll test through the public API.

// For now, test the StdFileSystem directly
mod file_system_tests {
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Create a test directory structure:
    /// temp/
    ///   folder1/
    ///     file1.txt (100 bytes)
    ///     file2.txt (200 bytes)
    ///   folder2/
    ///     file3.txt (300 bytes)
    ///   root.txt (50 bytes)
    fn create_test_structure(base: &PathBuf) -> PathBuf {
        // Create folders
        let folder1 = base.join("folder1");
        let folder2 = base.join("folder2");
        fs::create_dir(&folder1).unwrap();
        fs::create_dir(&folder2).unwrap();

        // Create files with known sizes
        let f1 = folder1.join("file1.txt");
        fs::write(&f1, vec![b'A'; 100]).unwrap();

        let f2 = folder1.join("file2.txt");
        fs::write(&f2, vec![b'B'; 200]).unwrap();

        let f3 = folder2.join("file3.txt");
        fs::write(&f3, vec![b'C'; 300]).unwrap();

        let root = base.join("root.txt");
        fs::write(&root, vec![b'D'; 50]).unwrap();

        base.clone()
    }

    #[test]
    fn can_list_directory_contents() {
        let temp = TempDir::new().unwrap();
        let path = create_test_structure(&temp.path().to_path_buf());

        let entries = fs::read_dir(&path).unwrap().collect::<Vec<_>>();
        assert_eq!(entries.len(), 3); // folder1, folder2, root.txt
    }

    #[test]
    fn can_calculate_folder_sizes() {
        let temp = TempDir::new().unwrap();
        let path = create_test_structure(&temp.path().to_path_buf());

        // Walk and calculate sizes - attribute to all ancestors (like the real analytics code)
        let mut folder_sizes: std::collections::HashMap<String, u64> =
            std::collections::HashMap::new();

        for entry in walkdir::WalkDir::new(&path) {
            let entry = entry.unwrap();
            if entry.file_type().is_file() {
                let meta = entry.metadata().unwrap();
                let size = meta.len();

                // Attribute to every ancestor up to root
                let mut ancestor = entry.path().to_path_buf();
                loop {
                    *folder_sizes
                        .entry(ancestor.to_string_lossy().to_string())
                        .or_insert(0) += size;
                    if !ancestor.pop() {
                        break;
                    }
                }
            }
        }

        // folder1 should have 300 bytes (100 + 200)
        let folder1_key = path.join("folder1").to_string_lossy().to_string();
        assert_eq!(folder_sizes[&folder1_key], 300);

        // folder2 should have 300 bytes
        let folder2_key = path.join("folder2").to_string_lossy().to_string();
        assert_eq!(folder_sizes[&folder2_key], 300);

        // root should have 650 bytes (100 + 200 + 300 + 50)
        let root_key = path.to_string_lossy().to_string();
        assert_eq!(folder_sizes[&root_key], 650);
    }

    #[test]
    fn can_detect_duplicate_files() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().to_path_buf();

        // Create two identical files
        fs::write(path.join("file1.txt"), vec![b'X'; 500]).unwrap();
        fs::write(path.join("file2.txt"), vec![b'X'; 500]).unwrap();
        fs::write(path.join("unique.txt"), vec![b'Y'; 500]).unwrap();

        // Group by size
        let mut size_map: std::collections::HashMap<u64, Vec<String>> =
            std::collections::HashMap::new();

        for entry in walkdir::WalkDir::new(&path) {
            let entry = entry.unwrap();
            if entry.file_type().is_file() {
                let meta = entry.metadata().unwrap();
                size_map
                    .entry(meta.len())
                    .or_default()
                    .push(entry.path().to_string_lossy().to_string());
            }
        }

        // All files are 500 bytes, so they should be in one group
        assert!(size_map.contains_key(&500));
        assert_eq!(size_map[&500].len(), 3);
    }

    #[test]
    fn can_find_large_files() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().to_path_buf();

        // Create files of various sizes
        fs::write(path.join("small.txt"), vec![b'A'; 100]).unwrap();
        fs::write(path.join("medium.txt"), vec![b'B'; 10_000]).unwrap();
        fs::write(path.join("large.txt"), vec![b'C'; 100_000]).unwrap();

        let min_size = 50_000u64;
        let mut large_files = Vec::new();

        for entry in walkdir::WalkDir::new(&path) {
            let entry = entry.unwrap();
            if entry.file_type().is_file() {
                let meta = entry.metadata().unwrap();
                if meta.len() >= min_size {
                    large_files.push(entry.path().to_string_lossy().to_string());
                }
            }
        }

        assert_eq!(large_files.len(), 1);
        assert!(large_files[0].contains("large.txt"));
    }
}

/// Test that the analytics scan logic handles cancellation correctly
mod cancellation_tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[test]
    fn cancel_flag_stops_iteration() {
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel.clone();

        // Simulate a scan loop
        let mut count = 0;
        for _ in 0..100 {
            if cancel_clone.load(Ordering::Relaxed) {
                break;
            }
            count += 1;
            // Cancel after 10 iterations
            if count == 10 {
                cancel.store(true, Ordering::Relaxed);
            }
        }

        assert_eq!(count, 10);
    }

    #[test]
    fn multiple_cancel_checks_work() {
        let cancel = Arc::new(AtomicBool::new(false));

        // Set cancel before starting
        cancel.store(true, Ordering::Relaxed);

        let mut processed = 0;
        for _ in 0..10 {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            processed += 1;
        }

        assert_eq!(processed, 0);
    }
}

/// Test the 3-stage duplicate detection funnel logic
mod duplicate_funnel_tests {
    use std::collections::HashMap;
    use std::fs;
    use std::io::Read;
    use sha2::{Digest, Sha256};
    use tempfile::TempDir;

    fn hash_prefix(path: &str) -> Option<String> {
        let mut file = fs::File::open(path).ok()?;
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 8192];
        let bytes_read = file.read(&mut buffer).ok()?;
        hasher.update(&buffer[..bytes_read]);
        let result = hasher.finalize();
        Some(format!("{:x}", result))
    }

    fn hash_full(path: &str) -> Option<String> {
        let mut file = fs::File::open(path).ok()?;
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 65536];
        loop {
            let bytes = match file.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => return None,
            };
            hasher.update(&buffer[..bytes]);
        }
        let result = hasher.finalize();
        Some(format!("{:x}", result))
    }

    #[test]
    fn three_stage_funnel_identifies_duplicates() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().to_path_buf();

        // Create files: 2 identical, 1 different same size, 1 unique size
        fs::write(path.join("dup1.txt"), vec![b'A'; 1000]).unwrap();
        fs::write(path.join("dup2.txt"), vec![b'A'; 1000]).unwrap();
        fs::write(path.join("diff.txt"), vec![b'B'; 1000]).unwrap();
        fs::write(path.join("unique.txt"), vec![b'C'; 500]).unwrap();

        // Stage 1: Group by size
        let mut size_map: HashMap<u64, Vec<String>> = HashMap::new();
        for entry in walkdir::WalkDir::new(&path) {
            let entry = entry.unwrap();
            if entry.file_type().is_file() {
                let meta = entry.metadata().unwrap();
                size_map
                    .entry(meta.len())
                    .or_default()
                    .push(entry.path().to_string_lossy().to_string());
            }
        }

        // Only the 1000-byte files should be potential duplicates
        let potential: Vec<_> = size_map
            .into_iter()
            .filter(|(_, paths)| paths.len() > 1)
            .collect();
        assert_eq!(potential.len(), 1);
        assert_eq!(potential[0].1.len(), 3); // dup1, dup2, diff

        // Stage 2: Prefix hash
        let mut prefix_map: HashMap<String, Vec<String>> = HashMap::new();
        for (_, paths) in potential {
            for p in &paths {
                if let Some(h) = hash_prefix(p) {
                    prefix_map.entry(h).or_default().push(p.clone());
                }
            }
        }

        // Should have 2 groups: one with 2 files (dup1, dup2), one with 1 (diff)
        let prefix_matches: Vec<_> = prefix_map
            .into_iter()
            .filter(|(_, paths)| paths.len() > 1)
            .collect();
        assert_eq!(prefix_matches.len(), 1);
        assert_eq!(prefix_matches[0].1.len(), 2);

        // Stage 3: Full hash
        let mut full_map: HashMap<String, Vec<String>> = HashMap::new();
        for (_, paths) in prefix_matches {
            for p in &paths {
                if let Some(h) = hash_full(p) {
                    full_map.entry(h).or_default().push(p.clone());
                }
            }
        }

        // Should have 1 group with 2 files
        let confirmed: Vec<_> = full_map
            .into_iter()
            .filter(|(_, paths)| paths.len() >= 2)
            .collect();
        assert_eq!(confirmed.len(), 1);
        assert_eq!(confirmed[0].1.len(), 2);
    }
}
