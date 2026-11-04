//! Router test fakes: every port in memory.

use std::collections::HashSet;
use std::sync::Mutex;

use proxenos_app::fakes::{
    CountingSecrets, FakeRoster, FakeSigner, FakeSwitch, FakeVault, FixedClock, InMemoryDeals,
};
use proxenos_app::ports::RepoError;
use proxenos_domain::timestamp::Timestamp;

use super::super::{Ports, SeenMessages};

#[derive(Default)]
pub(super) struct FakeSeen(Mutex<HashSet<String>>);

impl SeenMessages for FakeSeen {
    async fn mark_seen(&self, id: &str, _at: Timestamp) -> Result<bool, RepoError> {
        Ok(self.0.lock().unwrap().insert(id.to_owned()))
    }
}

#[derive(Default)]
pub(super) struct Fakes {
    pub(super) repo: InMemoryDeals,
    pub(super) vault: FakeVault,
    pub(super) clock: FixedClock,
    pub(super) secrets: CountingSecrets,
    pub(super) signer: FakeSigner,
    pub(super) switch: FakeSwitch,
    pub(super) roster: FakeRoster,
    pub(super) seen: FakeSeen,
}

impl Ports for Fakes {
    type Repo = InMemoryDeals;
    type Vault = FakeVault;
    type Clock = FixedClock;
    type Secrets = CountingSecrets;
    type Signer = FakeSigner;
    type Switch = FakeSwitch;
    type Roster = FakeRoster;
    type Seen = FakeSeen;
    fn repo(&self) -> &InMemoryDeals {
        &self.repo
    }
    fn vault(&self) -> &FakeVault {
        &self.vault
    }
    fn clock(&self) -> &FixedClock {
        &self.clock
    }
    fn secrets(&self) -> &CountingSecrets {
        &self.secrets
    }
    fn signer(&self) -> &FakeSigner {
        &self.signer
    }
    fn switch(&self) -> &FakeSwitch {
        &self.switch
    }
    fn roster(&self) -> &FakeRoster {
        &self.roster
    }
    fn seen(&self) -> &FakeSeen {
        &self.seen
    }
}
