/*
 * Copyright (c) 2023 Stalwart Labs Ltd.
 *
 * This file is part of Stalwart Mail Server.
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as
 * published by the Free Software Foundation, either version 3 of
 * the License, or (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 * in the LICENSE file at the top-level directory of this distribution.
 * You should have received a copy of the GNU Affero General Public License
 * along with this program.  If not, see <http://www.gnu.org/licenses/>.
 *
 * You can be released from the requirements of the AGPLv3 license by
 * purchasing a commercial license. Please contact licensing@stalw.art
 * for more details.
*/

use std::{
    borrow::Borrow,
    hash::Hash,
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use utils::config::{utils::AsKey, Config};

pub struct CachedDirectory {
    cached_domains: Mutex<LookupCache<String>>,
    cached_rcpts: Mutex<LookupCache<String>>,
}

#[allow(clippy::type_complexity)]
#[derive(Debug)]
pub struct LookupCache<T: Hash + Eq> {
    cache_pos: lru_cache::LruCache<T, Instant, ahash::RandomState>,
    cache_neg: lru_cache::LruCache<T, Instant, ahash::RandomState>,
    ttl_pos: Duration,
    ttl_neg: Duration,
}

impl CachedDirectory {
    pub fn try_from_config(
        config: &Config,
        prefix: impl AsKey,
    ) -> utils::config::Result<Option<Self>> {
        let prefix = prefix.as_key();
        if let Some(cached_entries) = config.property((&prefix, "cache.entries"))? {
            let cache_ttl_positive = config
                .property((&prefix, "cache.ttl.positive"))?
                .unwrap_or(Duration::from_secs(86400));
            let cache_ttl_negative = config
                .property((&prefix, "cache.ttl.positive"))?
                .unwrap_or_else(|| Duration::from_secs(3600));

            Ok(Some(CachedDirectory {
                cached_domains: Mutex::new(LookupCache::new(
                    cached_entries,
                    cache_ttl_positive,
                    cache_ttl_negative,
                )),
                cached_rcpts: Mutex::new(LookupCache::new(
                    cached_entries,
                    cache_ttl_positive,
                    cache_ttl_negative,
                )),
            }))
        } else {
            Ok(None)
        }
    }

    pub fn get_rcpt(&self, address: &str) -> Option<bool> {
        self.cached_rcpts.lock().get(address)
    }

    pub fn set_rcpt(&self, address: &str, exists: bool) {
        if exists {
            self.cached_rcpts.lock().insert_pos(address.to_string());
        } else {
            self.cached_rcpts.lock().insert_neg(address.to_string());
        }
    }

    pub fn get_domain(&self, domain: &str) -> Option<bool> {
        self.cached_domains.lock().get(domain)
    }

    pub fn set_domain(&self, domain: &str, exists: bool) {
        if exists {
            self.cached_domains.lock().insert_pos(domain.to_string());
        } else {
            self.cached_domains.lock().insert_neg(domain.to_string());
        }
    }
}

impl<T: Hash + Eq> LookupCache<T> {
    pub fn new(capacity: usize, ttl_pos: Duration, ttl_neg: Duration) -> Self {
        Self {
            cache_pos: lru_cache::LruCache::with_hasher(capacity, ahash::RandomState::new()),
            cache_neg: lru_cache::LruCache::with_hasher(capacity, ahash::RandomState::new()),
            ttl_pos,
            ttl_neg,
        }
    }

    pub fn get<Q: ?Sized>(&mut self, name: &Q) -> Option<bool>
    where
        T: Borrow<Q>,
        Q: Hash + Eq,
    {
        // Check positive cache
        if let Some(valid_until) = self.cache_pos.get_mut(name) {
            if *valid_until >= Instant::now() {
                return Some(true);
            } else {
                self.cache_pos.remove(name);
            }
        }

        // Check negative cache
        let valid_until = self.cache_neg.get_mut(name)?;
        if *valid_until >= Instant::now() {
            Some(false)
        } else {
            self.cache_pos.remove(name);
            None
        }
    }

    pub fn insert_pos(&mut self, item: T) {
        self.cache_pos.insert(item, Instant::now() + self.ttl_pos);
    }

    pub fn insert_neg(&mut self, item: T) {
        self.cache_neg.insert(item, Instant::now() + self.ttl_neg);
    }

    pub fn clear(&mut self) {
        self.cache_pos.clear();
        self.cache_neg.clear();
    }
}
