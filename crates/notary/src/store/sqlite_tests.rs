use proxenos_app::ports::{DealReader, DealRepository, RepoError, RequestId};
use proxenos_app::secret::SecretHash;
use proxenos_app::stored::{SignRequest, SignRequestState, StoredDeal};
use proxenos_domain::deal::Deal;
use proxenos_domain::kind::DealKind;
use proxenos_domain::party::{Role, RoomId};
use proxenos_domain::timestamp::Timestamp;

use super::sqlite::Store;

fn room(s: &str) -> RoomId {
    RoomId::new(s).unwrap()
}

fn new_deal(id: &str) -> (StoredDeal, Vec<proxenos_domain::event::DealEvent>) {
    let (deal, events) = Deal::open(
        id.parse().unwrap(),
        DealKind::PurchaseOrder,
        room(&format!("room-{id}")),
        Timestamp(100_000),
        8,
        Role::A,
    )
    .unwrap();
    (StoredDeal::new(deal), events)
}

#[tokio::test]
async fn round_trip_and_room_lookup() {
    let store = Store::in_memory().await.unwrap();
    let (mut stored, events) = new_deal("D-0001");
    store
        .save(&stored, &events, Timestamp(1_000))
        .await
        .unwrap();

    stored.version = 1;
    let desk = stored.deal.record_desk(Role::B, room("desk-b")).unwrap();
    store.save(&stored, &desk, Timestamp(1_001)).await.unwrap();

    let loaded = store.load(stored.deal.id()).await.unwrap().unwrap();
    assert_eq!(loaded.version, 2);
    assert_eq!(loaded.deal, stored.deal);
    assert!(
        store
            .load(&"D-0009".parse().unwrap())
            .await
            .unwrap()
            .is_none()
    );

    for r in ["room-D-0001", "desk-b"] {
        let found = store.in_room(&room(r)).await.unwrap();
        assert_eq!(found.len(), 1, "{r}");
    }
    assert!(store.in_room(&room("desk-a")).await.unwrap().is_empty());
    assert_eq!(
        store.active().await.unwrap(),
        vec![stored.deal.id().clone()]
    );
}

#[tokio::test]
async fn stale_version_conflicts_and_writes_nothing() {
    let store = Store::in_memory().await.unwrap();
    let (stored, events) = new_deal("D-0001");
    store
        .save(&stored, &events, Timestamp(1_000))
        .await
        .unwrap();
    // Version 0 again: a second open of the same id.
    let again = store.save(&stored, &events, Timestamp(1_000)).await;
    assert_eq!(again, Err(RepoError::Conflict));

    let mut stale = store.load(stored.deal.id()).await.unwrap().unwrap();
    stale.version = 5;
    let desk = stale.deal.record_desk(Role::A, room("desk-a")).unwrap();
    let res = store.save(&stale, &desk, Timestamp(1_001)).await;
    assert_eq!(res, Err(RepoError::Conflict));
    assert_eq!(store.events(stored.deal.id()).await.unwrap().len(), 1);
    assert!(store.in_room(&room("desk-a")).await.unwrap().is_empty());
}

#[tokio::test]
async fn events_get_monotonic_seq_per_deal() {
    let store = Store::in_memory().await.unwrap();
    let (mut stored, mut events) = new_deal("D-0001");
    events.extend(stored.deal.record_desk(Role::A, room("desk-a")).unwrap());
    store
        .save(&stored, &events, Timestamp(1_000))
        .await
        .unwrap();
    stored.version = 1;
    let more = stored.deal.record_desk(Role::B, room("desk-b")).unwrap();
    store.save(&stored, &more, Timestamp(1_005)).await.unwrap();
    let (other, other_events) = new_deal("D-0002");
    store
        .save(&other, &other_events, Timestamp(1_006))
        .await
        .unwrap();

    let log = store.events(stored.deal.id()).await.unwrap();
    assert_eq!(log.iter().map(|e| e.seq).collect::<Vec<_>>(), [1, 2, 3]);
    assert_eq!(log[2].at, Timestamp(1_005));
    assert_eq!(log[2].event, more[0]);
    assert_eq!(store.events(other.deal.id()).await.unwrap()[0].seq, 1);
}

#[tokio::test]
async fn secrets_resolve_until_cleared() {
    let store = Store::in_memory().await.unwrap();
    let (mut stored, events) = new_deal("D-0001");
    let token = SecretHash::keyed(b"server", "seal-token-1");
    let code = SecretHash::keyed(b"server", "JC-2");
    stored.party_mut(Role::B).seal_token = Some(token);
    stored.party_mut(Role::B).join_code = Some(code);
    store
        .save(&stored, &events, Timestamp(1_000))
        .await
        .unwrap();

    let id = stored.deal.id().clone();
    assert_eq!(
        store.by_seal_token(&token).await.unwrap(),
        Some((id.clone(), Role::B))
    );
    assert_eq!(
        store.by_join_code(&code).await.unwrap(),
        Some((id.clone(), Role::B))
    );
    assert_eq!(store.by_join_code(&token).await.unwrap(), None);

    stored.version = 1;
    stored.party_mut(Role::B).seal_token = None;
    store.save(&stored, &[], Timestamp(1_001)).await.unwrap();
    assert_eq!(store.by_seal_token(&token).await.unwrap(), None);
    assert_eq!(
        store.by_join_code(&code).await.unwrap(),
        Some((id, Role::B))
    );
}

#[tokio::test]
async fn sign_request_resolves_to_its_active_deal() {
    let store = Store::in_memory().await.unwrap();
    let (mut stored, events) = new_deal("D-0002");
    let (other, other_events) = new_deal("D-0001");
    let request = RequestId("card-1".into());
    stored.sign_requests.push(SignRequest {
        request_id: request.clone(),
        role: Role::A,
        terms_hash: "ab".repeat(32).parse().unwrap(),
        state: SignRequestState::Open,
        answered_by: None,
        at: Timestamp(1_000),
    });
    store
        .save(&stored, &events, Timestamp(1_000))
        .await
        .unwrap();
    store
        .save(&other, &other_events, Timestamp(1_000))
        .await
        .unwrap();

    let id = stored.deal.id().clone();
    assert_eq!(store.by_sign_request(&request).await.unwrap(), Some(id));
    let unknown = RequestId("card-2".into());
    assert_eq!(store.by_sign_request(&unknown).await.unwrap(), None);

    stored.version = 1;
    let expired = stored.deal.expire_due(Timestamp(100_000));
    assert!(!expired.is_empty());
    store
        .save(&stored, &expired, Timestamp(100_000))
        .await
        .unwrap();
    assert_eq!(store.by_sign_request(&request).await.unwrap(), None);
}

#[tokio::test]
async fn seen_messages_are_idempotent() {
    let store = Store::in_memory().await.unwrap();
    assert!(store.mark_seen("m-1", Timestamp(1_000)).await.unwrap());
    assert!(!store.mark_seen("m-1", Timestamp(1_001)).await.unwrap());
    assert!(store.mark_seen("m-2", Timestamp(1_001)).await.unwrap());

    assert_eq!(store.prune_seen(Timestamp(1_001)).await.unwrap(), 1);
    assert!(store.mark_seen("m-1", Timestamp(1_002)).await.unwrap());
    assert!(!store.mark_seen("m-2", Timestamp(1_002)).await.unwrap());
}

#[tokio::test]
async fn secret_hash_held_by_another_deal_conflicts() {
    let store = Store::in_memory().await.unwrap();
    let token = SecretHash::keyed(b"server", "seal-token-1");
    let (mut first, events) = new_deal("D-0001");
    first.party_mut(Role::A).seal_token = Some(token);
    store.save(&first, &events, Timestamp(1_000)).await.unwrap();
    let (mut second, events) = new_deal("D-0002");
    second.party_mut(Role::B).seal_token = Some(token);
    let res = store.save(&second, &events, Timestamp(1_000)).await;
    assert_eq!(res, Err(RepoError::Conflict));
    assert!(store.load(second.deal.id()).await.unwrap().is_none());
}

fn temp_db(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("proxenos-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("notary.db");
    (dir, path)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn racing_saves_of_one_version_let_exactly_one_win() {
    let (dir, path) = temp_db("race");
    let a = Store::open(&path).await.unwrap();
    let b = Store::open(&path).await.unwrap();
    let (stored, events) = new_deal("D-0001");

    let (ra, rb) = tokio::join!(
        a.save(&stored, &events, Timestamp(1_000)),
        b.save(&stored, &events, Timestamp(1_000))
    );
    assert_eq!([&ra, &rb].iter().filter(|r| r.is_ok()).count(), 1);
    assert!([ra, rb].contains(&Err(RepoError::Conflict)));

    let mut next = a.load(stored.deal.id()).await.unwrap().unwrap();
    let mut other = next.clone();
    let ea = next.deal.record_desk(Role::A, room("desk-a")).unwrap();
    let eb = other.deal.record_desk(Role::B, room("desk-b")).unwrap();
    let (ra, rb) = tokio::join!(
        a.save(&next, &ea, Timestamp(1_001)),
        b.save(&other, &eb, Timestamp(1_001))
    );
    assert_eq!([&ra, &rb].iter().filter(|r| r.is_ok()).count(), 1);
    assert!([ra, rb].contains(&Err(RepoError::Conflict)));
    assert_eq!(a.events(stored.deal.id()).await.unwrap().len(), 2);

    a.pool().close().await;
    b.pool().close().await;
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn file_database_persists_across_opens() {
    let (dir, path) = temp_db("db");
    let (stored, events) = new_deal("D-0001");
    {
        let store = Store::open(&path).await.unwrap();
        store
            .save(&stored, &events, Timestamp(1_000))
            .await
            .unwrap();
        store.pool().close().await;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let store = Store::open(&path).await.unwrap();
    assert_eq!(
        store.load(stored.deal.id()).await.unwrap().unwrap().version,
        1
    );
    store.pool().close().await;
    std::fs::remove_dir_all(&dir).unwrap();
}
