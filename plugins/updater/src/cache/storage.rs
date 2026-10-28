// Copyright 2019-2023 Tauri Programme within The Commons Conservancy
// SPDX-License-Identifier: Apache-2.0
// SPDX-License-Identifier: MIT

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use http_cache::{CacheManager, HttpResponse, Result};
use http_cache_semantics::CachePolicy;

use super::{CACHE_CAPACITY, MAX_BODY_SIZE};

/// In-memory storage for the HTTP cache, keeping the [`CACHE_CAPACITY`] most recently used entries.
/// Responses with a body over [`MAX_BODY_SIZE`] are not stored.
///
/// Entries are stored as-is, without serializing them.
/// Freshness isn't checked here: `http-cache` validates the stored [`CachePolicy`] on every read.
/// Stale entries are kept on purpose, so they can be revalidated, for example with an `ETag`.
///
/// Uses an internal `Arc` so cloning means we use a shared storage.
#[derive(Debug, Default, Clone)]
pub(super) struct MemoryStorage {
    // Ordered from least to most recently used.
    // With this few entries a linear scan is simpler and smaller than a map.
    entries: Arc<Mutex<Vec<Entry>>>,
}

#[derive(Debug)]
struct Entry {
    key: String,
    response: HttpResponse,
    policy: CachePolicy,
}

impl MemoryStorage {
    fn entries(&self) -> MutexGuard<'_, Vec<Entry>> {
        // Entries are only added or removed as a whole, so a panic while holding the lock
        // can't leave a half-written entry behind.
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl CacheManager for MemoryStorage {
    async fn get(&self, cache_key: &str) -> Result<Option<(HttpResponse, CachePolicy)>> {
        let mut entries = self.entries();
        let Some(index) = entries.iter().position(|entry| entry.key == cache_key) else {
            return Ok(None);
        };

        // Move it to the end, as the most recently used.
        let entry = entries.remove(index);
        let found = (entry.response.clone(), entry.policy.clone());
        entries.push(entry);
        Ok(Some(found))
    }

    async fn put(
        &self,
        cache_key: String,
        response: HttpResponse,
        policy: CachePolicy,
    ) -> Result<HttpResponse> {
        let mut entries = self.entries();
        // Also when not storing this response, so an older one isn't served in its place.
        entries.retain(|entry| entry.key != cache_key);
        if response.body.len() > MAX_BODY_SIZE {
            // Declining to store is fine, `http-cache` uses the returned response
            // and the next lookup is a cache miss.
            return Ok(response);
        }
        if entries.len() >= CACHE_CAPACITY {
            // Evict the least recently used.
            entries.remove(0);
        }
        entries.push(Entry {
            key: cache_key,
            response: response.clone(),
            policy,
        });
        Ok(response)
    }

    async fn delete(&self, cache_key: &str) -> Result<()> {
        self.entries().retain(|entry| entry.key != cache_key);
        Ok(())
    }
}
