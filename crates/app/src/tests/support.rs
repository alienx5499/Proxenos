//! Test harness: fakes and one-call steps.

use proxenos_domain::deal::DEFAULT_OPENER;
use proxenos_domain::deal_id::DealId;
use proxenos_domain::kind::DealKind;
use proxenos_domain::limit::LimitSet;
use proxenos_domain::offer::{Note, OfferId};
use proxenos_domain::party::{ParticipantId, Role, RoomId};
use proxenos_domain::signature::MessageId;
use proxenos_domain::terms::Terms;
use proxenos_domain::timestamp::Timestamp;

use super::fixtures::limits;
use crate::AppError;
use crate::commands::accept_offer::{AcceptOffer, AcceptOutcome, accept_offer};
use crate::commands::bind_negotiator::{BindNegotiator, bind_negotiator};
use crate::commands::bind_party::{BindParty, bind_party};
use crate::commands::open_deal::{OpenDeal, RecordDesk, SealTokens, open_deal, record_desk};
use crate::commands::open_sign_card::{OpenSignCard, open_sign_card};
use crate::commands::probe_own_limits::{ProbeOutcome, ProbeOwnLimits, probe_own_limits};
use crate::commands::record_signature::{RecordSignature, SignOutcome, record_signature};
use crate::commands::seal_limits::{SealLimits, Sealed, seal_limits};
use crate::commands::submit_offer::{OfferOutcome, SubmitOffer, submit_offer};
use crate::fakes::{CountingSecrets, FakeSigner, FakeSwitch, FakeVault, FixedClock, InMemoryDeals};
use crate::ports::{CardAnswer, CardOutcome, RequestId};

pub const DEADLINE: Timestamp = Timestamp(100_000);
pub fn id(s: &str) -> DealId {
    s.parse().unwrap()
}
pub fn pid(s: &str) -> ParticipantId {
    ParticipantId::new(s).unwrap()
}
pub fn room(s: &str) -> RoomId {
    RoomId::new(s).unwrap()
}
pub fn deal_room() -> RoomId {
    room("deal-room")
}
pub fn desk(r: Role) -> RoomId {
    room(&format!("desk-{r:?}"))
}
pub fn owner(r: Role) -> ParticipantId {
    pid(&format!("owner-{r:?}"))
}
pub fn agent(r: Role) -> ParticipantId {
    pid(&format!("agent-{r:?}"))
}

#[derive(Default)]
pub struct H {
    pub repo: InMemoryDeals,
    pub vault: FakeVault,
    pub clock: FixedClock,
    pub secrets: CountingSecrets,
    pub signer: FakeSigner,
    pub switch: FakeSwitch,
    msgs: std::sync::atomic::AtomicU32,
}

impl H {
    pub fn msg(&self) -> MessageId {
        let n = self.msgs.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        MessageId::new(format!("m{n}")).unwrap()
    }

    pub async fn open(&self, deal: &str, kind: DealKind) -> SealTokens {
        self.open_at(deal, kind, deal_room(), [desk(Role::A), desk(Role::B)])
            .await
    }

    /// Opens in the given rooms, desks recorded by the desk kickoff path.
    pub async fn open_at(
        &self,
        deal: &str,
        kind: DealKind,
        room: RoomId,
        desks: [RoomId; 2],
    ) -> SealTokens {
        let cmd = OpenDeal {
            id: id(deal),
            kind,
            room,
            deadline: DEADLINE,
            max_rounds: 8,
            opener: DEFAULT_OPENER,
            parties: None,
        };
        let tokens = open_deal(&self.repo, &self.clock, &self.secrets, cmd)
            .await
            .unwrap();
        for (role, room) in [Role::A, Role::B].into_iter().zip(desks) {
            let cmd = RecordDesk {
                deal: id(deal),
                role,
                room,
            };
            record_desk(&self.repo, &self.clock, cmd).await.unwrap();
        }
        tokens
    }

    pub async fn seal(&self, token: &str, limits: LimitSet) -> Result<Sealed, AppError> {
        let cmd = SealLimits {
            token: token.into(),
            limits,
        };
        seal_limits(&self.repo, &self.vault, &self.clock, &self.secrets, cmd).await
    }

    pub async fn join(
        &self,
        code: &str,
        sender: ParticipantId,
        room: RoomId,
    ) -> Result<(DealId, Role), AppError> {
        let cmd = BindParty {
            join_code: code.into(),
            sender,
            room,
        };
        bind_party(&self.repo, &self.clock, &self.secrets, cmd).await
    }

    /// Opens, seals both sides, binds both owners and negotiators: the deal is Open.
    pub async fn ready(&self, deal: &str, kind: DealKind) {
        let tokens = self.open(deal, kind).await;
        for (role, token) in [(Role::A, tokens.role_a), (Role::B, tokens.role_b)] {
            let sealed = self.seal(&token, limits(kind, role)).await.unwrap();
            self.join(&sealed.join_code, owner(role), desk(role))
                .await
                .unwrap();
            let cmd = BindNegotiator {
                deal: id(deal),
                sender: agent(role),
                room: desk(role),
                excluded: vec![pid("notary")],
            };
            bind_negotiator(&self.repo, &self.clock, cmd).await.unwrap();
        }
    }

    pub async fn probe(
        &self,
        deal: &str,
        role: Role,
        room: RoomId,
        terms: Terms,
    ) -> Result<ProbeOutcome, AppError> {
        let cmd = ProbeOwnLimits {
            deal: id(deal),
            sender: agent(role),
            room,
            terms,
        };
        probe_own_limits(&self.repo, &self.vault, &self.clock, cmd).await
    }

    pub async fn offer(
        &self,
        deal: &str,
        role: Role,
        terms: Terms,
    ) -> Result<OfferOutcome, AppError> {
        self.offer_in(deal, role, terms, desk(role)).await
    }

    pub async fn offer_in(
        &self,
        deal: &str,
        role: Role,
        terms: Terms,
        room: RoomId,
    ) -> Result<OfferOutcome, AppError> {
        self.offer_noted(deal, role, terms, room, "data only").await
    }

    pub async fn offer_noted(
        &self,
        deal: &str,
        role: Role,
        terms: Terms,
        room: RoomId,
        note: &str,
    ) -> Result<OfferOutcome, AppError> {
        self.clock.advance(10);
        let note = Note::new(note).unwrap();
        let cmd = SubmitOffer {
            deal: id(deal),
            sender: agent(role),
            room,
            terms,
            note,
            message_id: self.msg(),
        };
        submit_offer(&self.repo, &self.vault, &self.clock, cmd).await
    }

    pub async fn accept(
        &self,
        deal: &str,
        role: Role,
        offer: &str,
    ) -> Result<AcceptOutcome, AppError> {
        self.accept_in(deal, role, offer, desk(role)).await
    }

    pub async fn accept_in(
        &self,
        deal: &str,
        role: Role,
        offer: &str,
        room: RoomId,
    ) -> Result<AcceptOutcome, AppError> {
        self.clock.advance(10);
        let offer_id: OfferId = offer.parse().unwrap();
        let cmd = AcceptOffer {
            deal: id(deal),
            sender: agent(role),
            room,
            offer_id,
        };
        accept_offer(&self.repo, &self.vault, &self.clock, cmd).await
    }

    /// A typed `sign <hash>`.
    pub async fn sign(
        &self,
        deal: &str,
        signer: ParticipantId,
        hash: &str,
    ) -> Result<SignOutcome, AppError> {
        self.clock.advance(10);
        let cmd = RecordSignature::Typed {
            deal: id(deal),
            signer,
            hash: hash.into(),
            message_id: self.msg(),
        };
        record_signature(&self.repo, &self.clock, &self.signer, cmd).await
    }

    pub async fn open_card(&self, deal: &str, role: Role) -> Result<RequestId, AppError> {
        let cmd = OpenSignCard {
            deal: id(deal),
            role,
        };
        open_sign_card(&self.repo, &self.switch, &self.clock, cmd).await
    }

    pub async fn card(
        &self,
        deal: &str,
        request: RequestId,
        answer: CardAnswer,
    ) -> Result<SignOutcome, AppError> {
        self.clock.advance(10);
        let outcome = CardOutcome { request, answer };
        let cmd = RecordSignature::Card {
            deal: id(deal),
            outcome,
        };
        record_signature(&self.repo, &self.clock, &self.signer, cmd).await
    }
}
