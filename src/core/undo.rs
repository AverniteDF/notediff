//! Bounded in-memory undo for accepted hunks.
//!
//! The stack stores full prior snapshots, including the baseline and any
//! associated session metadata. Entries are bounded by both count and total
//! bytes so a large document cannot retain gigabytes of undo history. Undo
//! restores an exact snapshot without replaying edits.

use std::collections::VecDeque;

#[derive(Debug)]
pub struct UndoStack<T = String> {
    entries: VecDeque<T>,
    limit: usize,
    max_bytes: usize,
    bytes: usize,
    size: fn(&T) -> usize,
}

impl<T> UndoStack<T> {
    /// A stack limited to `limit` entries and `max_bytes` of retained data.
    /// The newest snapshot is always kept, even when it alone exceeds the
    /// byte budget.
    pub fn new(limit: usize, max_bytes: usize, size: fn(&T) -> usize) -> Self {
        Self {
            entries: VecDeque::new(),
            limit: limit.max(1),
            max_bytes,
            bytes: 0,
            size,
        }
    }

    pub fn push(&mut self, snapshot: T) {
        let size = (self.size)(&snapshot);
        while !self.entries.is_empty()
            && (self.entries.len() >= self.limit
                || self.bytes.saturating_add(size) > self.max_bytes)
        {
            if let Some(old) = self.entries.pop_front() {
                self.bytes = self.bytes.saturating_sub((self.size)(&old));
            }
        }
        self.entries.push_back(snapshot);
        self.bytes = self.bytes.saturating_add(size);
    }

    pub fn peek(&self) -> Option<&T> {
        self.entries.back()
    }

    pub fn pop(&mut self) -> Option<T> {
        let snapshot = self.entries.pop_back()?;
        self.bytes = self.bytes.saturating_sub((self.size)(&snapshot));
        Some(snapshot)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_is_lifo() {
        let mut stack = UndoStack::<String>::new(3, usize::MAX, |value| value.len());
        stack.push("one".into());
        stack.push("two".into());
        assert_eq!(stack.peek().map(String::as_str), Some("two"));
        assert_eq!(stack.pop().as_deref(), Some("two"));
        assert_eq!(stack.pop().as_deref(), Some("one"));
        assert_eq!(stack.pop(), None);
    }

    #[test]
    fn undo_is_bounded_by_count() {
        let mut stack = UndoStack::<String>::new(2, usize::MAX, |value| value.len());
        stack.push("one".into());
        stack.push("two".into());
        stack.push("three".into());
        assert_eq!(stack.len(), 2);
        assert_eq!(stack.pop().as_deref(), Some("three"));
        assert_eq!(stack.pop().as_deref(), Some("two"));
    }

    #[test]
    fn undo_is_bounded_by_total_bytes() {
        let mut stack = UndoStack::<String>::new(10, 5, |value| value.len());
        stack.push("aa".into());
        stack.push("bb".into());
        stack.push("ccc".into());
        assert_eq!(stack.len(), 2);
        assert_eq!(stack.pop().as_deref(), Some("ccc"));
        assert_eq!(stack.pop().as_deref(), Some("bb"));
    }

    #[test]
    fn an_oversized_snapshot_is_kept_as_the_only_entry() {
        let mut stack = UndoStack::<String>::new(10, 2, |value| value.len());
        stack.push("long".into());
        assert_eq!(stack.len(), 1);
        assert_eq!(stack.pop().as_deref(), Some("long"));
    }

    #[test]
    fn popping_frees_byte_budget_for_the_next_entry() {
        let mut stack = UndoStack::<String>::new(10, 4, |value| value.len());
        stack.push("aa".into());
        stack.push("bb".into());
        assert_eq!(stack.pop().as_deref(), Some("bb"));
        stack.push("cc".into());
        assert_eq!(stack.len(), 2);
        assert_eq!(stack.pop().as_deref(), Some("cc"));
        assert_eq!(stack.pop().as_deref(), Some("aa"));
    }
}
