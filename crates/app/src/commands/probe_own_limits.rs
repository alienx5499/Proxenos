//! Command: probe_own_limits, own desk room only, `PROBES_PER_ROUND` per round.

use proxenos_domain::deal::DealState;
use proxenos_domain::deal_id::DealId;
use proxenos_domain::error::DomainError;
use proxenos_domain::event::DealEvent;
use proxenos_domain::kind::Field;
use proxenos_domain::party::{ParticipantId, RoomId};
use proxenos_domain::terms::Terms;
use serde::Serialize;

use super::{negotiator_role, own_limits};
use crate::error::AppError;
use crate::ports::{Clock, DealRepository, LimitVaultPort};
use crate::stored::{Check, CheckKind, PROBES_PER_ROUND, load_deal};

pub struct ProbeOwnLimits {
    pub deal: DealId,
    pub sender: ParticipantId,
    pub room: RoomId,
    pub terms: Terms,
}

/// Own side only: failing field names, never limit values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProbeOutcome {
    pub within: bool,
    pub failing_fields: Vec<Field>,
    pub probes_left: u8,
}

pub async fn probe_own_limits(
    repo: &impl DealRepository,
    vault: &impl LimitVaultPort,
    clock: &impl Clock,
    cmd: ProbeOwnLimits,
) -> Result<ProbeOutcome, AppError> {
    let mut stored = load_deal(repo, &cmd.deal).await?;
    let deal = &stored.deal;
    let role = negotiator_role(deal, &cmd.sender)?;
    if deal.desk(role) != Some(&cmd.room) {
        return Err(AppError::ProbeOutsideDesk);
    }
    match deal.state() {
        DealState::Open => {}
        DealState::AwaitingSeals => return Err(DomainError::AwaitingSeals.into()),
        _ => return Err(DomainError::DealClosed.into()),
    }
    let now = clock.now();
    if now >= deal.deadline() {
        return Err(DomainError::DeadlinePassed.into());
    }
    if cmd.terms.kind() != deal.kind() {
        return Err(DomainError::KindMismatch.into());
    }
    let round = deal.round();
    if stored.party(role).probes_left(round) == 0 {
        return Err(AppError::ProbeBudgetSpent);
    }
    let verdict = own_limits(vault, &stored, role)?.verdict(&cmd.terms);
    let party = stored.party_mut(role);
    if party.probes_round != round {
        party.probes_round = round;
        party.probes_used = 0;
    }
    party.probes_used += 1;
    let probes_left = PROBES_PER_ROUND - party.probes_used;
    party.record_check(Check {
        kind: CheckKind::Probe,
        round,
        within: verdict.within,
        failing_fields: verdict.failing_fields.clone(),
        at: now,
    });
    let events = [DealEvent::Probed {
        role,
        within: verdict.within,
    }];
    repo.save(&stored, &events, now).await?;
    Ok(ProbeOutcome {
        within: verdict.within,
        failing_fields: verdict.failing_fields,
        probes_left,
    })
}
