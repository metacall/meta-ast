use std::sync::atomic::{AtomicU32, Ordering};

use serde::{Deserialize, Serialize};

macro_rules! define_id_type {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub u32);

        impl $name {
            pub fn to_raw(self) -> u32 {
                self.0
            }
        }
    };
}

define_id_type!(FileId);
define_id_type!(SymbolId);
define_id_type!(SnapshotId);

#[derive(Debug)]
pub struct IdGenerator<T> {
    counter: AtomicU32,
    _marker: std::marker::PhantomData<T>,
}

impl<T> IdGenerator<T> {
    pub fn new() -> Self {
        Self {
            counter: AtomicU32::new(0),
            _marker: std::marker::PhantomData,
        }
    }

    pub fn next(&self) -> T
    where
        T: From<u32>,
    {
        let val = self.counter.fetch_add(1, Ordering::Relaxed);
        T::from(val)
    }
}

impl<T> Default for IdGenerator<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl From<u32> for FileId {
    fn from(val: u32) -> Self {
        Self(val)
    }
}

impl From<u32> for SymbolId {
    fn from(val: u32) -> Self {
        Self(val)
    }
}

impl From<u32> for SnapshotId {
    fn from(val: u32) -> Self {
        Self(val)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Arc;

    #[test]
    fn file_id_sequential() {
        let idgen = IdGenerator::<FileId>::new();
        assert_eq!(idgen.next(), FileId(0));
        assert_eq!(idgen.next(), FileId(1));
        assert_eq!(idgen.next(), FileId(2));
    }

    #[test]
    fn symbol_id_sequential() {
        let idgen = IdGenerator::<SymbolId>::new();
        assert_eq!(idgen.next(), SymbolId(0));
        assert_eq!(idgen.next(), SymbolId(1));
        assert_eq!(idgen.next(), SymbolId(2));
    }

    #[test]
    fn snapshot_id_sequential() {
        let idgen = IdGenerator::<SnapshotId>::new();
        assert_eq!(idgen.next(), SnapshotId(0));
        assert_eq!(idgen.next(), SnapshotId(1));
        assert_eq!(idgen.next(), SnapshotId(2));
    }

    #[test]
    fn id_generator_thread_safe() {
        let idgen = Arc::new(IdGenerator::<SymbolId>::new());
        let mut handles = Vec::new();

        for _ in 0..4 {
            let idgen = Arc::clone(&idgen);
            handles.push(std::thread::spawn(move || {
                let mut ids = Vec::with_capacity(250);
                for _ in 0..250 {
                    ids.push(idgen.next());
                }
                ids
            }));
        }

        let all_ids: Vec<SymbolId> = handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect();

        let unique: HashSet<SymbolId> = all_ids.iter().copied().collect();
        assert_eq!(unique.len(), 1000, "expected 1000 unique IDs");
        assert!(
            unique.iter().all(|id| id.to_raw() < 1000),
            "all IDs must be in range 0..1000"
        );
    }

    #[test]
    fn file_id_serde_roundtrip() {
        let original = FileId(42);
        let json = serde_json::to_string(&original).unwrap();
        let roundtrip: FileId = serde_json::from_str(&json).unwrap();
        assert_eq!(original, roundtrip);
    }

    #[test]
    fn symbol_id_serde_roundtrip() {
        let original = SymbolId(99);
        let json = serde_json::to_string(&original).unwrap();
        let roundtrip: SymbolId = serde_json::from_str(&json).unwrap();
        assert_eq!(original, roundtrip);
    }

    #[test]
    fn file_id_to_raw() {
        assert_eq!(FileId(7).to_raw(), 7);
    }

    #[test]
    fn id_generator_default() {
        let idgen = IdGenerator::<FileId>::default();
        assert_eq!(idgen.next(), FileId(0));
    }
}
