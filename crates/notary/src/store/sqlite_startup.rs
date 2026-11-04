//! Startup checks against the existing database: bound deals and one sealed limit set.

use proxenos_app::ports::RepoError;
use proxenos_app::stored::SealedLimits;
use proxenos_domain::deal_id::DealId;
use proxenos_domain::party::Role;

use super::sqlite::Store;
use super::sqlite_rows::{backend, decode_deal};

impl Store {
    /// How many deals are Bound, so hold receipts signed with the current key.
    pub async fn bound_count(&self) -> Result<i64, RepoError> {
        sqlx::query_scalar("SELECT COUNT(*) FROM deals WHERE state = 'bound'")
            .fetch_one(&self.pool)
            .await
            .map_err(backend)
    }

    /// The newest deal's sealed limits, if any deal has some: one sample proves the vault key.
    /// Decodes bodies newest first, which is fine at a notary's scale.
    pub async fn sealed_sample(&self) -> Result<Option<(DealId, Role, SealedLimits)>, RepoError> {
        let rows: Vec<(String, i64)> =
            sqlx::query_as("SELECT body_json, version FROM deals ORDER BY id DESC")
                .fetch_all(&self.pool)
                .await
                .map_err(backend)?;
        for (body, version) in rows {
            let stored = decode_deal(&body, version)?;
            for role in [Role::A, Role::B] {
                if let Some(sealed) = &stored.party(role).sealed {
                    return Ok(Some((stored.deal.id().clone(), role, sealed.clone())));
                }
            }
        }
        Ok(None)
    }
}
