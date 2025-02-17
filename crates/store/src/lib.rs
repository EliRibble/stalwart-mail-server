/*
 * Copyright (c) 2023 Stalwart Labs Ltd.
 *
 * This file is part of the Stalwart Mail Server.
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

use std::{borrow::Cow, fmt::Display, sync::Arc};

pub mod backend;
pub mod config;
pub mod dispatch;
pub mod fts;
pub mod query;
pub mod write;

pub use ahash;
use ahash::AHashMap;
use backend::{fs::FsStore, memory::MemoryStore};
pub use blake3;
pub use parking_lot;
pub use rand;
pub use roaring;
use write::{BitmapClass, ValueClass};

#[cfg(feature = "s3")]
use backend::s3::S3Store;

#[cfg(feature = "postgres")]
use backend::postgres::PostgresStore;

#[cfg(feature = "mysql")]
use backend::mysql::MysqlStore;

#[cfg(feature = "sqlite")]
use backend::sqlite::SqliteStore;

#[cfg(feature = "foundation")]
use backend::foundationdb::FdbStore;

#[cfg(feature = "rocks")]
use backend::rocksdb::RocksDbStore;

#[cfg(feature = "elastic")]
use backend::elastic::ElasticSearchStore;

#[cfg(feature = "redis")]
use backend::redis::RedisStore;

pub trait Deserialize: Sized + Sync + Send {
    fn deserialize(bytes: &[u8]) -> crate::Result<Self>;
}

pub trait Serialize {
    fn serialize(self) -> Vec<u8>;
}

// Key serialization flags
pub(crate) const WITH_SUBSPACE: u32 = 1;
pub(crate) const WITHOUT_BLOCK_NUM: u32 = 1 << 1;

pub trait Key: Sync + Send {
    fn serialize(&self, flags: u32) -> Vec<u8>;
    fn subspace(&self) -> u8;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BitmapKey<T: AsRef<BitmapClass>> {
    pub account_id: u32,
    pub collection: u8,
    pub class: T,
    pub block_num: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IndexKey<T: AsRef<[u8]>> {
    pub account_id: u32,
    pub collection: u8,
    pub document_id: u32,
    pub field: u8,
    pub key: T,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IndexKeyPrefix {
    pub account_id: u32,
    pub collection: u8,
    pub field: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ValueKey<T: AsRef<ValueClass>> {
    pub account_id: u32,
    pub collection: u8,
    pub document_id: u32,
    pub class: T,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LogKey {
    pub account_id: u32,
    pub collection: u8,
    pub change_id: u64,
}

pub const BLOB_HASH_LEN: usize = 32;
pub const U64_LEN: usize = std::mem::size_of::<u64>();
pub const U32_LEN: usize = std::mem::size_of::<u32>();

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct BlobHash([u8; BLOB_HASH_LEN]);

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum BlobClass {
    Reserved {
        account_id: u32,
        expires: u64,
    },
    Linked {
        account_id: u32,
        collection: u8,
        document_id: u32,
    },
}

impl Default for BlobClass {
    fn default() -> Self {
        BlobClass::Reserved {
            account_id: 0,
            expires: 0,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    InternalError(String),
    AssertValueFailed,
}

impl std::error::Error for Error {}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::InternalError(msg) => write!(f, "Internal Error: {}", msg),
            Error::AssertValueFailed => write!(f, "Transaction failed: Hash mismatch"),
        }
    }
}

impl From<String> for Error {
    fn from(msg: String) -> Self {
        Error::InternalError(msg)
    }
}

pub const SUBSPACE_BITMAPS: u8 = b'b';
pub const SUBSPACE_VALUES: u8 = b'v';
pub const SUBSPACE_LOGS: u8 = b'l';
pub const SUBSPACE_INDEXES: u8 = b'i';
pub const SUBSPACE_BLOBS: u8 = b't';
pub const SUBSPACE_COUNTERS: u8 = b'c';

pub struct IterateParams<T: Key> {
    begin: T,
    end: T,
    first: bool,
    ascending: bool,
    values: bool,
}

#[derive(Clone, Default)]
pub struct Stores {
    pub stores: AHashMap<String, Store>,
    pub blob_stores: AHashMap<String, BlobStore>,
    pub fts_stores: AHashMap<String, FtsStore>,
    pub lookup_stores: AHashMap<String, LookupStore>,
}

#[derive(Clone)]
pub enum Store {
    #[cfg(feature = "sqlite")]
    SQLite(Arc<SqliteStore>),
    #[cfg(feature = "foundation")]
    FoundationDb(Arc<FdbStore>),
    #[cfg(feature = "postgres")]
    PostgreSQL(Arc<PostgresStore>),
    #[cfg(feature = "mysql")]
    MySQL(Arc<MysqlStore>),
    #[cfg(feature = "rocks")]
    RocksDb(Arc<RocksDbStore>),
}

#[derive(Clone)]
pub enum BlobStore {
    Store(Store),
    Fs(Arc<FsStore>),
    #[cfg(feature = "s3")]
    S3(Arc<S3Store>),
}

#[derive(Clone)]
pub enum FtsStore {
    Store(Store),
    #[cfg(feature = "elastic")]
    ElasticSearch(Arc<ElasticSearchStore>),
}

#[derive(Clone)]
pub enum LookupStore {
    Store(Store),
    Query(Arc<QueryStore>),
    Memory(Arc<MemoryStore>),
    #[cfg(feature = "redis")]
    Redis(Arc<RedisStore>),
}

pub struct QueryStore {
    pub store: LookupStore,
    pub query: String,
}

#[cfg(feature = "sqlite")]
impl From<SqliteStore> for Store {
    fn from(store: SqliteStore) -> Self {
        Self::SQLite(Arc::new(store))
    }
}

#[cfg(feature = "foundation")]
impl From<FdbStore> for Store {
    fn from(store: FdbStore) -> Self {
        Self::FoundationDb(Arc::new(store))
    }
}

#[cfg(feature = "postgres")]
impl From<PostgresStore> for Store {
    fn from(store: PostgresStore) -> Self {
        Self::PostgreSQL(Arc::new(store))
    }
}

#[cfg(feature = "mysql")]
impl From<MysqlStore> for Store {
    fn from(store: MysqlStore) -> Self {
        Self::MySQL(Arc::new(store))
    }
}

#[cfg(feature = "rocks")]
impl From<RocksDbStore> for Store {
    fn from(store: RocksDbStore) -> Self {
        Self::RocksDb(Arc::new(store))
    }
}

impl From<FsStore> for BlobStore {
    fn from(store: FsStore) -> Self {
        Self::Fs(Arc::new(store))
    }
}

#[cfg(feature = "s3")]
impl From<S3Store> for BlobStore {
    fn from(store: S3Store) -> Self {
        Self::S3(Arc::new(store))
    }
}

#[cfg(feature = "elastic")]
impl From<ElasticSearchStore> for FtsStore {
    fn from(store: ElasticSearchStore) -> Self {
        Self::ElasticSearch(Arc::new(store))
    }
}

#[cfg(feature = "redis")]
impl From<RedisStore> for LookupStore {
    fn from(store: RedisStore) -> Self {
        Self::Redis(Arc::new(store))
    }
}

impl From<Store> for FtsStore {
    fn from(store: Store) -> Self {
        Self::Store(store)
    }
}

impl From<Store> for BlobStore {
    fn from(store: Store) -> Self {
        Self::Store(store)
    }
}

impl From<Store> for LookupStore {
    fn from(store: Store) -> Self {
        Self::Store(store)
    }
}

impl From<MemoryStore> for LookupStore {
    fn from(store: MemoryStore) -> Self {
        Self::Memory(Arc::new(store))
    }
}

#[derive(Clone, Debug)]
pub enum LookupKey {
    Key(Vec<u8>),
    Counter(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LookupValue<T> {
    Value { value: T, expires: u64 },
    Counter { num: i64 },
    None,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value<'x> {
    Integer(i64),
    Bool(bool),
    Float(f64),
    Text(Cow<'x, str>),
    Blob(Cow<'x, [u8]>),
    Null,
}

impl Eq for Value<'_> {}

impl<'x> Value<'x> {
    pub fn to_str<'y: 'x>(&'y self) -> Cow<'x, str> {
        match self {
            Value::Text(s) => s.as_ref().into(),
            Value::Integer(i) => Cow::Owned(i.to_string()),
            Value::Bool(b) => Cow::Owned(b.to_string()),
            Value::Float(f) => Cow::Owned(f.to_string()),
            Value::Blob(b) => String::from_utf8_lossy(b.as_ref()),
            Value::Null => Cow::Borrowed(""),
        }
    }
}

impl From<LookupKey> for String {
    fn from(value: LookupKey) -> Self {
        let key = match value {
            LookupKey::Key(key) | LookupKey::Counter(key) => key,
        };
        String::from_utf8(key)
            .unwrap_or_else(|err| String::from_utf8_lossy(&err.into_bytes()).into_owned())
    }
}

#[derive(Clone, Debug)]
pub struct Row {
    pub values: Vec<Value<'static>>,
}

#[derive(Clone, Debug)]
pub struct Rows {
    pub rows: Vec<Row>,
}

#[derive(Clone, Debug)]
pub struct NamedRows {
    pub names: Vec<String>,
    pub rows: Vec<Row>,
}

#[derive(Clone, Copy)]
pub enum QueryType {
    Execute,
    Exists,
    QueryAll,
    QueryOne,
}

pub trait QueryResult: Sync + Send + 'static {
    fn from_exec(items: usize) -> Self;
    fn from_exists(exists: bool) -> Self;
    fn from_query_one(items: impl IntoRows) -> Self;
    fn from_query_all(items: impl IntoRows) -> Self;

    fn query_type() -> QueryType;
}

pub trait IntoRows {
    fn into_row(self) -> Option<Row>;
    fn into_rows(self) -> Rows;
    fn into_named_rows(self) -> NamedRows;
}

impl QueryResult for Option<Row> {
    fn query_type() -> QueryType {
        QueryType::QueryOne
    }

    fn from_exec(_: usize) -> Self {
        unreachable!()
    }

    fn from_exists(_: bool) -> Self {
        unreachable!()
    }

    fn from_query_all(_: impl IntoRows) -> Self {
        unreachable!()
    }

    fn from_query_one(items: impl IntoRows) -> Self {
        items.into_row()
    }
}

impl QueryResult for Rows {
    fn query_type() -> QueryType {
        QueryType::QueryAll
    }

    fn from_exec(_: usize) -> Self {
        unreachable!()
    }

    fn from_exists(_: bool) -> Self {
        unreachable!()
    }

    fn from_query_all(items: impl IntoRows) -> Self {
        items.into_rows()
    }

    fn from_query_one(_: impl IntoRows) -> Self {
        unreachable!()
    }
}

impl QueryResult for NamedRows {
    fn query_type() -> QueryType {
        QueryType::QueryAll
    }

    fn from_exec(_: usize) -> Self {
        unreachable!()
    }

    fn from_exists(_: bool) -> Self {
        unreachable!()
    }

    fn from_query_all(items: impl IntoRows) -> Self {
        items.into_named_rows()
    }

    fn from_query_one(_: impl IntoRows) -> Self {
        unreachable!()
    }
}

impl QueryResult for bool {
    fn query_type() -> QueryType {
        QueryType::Exists
    }

    fn from_exec(_: usize) -> Self {
        unreachable!()
    }

    fn from_exists(exists: bool) -> Self {
        exists
    }

    fn from_query_all(_: impl IntoRows) -> Self {
        unreachable!()
    }

    fn from_query_one(_: impl IntoRows) -> Self {
        unreachable!()
    }
}

impl QueryResult for usize {
    fn query_type() -> QueryType {
        QueryType::Execute
    }

    fn from_exec(items: usize) -> Self {
        items
    }

    fn from_exists(_: bool) -> Self {
        unreachable!()
    }

    fn from_query_all(_: impl IntoRows) -> Self {
        unreachable!()
    }

    fn from_query_one(_: impl IntoRows) -> Self {
        unreachable!()
    }
}

impl<'x> From<&'x str> for Value<'x> {
    fn from(value: &'x str) -> Self {
        Self::Text(value.into())
    }
}

impl<'x> From<String> for Value<'x> {
    fn from(value: String) -> Self {
        Self::Text(value.into())
    }
}

impl<'x> From<&'x String> for Value<'x> {
    fn from(value: &'x String) -> Self {
        Self::Text(value.into())
    }
}

impl<'x> From<Cow<'x, str>> for Value<'x> {
    fn from(value: Cow<'x, str>) -> Self {
        Self::Text(value)
    }
}

impl<'x> From<bool> for Value<'x> {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl<'x> From<i64> for Value<'x> {
    fn from(value: i64) -> Self {
        Self::Integer(value)
    }
}

impl<'x> From<u64> for Value<'x> {
    fn from(value: u64) -> Self {
        Self::Integer(value as i64)
    }
}

impl<'x> From<u32> for Value<'x> {
    fn from(value: u32) -> Self {
        Self::Integer(value as i64)
    }
}

impl<'x> From<f64> for Value<'x> {
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}

impl<'x> From<&'x [u8]> for Value<'x> {
    fn from(value: &'x [u8]) -> Self {
        Self::Blob(value.into())
    }
}

impl<'x> From<Vec<u8>> for Value<'x> {
    fn from(value: Vec<u8>) -> Self {
        Self::Blob(value.into())
    }
}

impl<'x> Value<'x> {
    pub fn into_string(self) -> String {
        match self {
            Value::Text(s) => s.into_owned(),
            Value::Integer(i) => i.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Float(f) => f.to_string(),
            Value::Blob(b) => String::from_utf8_lossy(b.as_ref()).into_owned(),
            Value::Null => String::new(),
        }
    }
}

impl From<Row> for Vec<String> {
    fn from(value: Row) -> Self {
        value.values.into_iter().map(|v| v.into_string()).collect()
    }
}

impl From<Row> for Vec<u32> {
    fn from(value: Row) -> Self {
        value
            .values
            .into_iter()
            .filter_map(|v| {
                if let Value::Integer(v) = v {
                    Some(v as u32)
                } else {
                    None
                }
            })
            .collect()
    }
}

impl From<Rows> for Vec<String> {
    fn from(value: Rows) -> Self {
        value
            .rows
            .into_iter()
            .flat_map(|v| v.values.into_iter().map(|v| v.into_string()))
            .collect()
    }
}

impl From<Rows> for Vec<u32> {
    fn from(value: Rows) -> Self {
        value
            .rows
            .into_iter()
            .flat_map(|v| {
                v.values.into_iter().filter_map(|v| {
                    if let Value::Integer(v) = v {
                        Some(v as u32)
                    } else {
                        None
                    }
                })
            })
            .collect()
    }
}
