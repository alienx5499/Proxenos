//! The writes of one `DealRepository::save`, each run inside its transaction.

use proxenos_app::ports::RepoError;
use proxenos_app::stored::StoredDeal;
use proxenos_domain::event::DealEvent;
use proxenos_domain::party::{Role, RoomId};
use proxenos_domain::timestamp::Timestamp;
use sqlx::SqliteConnection;

use super::sqlite_rows::{backend, encode_event, role_text, state_columns};

/// Inserts the deal row when `expected` is 0, else updates it only at version `expected`.
pub(super) async fn write_deal(
    tx: &mut SqliteConnection,
    stored: &StoredDeal,
    expected: i64,
    body: String,
) -> Result<(), RepoError> {
    let deal = &stored.deal;
    let (state, terms_hash) = state_columns(deal.state());
    let desk = |r| deal.desk(r).map(|d: &RoomId| d.as_str().to_owned());
    let sql = if expected == 0 {
        "INSERT INTO deals (kind, room_id, desk_a_room_id, desk_b_room_id, state, terminal, \
         round, max_rounds, deadline, terms_hash, body_json, version, id) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
    } else {
        "UPDATE deals SET kind = ?, room_id = ?, desk_a_room_id = ?, desk_b_room_id = ?, \
         state = ?, terminal = ?, round = ?, max_rounds = ?, deadline = ?, terms_hash = ?, \
         body_json = ?, version = ? WHERE id = ? AND version = ?"
    };
    let mut query = sqlx::query(sql)
        .bind(deal.kind().name())
        .bind(deal.room().as_str())
        .bind(desk(Role::A))
        .bind(desk(Role::B))
        .bind(state)
        .bind(deal.state().is_terminal())
        .bind(i64::from(deal.round()))
        .bind(i64::from(deal.max_rounds()))
        .bind(deal.deadline().0)
        .bind(terms_hash)
        .bind(body)
        .bind(expected + 1)
        .bind(deal.id().as_str());
    if expected != 0 {
        query = query.bind(expected);
    }
    let written = query.execute(&mut *tx).await;
    match written {
        Ok(done) if done.rows_affected() == 1 => Ok(()),
        Ok(_) => Err(RepoError::Conflict),
        Err(e) => Err(conflict_or_backend(e)),
    }
}

/// Rewrites both party rows, so a used secret's hash is cleared.
pub(super) async fn write_parties(
    tx: &mut SqliteConnection,
    stored: &StoredDeal,
) -> Result<(), RepoError> {
    let id = stored.deal.id().as_str();
    sqlx::query("DELETE FROM parties WHERE deal_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(backend)?;
    for role in [Role::A, Role::B] {
        let p = stored.party(role);
        sqlx::query(
            "INSERT INTO parties (deal_id, role, seal_token_hash, join_code_hash) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(id)
        .bind(role_text(role))
        .bind(p.seal_token.map(|h| h.to_string()))
        .bind(p.join_code.map(|h| h.to_string()))
        .execute(&mut *tx)
        .await
        .map_err(conflict_or_backend)?;
    }
    Ok(())
}

/// Appends `events` after the deal's last sequence number, all stamped `at`.
pub(super) async fn append_events(
    tx: &mut SqliteConnection,
    id: &str,
    events: &[DealEvent],
    at: Timestamp,
) -> Result<(), RepoError> {
    let (last,): (i64,) =
        sqlx::query_as("SELECT COALESCE(MAX(seq), 0) FROM deal_events WHERE deal_id = ?")
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(backend)?;
    for (seq, event) in (last + 1..).zip(events) {
        let (kind, payload) = encode_event(event)?;
        sqlx::query(
            "INSERT INTO deal_events (deal_id, seq, type, payload_json, at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(seq)
        .bind(kind)
        .bind(payload)
        .bind(at.0)
        .execute(&mut *tx)
        .await
        .map_err(backend)?;
    }
    Ok(())
}

/// A unique violation (same deal id, or a secret hash already held by another party) is a
/// conflict the caller can retry; anything else is a backend failure.
fn conflict_or_backend(e: sqlx::Error) -> RepoError {
    if e.as_database_error()
        .is_some_and(|d| d.is_unique_violation())
    {
        RepoError::Conflict
    } else {
        backend(e)
    }
}
