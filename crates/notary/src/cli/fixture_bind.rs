//! Seals each fixture side and checks or completes its owner and negotiator bindings.

use proxenos_app::commands::bind_negotiator::{BindNegotiator, bind_negotiator};
use proxenos_app::commands::bind_party::{BindParty, bind_party};
use proxenos_app::commands::seal_limits::{SealLimits, seal_limits};
use proxenos_app::ports::{Clock, DealRepository, LimitVaultPort, SecretSource};
use proxenos_app::stored::StoredDeal;
use proxenos_domain::party::{Role, RoomId};

use super::CliError;
use super::fixture::{Plan, scripted_limits};
use super::fixture_play::{Ports, diverged, fail, load, retry};

fn side(role: Role) -> &'static str {
    match role {
        Role::A => "buyer",
        Role::B => "seller",
    }
}

/// Seals a side that is not sealed yet (a sealed side must come without a token), then
/// checks or completes its owner and negotiator bindings. Returns the side's desk room.
pub(super) async fn seal_and_bind<R, V, C, S, W>(
    p: &Ports<'_, R, V, C, S, W>,
    plan: &Plan,
    stored: &StoredDeal,
    role: Role,
    today: i32,
) -> Result<RoomId, CliError>
where
    R: DealRepository,
    V: LimitVaultPort,
    C: Clock,
    S: SecretSource,
{
    let (i, name) = (usize::from(role == Role::B), side(role));
    let desk = stored
        .deal
        .desk(role)
        .cloned()
        .ok_or_else(|| CliError(format!("no desk kickoff recorded for the {name}")))?;
    ensure_sealed(p, plan, stored.deal.is_sealed(role), role, today, &desk).await?;
    let now = load(p, plan).await?;
    let Some(binding) = now.deal.binding(role) else {
        let what =
            format!("the {name} is sealed but no owner is bound (its join code was shown once)");
        return Err(diverged(plan, what));
    };
    if binding.owner != plan.owners[i] {
        return Err(diverged(
            plan,
            format!("the {name} owner is {}", binding.owner),
        ));
    }
    match &binding.agent {
        Some(a) if *a == plan.agents[i] => {}
        Some(a) => {
            return Err(diverged(
                plan,
                format!("the {name} negotiator is already {a}"),
            ));
        }
        None => {
            let cmd = BindNegotiator {
                deal: plan.deal.clone(),
                sender: plan.agents[i].clone(),
                room: desk.clone(),
                excluded: p.excluded.clone(),
            };
            retry(async || bind_negotiator(p.repo, p.clock, cmd.clone()).await)
                .await
                .map_err(|e| fail(plan, e))?;
        }
    }
    Ok(desk)
}

/// Seals the side with its token unless it is sealed already, then binds its owner if the
/// seal did not. A sealed side given a token, or an unsealed one without, is an error.
async fn ensure_sealed<R, V, C, S, W>(
    p: &Ports<'_, R, V, C, S, W>,
    plan: &Plan,
    sealed: bool,
    role: Role,
    today: i32,
    desk: &RoomId,
) -> Result<(), CliError>
where
    R: DealRepository,
    V: LimitVaultPort,
    C: Clock,
    S: SecretSource,
{
    let (i, name) = (usize::from(role == Role::B), side(role));
    match (sealed, &plan.seals[i]) {
        (true, Some(_)) => {
            let msg = format!("the {name} is already sealed; drop its seal token to resume");
            Err(CliError(msg))
        }
        (true, None) => Ok(()),
        (false, None) => Err(CliError(format!(
            "the {name} is not sealed yet; give its seal token"
        ))),
        (false, Some(token)) => {
            let cmd = SealLimits {
                token: token.expose().to_owned(),
                limits: scripted_limits(role, today),
            };
            let sealed =
                retry(async || seal_limits(p.repo, p.vault, p.clock, p.secrets, cmd.clone()).await)
                    .await
                    .map_err(|e| fail(plan, e))?;
            if !sealed.bound {
                let cmd = BindParty {
                    join_code: sealed.join_code,
                    sender: plan.owners[i].clone(),
                    room: desk.clone(),
                };
                retry(async || bind_party(p.repo, p.clock, p.secrets, cmd.clone()).await)
                    .await
                    .map_err(|e| fail(plan, e))?;
            }
            Ok(())
        }
    }
}
