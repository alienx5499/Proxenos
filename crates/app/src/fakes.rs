//! In-memory fakes for every port, for use-case tests here and router tests in the notary.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};

use proxenos_domain::deal_id::DealId;
use proxenos_domain::event::DealEvent;
use proxenos_domain::limit::LimitSet;
use proxenos_domain::party::{Role, RoomId};
use proxenos_domain::timestamp::Timestamp;
use sha2::{Digest, Sha256};

use crate::ports::{
    Clock, DealReader, DealRepository, LimitVaultPort, ReceiptSigner, RepoError, RequestId,
    SecretSource, VaultError,
};
use crate::secret::SecretHash;
use crate::stored::{LoggedEvent, PartyState, SealedLimits, StoredDeal};

#[path = "fakes_switch.rs"]
mod switch;
pub use switch::{Admitted, FakeAdmin, FakeRoster, FakeSwitch, SwitchCall};

#[derive(Default)]
struct Store {
    deals: BTreeMap<DealId, StoredDeal>,
    events: BTreeMap<DealId, Vec<LoggedEvent>>,
}

/// `conflicts`: how many upcoming saves fail with `Conflict` (for retry tests).
#[derive(Default)]
pub struct InMemoryDeals(Mutex<Store>, pub AtomicU32);

impl InMemoryDeals {
    fn with<T>(&self, f: impl FnOnce(&mut Store) -> T) -> T {
        f(&mut self.0.lock().expect("store lock"))
    }

    fn find_secret(
        &self,
        pick: impl Fn(&PartyState) -> Option<SecretHash>,
        hash: &SecretHash,
    ) -> Option<(DealId, Role)> {
        self.with(|s| {
            s.deals.values().find_map(|d| {
                [Role::A, Role::B]
                    .into_iter()
                    .find(|r| pick(d.party(*r)).as_ref() == Some(hash))
                    .map(|r| (d.deal.id().clone(), r))
            })
        })
    }
}

impl DealReader for InMemoryDeals {
    async fn load(&self, id: &DealId) -> Result<Option<StoredDeal>, RepoError> {
        Ok(self.with(|s| s.deals.get(id).cloned()))
    }

    async fn in_room(&self, room: &RoomId) -> Result<Vec<StoredDeal>, RepoError> {
        Ok(self.with(|s| {
            s.deals
                .values()
                .filter(|d| {
                    d.deal.room() == room
                        || [Role::A, Role::B]
                            .iter()
                            .any(|r| d.deal.desk(*r) == Some(room))
                })
                .cloned()
                .collect()
        }))
    }

    async fn active(&self) -> Result<Vec<DealId>, RepoError> {
        Ok(self.with(|s| {
            s.deals
                .values()
                .filter(|d| !d.deal.state().is_terminal())
                .map(|d| d.deal.id().clone())
                .collect()
        }))
    }

    async fn dirty_sheets(&self) -> Result<Vec<DealId>, RepoError> {
        Ok(self.with(|s| {
            s.deals
                .values()
                .filter(|d| d.sheet_dirty)
                .map(|d| d.deal.id().clone())
                .collect()
        }))
    }

    async fn by_seal_token(&self, hash: &SecretHash) -> Result<Option<(DealId, Role)>, RepoError> {
        Ok(self.find_secret(|p| p.seal_token, hash))
    }

    async fn by_join_code(&self, hash: &SecretHash) -> Result<Option<(DealId, Role)>, RepoError> {
        Ok(self.find_secret(|p| p.join_code, hash))
    }

    async fn by_sign_request(&self, request: &RequestId) -> Result<Option<DealId>, RepoError> {
        Ok(self.with(|s| {
            s.deals
                .values()
                .filter(|d| !d.deal.state().is_terminal())
                .find(|d| d.sign_requests.iter().any(|r| &r.request_id == request))
                .map(|d| d.deal.id().clone())
        }))
    }

    async fn events(&self, id: &DealId) -> Result<Vec<LoggedEvent>, RepoError> {
        Ok(self.with(|s| s.events.get(id).cloned().unwrap_or_default()))
    }
}

impl DealRepository for InMemoryDeals {
    async fn save(
        &self,
        deal: &StoredDeal,
        events: &[DealEvent],
        at: Timestamp,
    ) -> Result<(), RepoError> {
        let injected = self
            .1
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1));
        if injected.is_ok() {
            return Err(RepoError::Conflict);
        }
        self.with(|s| {
            let id = deal.deal.id().clone();
            let current = s.deals.get(&id).map_or(0, |d| d.version);
            if current != deal.version {
                return Err(RepoError::Conflict);
            }
            let mut next = deal.clone();
            next.version += 1;
            s.deals.insert(id.clone(), next);
            let log = s.events.entry(id).or_default();
            for event in events {
                let seq = log.len() as u64 + 1;
                log.push(LoggedEvent {
                    seq,
                    at,
                    event: event.clone(),
                });
            }
            Ok(())
        })
    }
}

/// Reversible fake encryption: XOR with a per-(deal, role) keystream, nonce = AAD tag.
/// Decrypting under another deal or role, or after tampering, yields `Tampered`.
#[derive(Default)]
pub struct FakeVault;

fn aad_digest(label: &[u8], deal: &DealId, role: Role) -> Vec<u8> {
    let aad = format!("{deal}|{role:?}");
    Sha256::new()
        .chain_update(label)
        .chain_update(aad)
        .finalize()
        .to_vec()
}

fn xor(bytes: &[u8], key: &[u8]) -> Vec<u8> {
    bytes
        .iter()
        .zip(key.iter().cycle())
        .map(|(b, k)| b ^ k)
        .collect()
}

impl LimitVaultPort for FakeVault {
    fn encrypt(
        &self,
        deal: &DealId,
        role: Role,
        limits: &LimitSet,
    ) -> Result<SealedLimits, VaultError> {
        let plain = serde_json::to_vec(limits).map_err(|e| VaultError::Backend(e.to_string()))?;
        Ok(SealedLimits {
            nonce: aad_digest(b"tag", deal, role),
            ciphertext: xor(&plain, &aad_digest(b"key", deal, role)),
        })
    }

    fn decrypt(
        &self,
        deal: &DealId,
        role: Role,
        sealed: &SealedLimits,
    ) -> Result<LimitSet, VaultError> {
        if sealed.nonce != aad_digest(b"tag", deal, role) {
            return Err(VaultError::Tampered);
        }
        let plain = xor(&sealed.ciphertext, &aad_digest(b"key", deal, role));
        serde_json::from_slice(&plain).map_err(|_| VaultError::Tampered)
    }
}

/// Starts at `Timestamp(1_000)`; tests move it with `set` or `advance`.
pub struct FixedClock(AtomicI64);

impl Default for FixedClock {
    fn default() -> Self {
        Self(AtomicI64::new(1_000))
    }
}

impl FixedClock {
    pub fn set(&self, t: Timestamp) {
        self.0.store(t.0, Ordering::SeqCst);
    }

    pub fn advance(&self, secs: i64) {
        self.0.fetch_add(secs, Ordering::SeqCst);
    }
}

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        Timestamp(self.0.load(Ordering::SeqCst))
    }
}

/// Deterministic, not secure: SHA-256 of a fixed key and the bytes.
#[derive(Default)]
pub struct FakeSigner;

impl FakeSigner {
    pub const PUBLIC_KEY: &'static [u8] = b"fake-public-key";

    pub fn expected(bytes: &[u8]) -> Vec<u8> {
        Sha256::new()
            .chain_update(b"fake-private-key")
            .chain_update(bytes)
            .finalize()
            .to_vec()
    }
}

impl ReceiptSigner for FakeSigner {
    fn sign(&self, bytes: &[u8]) -> Vec<u8> {
        Self::expected(bytes)
    }

    fn public_key(&self) -> Vec<u8> {
        Self::PUBLIC_KEY.to_vec()
    }
}

/// Deterministic secrets: counter-derived random bytes, HMAC under a fixed key.
/// With `stuck` set, `random` repeats one value (for collision tests).
#[derive(Default)]
pub struct CountingSecrets {
    count: AtomicU32,
    pub stuck: AtomicBool,
}

impl CountingSecrets {
    pub const KEY: &'static [u8] = b"fake-server-key";
}

impl SecretSource for CountingSecrets {
    fn random(&self) -> [u8; 32] {
        let n = if self.stuck.load(Ordering::SeqCst) {
            0
        } else {
            self.count.fetch_add(1, Ordering::SeqCst) + 1
        };
        Sha256::digest(n.to_be_bytes()).into()
    }

    fn hash(&self, secret: &str) -> SecretHash {
        SecretHash::keyed(Self::KEY, secret)
    }
}
