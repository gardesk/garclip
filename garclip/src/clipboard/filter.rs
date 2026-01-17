use regex::Regex;

use crate::clipboard::ClipboardContent;
use crate::config::{BehaviorConfig, FilterConfig};

/// Content filter for clipboard entries
pub struct ContentFilter {
    /// Compiled regex patterns to ignore
    ignore_patterns: Vec<Regex>,

    /// Window classes to ignore (lowercase for case-insensitive matching)
    ignore_classes: Vec<String>,

    /// Minimum text length
    min_length: usize,

    /// Maximum text length
    max_length: usize,

    /// Maximum image size
    max_image_size: usize,

    /// Ignore empty content
    ignore_empty: bool,
}

impl ContentFilter {
    /// Create a new content filter from config
    pub fn new(behavior: &BehaviorConfig, filters: &FilterConfig) -> Self {
        let ignore_patterns = filters
            .ignore_patterns
            .iter()
            .filter_map(|p| match Regex::new(p) {
                Ok(re) => Some(re),
                Err(e) => {
                    tracing::warn!("Invalid ignore pattern '{}': {}", p, e);
                    None
                }
            })
            .collect();

        let ignore_classes: Vec<String> = filters
            .ignore_classes
            .iter()
            .map(|c| c.to_lowercase())
            .collect();

        Self {
            ignore_patterns,
            ignore_classes,
            min_length: behavior.min_length,
            max_length: behavior.max_length,
            max_image_size: behavior.max_image_size,
            ignore_empty: behavior.ignore_empty,
        }
    }

    /// Check if content should be filtered (returns true if should be ignored)
    pub fn should_filter(&self, content: &ClipboardContent, source: Option<&str>) -> bool {
        // Check source window class
        if let Some(src) = source {
            let src_lower = src.to_lowercase();
            if self.ignore_classes.iter().any(|c| src_lower.contains(c)) {
                tracing::debug!("Filtering content from ignored class: {}", src);
                return true;
            }
        }

        match content {
            ClipboardContent::Text(text) => self.should_filter_text(text),
            ClipboardContent::Image { data, .. } => self.should_filter_image(data),
        }
    }

    /// Check if text content should be filtered
    fn should_filter_text(&self, text: &str) -> bool {
        // Check empty
        if self.ignore_empty && text.trim().is_empty() {
            tracing::debug!("Filtering empty text");
            return true;
        }

        // Check min length
        if text.len() < self.min_length {
            tracing::debug!("Filtering text below min length: {} < {}", text.len(), self.min_length);
            return true;
        }

        // Check max length
        if text.len() > self.max_length {
            tracing::debug!("Filtering text above max length: {} > {}", text.len(), self.max_length);
            return true;
        }

        // Check ignore patterns
        for pattern in &self.ignore_patterns {
            if pattern.is_match(text) {
                tracing::debug!("Filtering text matching pattern: {}", pattern.as_str());
                return true;
            }
        }

        false
    }

    /// Check if image content should be filtered
    fn should_filter_image(&self, data: &[u8]) -> bool {
        // Check empty
        if self.ignore_empty && data.is_empty() {
            tracing::debug!("Filtering empty image");
            return true;
        }

        // Check max size
        if data.len() > self.max_image_size {
            tracing::debug!("Filtering image above max size: {} > {}", data.len(), self.max_image_size);
            return true;
        }

        false
    }

    /// Reload filter from new config
    pub fn reload(&mut self, behavior: &BehaviorConfig, filters: &FilterConfig) {
        let new_filter = Self::new(behavior, filters);
        *self = new_filter;
        tracing::info!("Content filter reloaded");
    }
}
