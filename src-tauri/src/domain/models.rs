use serde::{Deserialize, Serialize};

/// A single filesystem entry (file, folder, symlink, or drive).
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub modified: String,
    pub entry_type: EntryType,
    pub extension: Option<String>,
}

/// Classification of a filesystem entry.
/// Serde renames ensure the frontend receives "Folder", "File" etc.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub enum EntryType {
    #[serde(rename = "Folder")]
    Folder,
    #[serde(rename = "Drive")]
    Drive,
    #[serde(rename = "Symlink")]
    Symlink,
    #[serde(rename = "File")]
    File,
}

impl EntryType {
    /// Sort priority: folders first, then drives, symlinks, files.
    pub fn sort_order(&self) -> u8 {
        match self {
            EntryType::Folder => 0,
            EntryType::Drive => 1,
            EntryType::Symlink => 2,
            EntryType::File => 3,
        }
    }
}

/// A mounted volume / drive visible to the user.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Volume {
    pub name: String,
    pub path: String,
}

/// Sort a list of entries: by type priority, then case-insensitive name.
pub fn sort_entries(entries: &mut [Entry]) {
    entries.sort_by(|a, b| {
        a.entry_type
            .sort_order()
            .cmp(&b.entry_type.sort_order())
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

// ─── Analytics Models ───

/// Result of scanning a single folder for disk usage.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FolderUsage {
    pub path: String,
    pub size: u64,
    pub file_count: u64,
    pub folder_count: u64,
}

/// A single folder in the directory structure tree.
/// Sent during Phase 1 (structure scan) so the frontend can render the tree immediately.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FolderStructure {
    pub path: String,
    pub name: String,
    /// Direct child folder paths (immediate children only).
    pub children: Vec<String>,
}

/// A single child folder in a tree children response.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScanTreeChild {
    pub path: String,
    pub name: String,
}

/// Emitted when a scan begins, telling the frontend the root folder.
/// Frontend renders the root row and calls `get_scan_tree_children` on expand.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScanTreeStarted {
    pub scan_id: String,
    pub root_path: String,
    pub root_name: String,
}

/// Emitted when children for a folder have been discovered.
/// Frontend enables the expand button and stores the children count.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScanTreeChildren {
    pub scan_id: String,
    pub parent_path: String,
    pub children: Vec<ScanTreeChild>,
}

/// A group of files confirmed to be identical.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateGroup {
    pub hash: String,
    pub size_each: u64,
    pub files: Vec<String>,
    pub wasted_space: u64, // size_each * (files.len() - 1)
}

/// A point-in-time snapshot of disk usage, stored in SQLite.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UsageSnapshot {
    pub id: i64,
    pub path: String,
    pub total_size: u64,
    pub file_count: u64,
    pub folder_count: u64,
    pub top_folders: String,       // JSON array of FolderUsage
    pub scanned_at: String,        // ISO 8601 UTC
}

/// Progress update emitted during a long-running scan.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub scan_id: String,
    pub percentage: f64,
    pub message: String,
}

/// A chunk of results emitted incrementally during a scan.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScanChunk {
    pub scan_id: String,
    pub data: ScanChunkData,
}

/// The payload inside a ScanChunk — varies by scan type.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ScanChunkData {
    /// One folder finished during usage scan.
    FolderUsage { usage: FolderUsage },
    /// One large file found.
    LargeFile { entry: Entry },
    /// One duplicate group confirmed.
    DuplicateGroup { group: DuplicateGroup },
}

/// Error emitted when a scan fails.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScanError {
    pub scan_id: String,
    pub message: String,
}

/// Final summary emitted when a scan completes.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScanComplete {
    pub scan_id: String,
    pub total_items: u64,
    pub total_size: u64,
    pub duration_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, entry_type: EntryType, size: u64) -> Entry {
        Entry {
            name: name.to_string(),
            path: format!("/test/{}", name),
            size,
            modified: String::new(),
            entry_type,
            extension: None,
        }
    }

    // ── sort_entries ──

    #[test]
    fn folders_sort_before_files() {
        let mut entries = vec![entry("file.txt", EntryType::File, 100), entry("folder", EntryType::Folder, 0)];
        sort_entries(&mut entries);
        assert_eq!(entries[0].entry_type, EntryType::Folder);
        assert_eq!(entries[1].entry_type, EntryType::File);
    }

    #[test]
    fn sort_order_is_folder_drive_symlink_file() {
        let mut entries = vec![
            entry("z.txt", EntryType::File, 0),
            entry("a.txt", EntryType::Symlink, 0),
            entry("b", EntryType::Drive, 0),
            entry("c", EntryType::Folder, 0),
        ];
        sort_entries(&mut entries);
        assert_eq!(entries[0].entry_type, EntryType::Folder);
        assert_eq!(entries[1].entry_type, EntryType::Drive);
        assert_eq!(entries[2].entry_type, EntryType::Symlink);
        assert_eq!(entries[3].entry_type, EntryType::File);
    }

    #[test]
    fn same_type_sorted_by_name_case_insensitive() {
        let mut entries = vec![
            entry("Zebra", EntryType::File, 0),
            entry("apple", EntryType::File, 0),
            entry("Banana", EntryType::File, 0),
        ];
        sort_entries(&mut entries);
        assert_eq!(entries[0].name, "apple");
        assert_eq!(entries[1].name, "Banana");
        assert_eq!(entries[2].name, "Zebra");
    }

    #[test]
    fn type_priority_overrides_name() {
        let mut entries = vec![entry("aaa", EntryType::File, 0), entry("zzz", EntryType::Folder, 0)];
        sort_entries(&mut entries);
        assert_eq!(entries[0].name, "zzz"); // Folder comes first despite name
        assert_eq!(entries[1].name, "aaa");
    }

    // ── DuplicateGroup ──

    #[test]
    fn wasted_space_calculated_correctly() {
        let group = DuplicateGroup {
            hash: "abc123".to_string(),
            size_each: 1000,
            files: vec!["/a".into(), "/b".into(), "/c".into()],
            wasted_space: 2000, // 1000 * (3 - 1)
        };
        assert_eq!(group.wasted_space, group.size_each * (group.files.len() as u64 - 1));
    }

    // ── Serialization ──

    #[test]
    fn entry_serializes_to_frontend_format() {
        let e = entry("test.txt", EntryType::File, 42);
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"entryType\":\"File\""));
        assert!(json.contains("\"name\":\"test.txt\""));
    }

    #[test]
    fn folder_usage_serializes() {
        let usage = FolderUsage {
            path: "/test".into(),
            size: 1024,
            file_count: 5,
            folder_count: 2,
        };
        let json = serde_json::to_string(&usage).unwrap();
        assert!(json.contains("\"path\":\"/test\""));
        assert!(json.contains("\"size\":1024"));
        assert!(json.contains("\"fileCount\":5"));
        assert!(json.contains("\"folderCount\":2"));
    }

    #[test]
    fn scan_chunk_data_serializes_with_tag() {
        let data = ScanChunkData::FolderUsage {
            usage: FolderUsage {
                path: "/test".into(),
                size: 100,
                file_count: 1,
                folder_count: 0,
            },
        };
        let json = serde_json::to_string(&data).unwrap();
        assert!(json.contains("\"type\":\"folder_usage\""));
    }
}
