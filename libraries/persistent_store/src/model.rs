// Copyright 2019-2020 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::format::Format;
use crate::{StoreError, StoreRatio, StoreResult, StoreUpdate};
use std::collections::{HashMap, HashSet};

/// Models the mutable operations of a store.
///
/// The model doesn't model the storage and read-only operations. This is done by the driver.
#[derive(Clone, Debug)]
pub struct StoreModel {
    /// Represents the content of the store.
    content: HashMap<usize, Box<[u8]>>,

    /// The modeled storage configuration.
    format: Format,
}

/// Mutable operations on a store.
#[derive(Clone, Debug)]
pub enum StoreOperation {
    /// Applies a transaction.
    Transaction {
        /// The list of updates to be applied.
        updates: Vec<StoreUpdate>,
    },

    /// Deletes all keys above a threshold.
    Clear {
        /// The minimum key to be deleted.
        min_key: usize,
    },

    /// Compacts the store until a given capacity is immediately available.
    Prepare {
        /// How much capacity should be immediately available after compaction.
        length: usize,
    },
}

impl StoreModel {
    /// Creates an empty model for a given storage configuration.
    pub fn new(format: Format) -> StoreModel {
        let content = HashMap::new();
        StoreModel { content, format }
    }

    /// Returns the modeled content.
    pub fn content(&self) -> &HashMap<usize, Box<[u8]>> {
        &self.content
    }

    /// Returns the storage configuration.
    pub fn format(&self) -> &Format {
        &self.format
    }

    /// Simulates a store operation.
    pub fn apply(&mut self, operation: StoreOperation) -> StoreResult<()> {
        match operation {
            StoreOperation::Transaction { updates } => self.transaction(updates),
            StoreOperation::Clear { min_key } => self.clear(min_key),
            StoreOperation::Prepare { length } => self.prepare(length),
        }
    }

    /// Returns the capacity according to the model.
    pub fn capacity(&self) -> StoreRatio {
        let total = self.format.total_capacity();
        let used: usize = self.content.values().map(|x| self.entry_size(x)).sum();
        StoreRatio { used, total }
    }

    /// Applies a transaction.
    fn transaction(&mut self, updates: Vec<StoreUpdate>) -> StoreResult<()> {
        // Fail if too many updates.
        if updates.len() > self.format.max_updates() {
            return Err(StoreError::InvalidArgument);
        }
        // Fail if an update is invalid.
        if !updates.iter().all(|x| self.update_valid(x)) {
            return Err(StoreError::InvalidArgument);
        }
        // Fail if updates are not disjoint, i.e. there are duplicate keys.
        let keys: HashSet<_> = updates.iter().map(|x| x.key()).collect();
        if keys.len() != updates.len() {
            return Err(StoreError::InvalidArgument);
        }
        // Fail if there is not enough capacity.
        let capacity = match updates.len() {
            // An empty transaction doesn't consume anything.
            0 => 0,
            // Transactions with a single update are optimized by avoiding a marker entry.
            1 => match &updates[0] {
                StoreUpdate::Insert { value, .. } => self.entry_size(value),
                // Transactions with a single update which is a removal don't consume anything.
                StoreUpdate::Remove { .. } => 0,
            },
            // A transaction consumes one word for the marker entry in addition to its updates.
            _ => 1 + updates.iter().map(|x| self.update_size(x)).sum::<usize>(),
        };
        if self.capacity().remaining() < capacity {
            return Err(StoreError::NoCapacity);
        }
        // Apply the updates.
        for update in updates {
            match update {
                StoreUpdate::Insert { key, value } => {
                    self.content.insert(key, value.into_boxed_slice());
                }
                StoreUpdate::Remove { key } => {
                    self.content.remove(&key);
                }
            }
        }
        Ok(())
    }

    /// Applies a clear operation.
    fn clear(&mut self, min_key: usize) -> StoreResult<()> {
        if min_key > self.format.max_key() {
            return Err(StoreError::InvalidArgument);
        }
        self.content.retain(|&k, _| k < min_key);
        Ok(())
    }

    /// Applies a prepare operation.
    fn prepare(&self, length: usize) -> StoreResult<()> {
        if self.capacity().remaining() < length {
            return Err(StoreError::NoCapacity);
        }
        Ok(())
    }

    /// Returns the word capacity of an update.
    fn update_size(&self, update: &StoreUpdate) -> usize {
        match update {
            StoreUpdate::Insert { value, .. } => self.entry_size(value),
            StoreUpdate::Remove { .. } => 1,
        }
    }

    /// Returns the word capacity of an entry.
    fn entry_size(&self, value: &[u8]) -> usize {
        1 + self.format.bytes_to_words(value.len())
    }

    /// Returns whether an update is valid.
    fn update_valid(&self, update: &StoreUpdate) -> bool {
        update.key() <= self.format.max_key()
            && update
                .value()
                .map_or(true, |x| x.len() <= self.format.max_value_len())
    }
}
