//! Switch Gateway setup for one run: a fresh room group from the deal template, the arm's
//! addressing policies, and removal of the group afterwards. A room that held an earlier deal
//! is never reused: the notary refuses a second live deal in a room, and old history would
//! reach the agents.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::BenchError;
use crate::config::{Live, need};
use crate::driver::Arm;
use crate::notary_api::Http;
use crate::prereg::{Scenario, Side};

/// The deal template, shipped with the repo; the bench opens deals through the notary API,
/// so its kickoff lines (which would open a second deal) are left out.
pub fn template() -> String {
    let lines = include_str!("../../../templates/deal.yaml")
        .lines()
        .filter(|l| !l.trim_start().starts_with("kickoff:"));
    lines.map(|l| format!("{l}\n")).collect()
}

/// The template's side names for a kind, in (buyer, seller) order.
fn sides(kind: &str) -> [&'static str; 2] {
    match kind {
        "lease" => ["tenant", "landlord"],
        _ => ["buyer", "seller"],
    }
}

/// One run's rooms: the deal room and each side's desk, plus what removal needs.
#[derive(Clone, Debug, PartialEq)]
pub struct DealRooms {
    pub deal: String,
    pub desks: BTreeMap<Side, String>,
    room_ids: Vec<String>,
    group_id: Option<String>,
}

/// Reads the rooms a template run created, by the template's room names.
fn created(v: &Value, deal_id: &str, kind: &str) -> Result<DealRooms, BenchError> {
    let by_name: BTreeMap<&str, &str> = v["room"]["rooms"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| Some((r["room_name"].as_str()?, r["room_id"].as_str()?)))
        .collect();
    let find = |name: String| {
        by_name
            .get(name.as_str())
            .map(|id| id.to_string())
            .ok_or_else(|| {
                BenchError::Http(format!("template run made no room {name}: {}", short(v)))
            })
    };
    let [a, b] = sides(kind);
    Ok(DealRooms {
        deal: find(format!("{deal_id}-{kind}"))?,
        desks: BTreeMap::from([
            (Side::Buyer, find(format!("{deal_id}-desk-{a}"))?),
            (Side::Seller, find(format!("{deal_id}-desk-{b}"))?),
        ]),
        room_ids: by_name.values().map(|id| id.to_string()).collect(),
        group_id: None,
    })
}

/// The start of a Gateway answer, for error messages.
fn short(v: &Value) -> String {
    v.to_string().chars().take(300).collect()
}

/// A copy of `live` pointed at this run's fresh rooms.
pub async fn fresh_rooms(
    live: &Live,
    deal_id: &str,
    arm: Arm,
    sc: &Scenario,
) -> Result<(Live, DealRooms), BenchError> {
    let gw = client(live)?;
    let owners = both(
        &live.buyer_owner_id,
        &live.seller_owner_id,
        "BENCH_*_OWNER_ID",
    )?;
    let agents = both(
        &live.buyer_agent_id,
        &live.seller_agent_id,
        "BENCH_*_AGENT_ID",
    )?;
    let [side_a, side_b] = sides(&sc.kind);
    let inputs = json!({
        "deal_id": deal_id,
        "deal_name": format!("bench-{}-{}", arm.name(), sc.id).to_ascii_lowercase(),
        "kind": sc.kind, "side_a": side_a, "side_b": side_b,
        "party_a": owners[0], "party_b": owners[1], "notary": "notary",
    });
    let body = json!({"template": template(), "template_name": "proxenos-bench", "inputs": inputs});
    let url = gw.url("/gateway/rooms/from-template-document");
    let mut rooms = created(
        &gw.send(gw.http.post(url).json(&body)).await?,
        deal_id,
        &sc.kind,
    )?;
    rooms.group_id = group_of(
        &gw.send(gw.http.get(gw.url("/gateway/rooms"))).await?,
        &rooms.deal,
    );
    for (side, agent) in [Side::Buyer, Side::Seller].into_iter().zip(agents) {
        add_agent(&gw, &rooms.deal, agent).await?;
        add_agent(&gw, &rooms.desks[&side], agent).await?;
    }
    if let Some(injector) = &live.injector_agent_id {
        add_agent(&gw, &rooms.deal, injector).await?;
    }
    let pointed = Live {
        deal_room: Some(rooms.deal.clone()),
        buyer_desk: Some(rooms.desks[&Side::Buyer].clone()),
        seller_desk: Some(rooms.desks[&Side::Seller].clone()),
        ..live.clone()
    };
    Ok((pointed, rooms))
}

/// The group id `GET /gateway/rooms` lists for `room`.
fn group_of(listed: &Value, room: &str) -> Option<String> {
    let r = listed.as_array()?.iter().find(|r| r["id"] == room)?;
    r["group_id"].as_str().map(str::to_owned)
}

fn both<'a>(
    b: &'a Option<String>,
    s: &'a Option<String>,
    what: &str,
) -> Result<[&'a str; 2], BenchError> {
    Ok([need(b, what)?, need(s, what)?])
}

async fn add_agent(gw: &Http, room: &str, agent: &str) -> Result<(), BenchError> {
    let url = gw.url(&format!("/gateway/rooms/{room}/agents"));
    gw.send(gw.http.post(url).json(&json!({"agent_ids": [agent]})))
        .await
        .map(drop)
}

/// Deletes the run's rooms (and their channels), then their group; anything already gone is fine.
pub async fn remove_rooms(live: &Live, rooms: &DealRooms) -> Result<(), BenchError> {
    let gw = client(live)?;
    for id in &rooms.room_ids {
        let url = gw.url(&format!("/gateway/rooms/{id}?retire_channel=true"));
        match gw.send(gw.http.delete(url)).await {
            Err(BenchError::Http(m)) if m.starts_with("404") => {}
            other => other.map(drop)?,
        }
    }
    if let Some(g) = &rooms.group_id {
        match gw
            .send(gw.http.delete(gw.url(&format!("/gateway/room-groups/{g}"))))
            .await
        {
            Err(BenchError::Http(m)) if m.starts_with("404") => {}
            other => other.map(drop)?,
        }
    }
    Ok(())
}

/// Arm A lets only the owner and the notary message each agent; B and D clear the policy.
pub async fn set_policies(live: &Live, arm: Arm) -> Result<(), BenchError> {
    let gw = client(live)?;
    let policy = match arm {
        Arm::A => Some(owner_and_notary(need(
            &live.notary_agent_id,
            "NOTARY_AGENT_ID",
        )?)),
        Arm::B | Arm::D => None,
    };
    for side in [Side::Buyer, Side::Seller] {
        let id = need(
            live.by_side(side, &live.buyer_agent_id, &live.seller_agent_id),
            "agent id",
        )?;
        let url = gw.url(&format!("/gateway/agents/{id}/addressing-policy"));
        gw.send(gw.http.put(url).json(&json!({ "policy": policy })))
            .await?;
    }
    Ok(())
}

/// Arm A: the owner plus the notary agent, nobody else.
pub fn owner_and_notary(notary_agent_id: &str) -> Value {
    json!({ "rules": [{ "owner": true, "users": [], "agents": [notary_agent_id] }] })
}

fn client(live: &Live) -> Result<Http, BenchError> {
    Http::gateway(
        need(&live.gateway_url, "SWITCH_GATEWAY_URL")?,
        need(&live.gateway_key, "SWITCH_GATEWAY_KEY")?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_keeps_the_rooms_and_drops_every_kickoff() {
        let t = template();
        assert!(!t.contains("kickoff:"));
        assert_eq!(t.matches("- name: \"{deal_id}-").count(), 3);
    }

    #[test]
    fn created_rooms_are_found_by_template_name_per_kind() {
        let v = json!({"room": {"rooms": [
            {"room_name": "D-1101-lease", "room_id": "r-deal"},
            {"room_name": "D-1101-desk-tenant", "room_id": "r-a"},
            {"room_name": "D-1101-desk-landlord", "room_id": "r-b"},
        ]}});
        let r = created(&v, "D-1101", "lease").unwrap();
        assert_eq!(r.deal, "r-deal");
        assert_eq!(r.desks[&Side::Buyer], "r-a");
        assert_eq!(r.desks[&Side::Seller], "r-b");
        assert_eq!(r.room_ids.len(), 3);
        let listed = json!([{"id": "r-x"}, {"id": "r-deal", "group_id": "g1"}]);
        assert_eq!(group_of(&listed, "r-deal").as_deref(), Some("g1"));
        assert_eq!(group_of(&listed, "r-x"), None);
        let err = created(&v, "D-1101", "purchase-order").unwrap_err();
        assert!(matches!(err, BenchError::Http(m) if m.contains("D-1101-purchase-order")));
    }
}
