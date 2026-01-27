use std::collections::VecDeque;
use std::fs;
use std::path::Path;

use crate::clipboard::{ClipboardContent, ClipboardEntry};
use crate::error::Result;

/// Manages clipboard history
#[derive(Debug)]
pub struct ClipboardHistory {
    /// History entries (newest first)
    entries: VecDeque<ClipboardEntry>,

    /// Maximum number of entries to keep
    max_entries: usize,

    /// Next ID to assign
    next_id: u64,

    /// Whether to deduplicate entries
    deduplicate: bool,

    /// Hash set for fast deduplication lookup
    hashes: std::collections::HashSet<String>,
}

impl ClipboardHistory {
    /// Create a new empty history
    pub fn new(max_entries: usize, deduplicate: bool) -> Self {
        Self {
            entries: VecDeque::new(),
            max_entries,
            next_id: 1,
            deduplicate,
            hashes: std::collections::HashSet::new(),
        }
    }

    /// Push new content to history
    /// Returns the entry ID if added, None if deduplicated
    pub fn push(&mut self, content: ClipboardContent, source: Option<String>) -> Option<u64> {
        let hash = content.hash();

        // Check for duplicates
        if self.deduplicate && self.hashes.contains(&hash) {
            // Move existing entry to front
            if let Some(pos) = self.entries.iter().position(|e| e.hash == hash) {
                if pos > 0 {
                    if let Some(mut entry) = self.entries.remove(pos) {
                        entry.timestamp = chrono::Utc::now();
                        let id = entry.id;
                        self.entries.push_front(entry);
                        return Some(id);
                    }
                }
            }
            return None;
        }

        // Create new entry
        let id = self.next_id;
        self.next_id += 1;

        let entry = ClipboardEntry::new(id, content, source);
        self.hashes.insert(hash);
        self.entries.push_front(entry);

        // Enforce max entries (keep pinned entries)
        self.enforce_limit();

        Some(id)
    }

    /// Enforce the max entries limit
    fn enforce_limit(&mut self) {
        while self.entries.len() > self.max_entries {
            // Find the oldest non-pinned entry
            if let Some(pos) = self
                .entries
                .iter()
                .enumerate()
                .rev()
                .find(|(_, e)| !e.pinned)
                .map(|(i, _)| i)
            {
                if let Some(removed) = self.entries.remove(pos) {
                    self.hashes.remove(&removed.hash);
                }
            } else {
                // All entries are pinned, can't remove anything
                break;
            }
        }
    }

    /// Get the current (most recent) entry
    pub fn current(&self) -> Option<&ClipboardEntry> {
        self.entries.front()
    }

    /// Get an entry by ID
    pub fn get(&self, id: u64) -> Option<&ClipboardEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Get mutable entry by ID
    pub fn get_mut(&mut self, id: u64) -> Option<&mut ClipboardEntry> {
        self.entries.iter_mut().find(|e| e.id == id)
    }

    /// List entries (newest first)
    pub fn list(&self, limit: usize) -> Vec<&ClipboardEntry> {
        self.entries.iter().take(limit).collect()
    }

    /// List all entries
    pub fn list_all(&self) -> Vec<&ClipboardEntry> {
        self.entries.iter().collect()
    }

    /// List pinned entries
    pub fn list_pinned(&self) -> Vec<&ClipboardEntry> {
        self.entries.iter().filter(|e| e.pinned).collect()
    }

    /// Remove an entry by ID
    pub fn remove(&mut self, id: u64) -> bool {
        if let Some(pos) = self.entries.iter().position(|e| e.id == id) {
            if let Some(removed) = self.entries.remove(pos) {
                self.hashes.remove(&removed.hash);
                return true;
            }
        }
        false
    }

    /// Clear all non-pinned entries
    pub fn clear(&mut self, keep_pinned: bool) {
        if keep_pinned {
            let pinned: VecDeque<_> = self.entries.drain(..).filter(|e| e.pinned).collect();
            self.hashes.clear();
            for entry in &pinned {
                self.hashes.insert(entry.hash.clone());
            }
            self.entries = pinned;
        } else {
            self.entries.clear();
            self.hashes.clear();
        }
    }

    /// Pin an entry
    pub fn pin(&mut self, id: u64) -> bool {
        if let Some(entry) = self.get_mut(id) {
            entry.pinned = true;
            true
        } else {
            false
        }
    }

    /// Unpin an entry
    pub fn unpin(&mut self, id: u64) -> bool {
        if let Some(entry) = self.get_mut(id) {
            entry.pinned = false;
            true
        } else {
            false
        }
    }

    /// Select an entry (move it to front, making it current)
    pub fn select(&mut self, id: u64) -> bool {
        if let Some(pos) = self.entries.iter().position(|e| e.id == id) {
            if pos > 0 {
                if let Some(entry) = self.entries.remove(pos) {
                    self.entries.push_front(entry);
                }
            }
            true
        } else {
            false
        }
    }

    /// Search entries by text content
    pub fn search(&self, query: &str, limit: usize) -> Vec<&ClipboardEntry> {
        let query_lower = query.to_lowercase();
        self.entries
            .iter()
            .filter(|e| match &e.content {
                ClipboardContent::Text(text) => text.to_lowercase().contains(&query_lower),
                ClipboardContent::Image { .. } => false,
                ClipboardContent::Files { uris, .. } => {
                    // Search by file paths/names
                    uris.iter().any(|uri| uri.to_lowercase().contains(&query_lower))
                }
            })
            .take(limit)
            .collect()
    }

    /// Get the number of entries
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Save history to a file
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let entries: Vec<_> = self.entries.iter().collect();
        let json = serde_json::to_string_pretty(&entries)?;

        // Create parent directories if needed
        if let Some(parent) = path.as_ref().parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(path, json)?;
        Ok(())
    }

    /// Load history from a file
    pub fn load<P: AsRef<Path>>(path: P, max_entries: usize, deduplicate: bool) -> Result<Self> {
        let json = fs::read_to_string(&path)?;
        let entries: Vec<ClipboardEntry> = serde_json::from_str(&json)?;

        let mut history = Self::new(max_entries, deduplicate);

        // Find the highest ID
        let max_id = entries.iter().map(|e| e.id).max().unwrap_or(0);
        history.next_id = max_id + 1;

        // Add entries (in reverse order since they're stored newest-first)
        for entry in entries.into_iter().rev() {
            history.hashes.insert(entry.hash.clone());
            history.entries.push_front(entry);
        }

        // Enforce limit
        history.enforce_limit();

        Ok(history)
    }

    /// Load history or create new if file doesn't exist
    pub fn load_or_new<P: AsRef<Path>>(
        path: P,
        max_entries: usize,
        deduplicate: bool,
    ) -> Result<Self> {
        if path.as_ref().exists() {
            Self::load(path, max_entries, deduplicate)
        } else {
            Ok(Self::new(max_entries, deduplicate))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_and_get() {
        let mut history = ClipboardHistory::new(100, true);
        let id = history.push(ClipboardContent::Text("hello".into()), None);
        assert!(id.is_some());

        let entry = history.get(id.unwrap());
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().content.as_text(), Some("hello"));
    }

    #[test]
    fn test_deduplication() {
        let mut history = ClipboardHistory::new(100, true);

        let id1 = history.push(ClipboardContent::Text("hello".into()), None);
        let id2 = history.push(ClipboardContent::Text("world".into()), None);
        let id3 = history.push(ClipboardContent::Text("hello".into()), None);

        // Third push should return the ID of the first entry (moved to front)
        assert_eq!(id3, id1);
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn test_limit_enforcement() {
        let mut history = ClipboardHistory::new(3, false);

        for i in 0..5 {
            history.push(ClipboardContent::Text(format!("entry {}", i)), None);
        }

        assert_eq!(history.len(), 3);
        // Should have entries 2, 3, 4 (newest)
        let entries = history.list(10);
        assert_eq!(entries[0].content.as_text(), Some("entry 4"));
    }

    #[test]
    fn test_pinned_entries() {
        let mut history = ClipboardHistory::new(3, false);

        let id1 = history.push(ClipboardContent::Text("pinned".into()), None).unwrap();
        history.pin(id1);

        for i in 0..5 {
            history.push(ClipboardContent::Text(format!("entry {}", i)), None);
        }

        // Pinned entry should still exist
        assert!(history.get(id1).is_some());
        assert!(history.get(id1).unwrap().pinned);
    }
}
