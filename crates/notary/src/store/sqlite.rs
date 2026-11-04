//! sqlx + SQLite adapter for DealReader and DealRepository, plus `seen_messages`.

use std::path::Path;
use std::time::Duration;

use proxenos_app::ports::{DealReader, DealRepository, RepoError, RequestId};
use proxenos_app::secret::SecretHash;
use proxenos_app::stored::{LoggedEvent, StoredDeal};
use proxenos_domain::deal_id::DealId;
use proxenos_domain::event::DealEvent;
use proxenos_domain::party::{Role, RoomId};
use proxenos_domain::timestamp::Timestamp;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};

use super::sqlite_rows::{backend, decode_deal, decode_event, parse_id, parse_role};
use super::sqlite_save::{append_events, write_deal, write_parties};

#[derive(Clone)]
pub struct Store {
    pub(super) pool: SqlitePool,
}

impl Store {
    /// Opens (creating it mode 0600 if needed) the database file in WAL mode and applies
    /// embedded migrations.
    pub async fn open(path: &Path) -> Result<Self, RepoError> {
        let mut create = std::fs::OpenOptions::new();
        create.write(true).create(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut create, 0o600);
        create.open(path).map_err(backend)?;
        let options = SqliteConnectOptions::new()
            .filename(path)
            .journal_mode(SqliteJournalMode::Wal);
        Self::connect(options, SqlitePoolOptions::new().max_connections(4)).await
    }

    /// A private in-memory database: one connection that is never recycled, since a new
    /// connection would get an empty copy.
    #[cfg(test)]
    pub async fn in_memory() -> Result<Self, RepoError> {
        let options = <SqliteConnectOptions as std::str::FromStr>::from_str("sqlite::memory:")
            .map_err(backend)?;
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .idle_timeout(None)
            .max_lifetime(None);
        Self::connect(options, pool).await
    }

    async fn connect(
        options: SqliteConnectOptions,
        pool: SqlitePoolOptions,
    ) -> Result<Self, RepoError> {
        let options = options
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));
        let pool = pool.connect_with(options).await.map_err(backend)?;
        sqlx::migrate!("src/store/migrations")
            .run(&pool)
            .await
            .map_err(backend)?;
        Ok(Self { pool })
    }

    #[cfg(test)]
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Records `message_id`; true the first time, false on every redelivery.
    pub async fn mark_seen(&self, message_id: &str, at: Timestamp) -> Result<bool, RepoError> {
        let done =
            sqlx::query("INSERT OR IGNORE INTO seen_messages (message_id, at) VALUES (?, ?)")
                .bind(message_id)
                .bind(at.0)
                .execute(&self.pool)
                .await
                .map_err(backend)?;
        Ok(done.rows_affected() == 1)
    }

    /// Forgets message ids first seen before `older_than`; returns how many were removed.
    pub async fn prune_seen(&self, older_than: Timestamp) -> Result<u64, RepoError> {
        let done = sqlx::query("DELETE FROM seen_messages WHERE at < ?")
            .bind(older_than.0)
            .execute(&self.pool)
            .await
            .map_err(backend)?;
        Ok(done.rows_affected())
    }

    async fn deals_where(
        &self,
        sql: &'static str,
        arg: &str,
    ) -> Result<Vec<StoredDeal>, RepoError> {
        let rows: Vec<(String, i64)> = sqlx::query_as(sql)
            .bind(arg)
            .fetch_all(&self.pool)
            .await
            .map_err(backend)?;
        rows.iter().map(|(body, v)| decode_deal(body, *v)).collect()
    }

    async fn party_by(
        &self,
        sql: &'static str,
        hash: &SecretHash,
    ) -> Result<Option<(DealId, Role)>, RepoError> {
        let row: Option<(String, String)> = sqlx::query_as(sql)
            .bind(hash.to_string())
            .fetch_optional(&self.pool)
            .await
            .map_err(backend)?;
        row.map(|(id, role)| Ok((parse_id(&id)?, parse_role(&role)?)))
            .transpose()
    }
}

impl DealReader for Store {
    async fn load(&self, id: &DealId) -> Result<Option<StoredDeal>, RepoError> {
        let sql = "SELECT body_json, version FROM deals WHERE id = ?";
        Ok(self.deals_where(sql, id.as_str()).await?.pop())
    }

    async fn in_room(&self, room: &RoomId) -> Result<Vec<StoredDeal>, RepoError> {
        let sql = "SELECT body_json, version FROM deals \
                   WHERE room_id = ?1 OR desk_a_room_id = ?1 OR desk_b_room_id = ?1 ORDER BY id";
        self.deals_where(sql, room.as_str()).await
    }

    async fn active(&self) -> Result<Vec<DealId>, RepoError> {
        let ids: Vec<(String,)> =
            sqlx::query_as("SELECT id FROM deals WHERE terminal = 0 ORDER BY id")
                .fetch_all(&self.pool)
                .await
                .map_err(backend)?;
        ids.iter().map(|(id,)| parse_id(id)).collect()
    }

    async fn dirty_sheets(&self) -> Result<Vec<DealId>, RepoError> {
        let sql =
            "SELECT id FROM deals WHERE json_extract(body_json, '$.sheet_dirty') = 1 ORDER BY id";
        let ids: Vec<(String,)> = sqlx::query_as(sql)
            .fetch_all(&self.pool)
            .await
            .map_err(backend)?;
        ids.iter().map(|(id,)| parse_id(id)).collect()
    }

    async fn by_seal_token(&self, hash: &SecretHash) -> Result<Option<(DealId, Role)>, RepoError> {
        self.party_by(
            "SELECT deal_id, role FROM parties WHERE seal_token_hash = ?",
            hash,
        )
        .await
    }

    async fn by_join_code(&self, hash: &SecretHash) -> Result<Option<(DealId, Role)>, RepoError> {
        self.party_by(
            "SELECT deal_id, role FROM parties WHERE join_code_hash = ?",
            hash,
        )
        .await
    }

    async fn by_sign_request(&self, request: &RequestId) -> Result<Option<DealId>, RepoError> {
        let sql = "SELECT deals.id FROM deals, json_each(deals.body_json, '$.sign_requests') AS r \
                   WHERE deals.terminal = 0 AND json_extract(r.value, '$.request_id') = ? \
                   ORDER BY deals.id LIMIT 1";
        let row: Option<(String,)> = sqlx::query_as(sql)
            .bind(&request.0)
            .fetch_optional(&self.pool)
            .await
            .map_err(backend)?;
        row.map(|(id,)| parse_id(&id)).transpose()
    }

    async fn events(&self, id: &DealId) -> Result<Vec<LoggedEvent>, RepoError> {
        let rows: Vec<(i64, i64, String)> = sqlx::query_as(
            "SELECT seq, at, payload_json FROM deal_events WHERE deal_id = ? ORDER BY seq",
        )
        .bind(id.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(backend)?;
        rows.iter()
            .map(|(seq, at, payload)| decode_event(*seq, *at, payload))
            .collect()
    }
}

impl DealRepository for Store {
    async fn save(
        &self,
        stored: &StoredDeal,
        events: &[DealEvent],
        at: Timestamp,
    ) -> Result<(), RepoError> {
        let id = stored.deal.id().as_str();
        let expected = i64::try_from(stored.version).map_err(backend)?;
        let mut next = stored.clone();
        next.version += 1;
        let body = serde_json::to_string(&next).map_err(backend)?;

        // IMMEDIATE takes the write lock up front, so any read inside stays consistent.
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(backend)?;
        write_deal(&mut tx, stored, expected, body).await?;
        write_parties(&mut tx, stored).await?;
        append_events(&mut tx, id, events, at).await?;
        tx.commit().await.map_err(backend)
    }
}
