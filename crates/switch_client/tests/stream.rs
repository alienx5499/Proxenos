mod common;

use std::time::Duration;

use common::{Mock, Script, Sse, message_frame, state_frame};
use futures_util::StreamExt;
use proxenos_switch_client::stream::{EventKind, Item, StreamSpec, Subscription};
use proxenos_switch_client::{Error, heartbeat};

async fn next(sub: &mut Subscription) -> Item {
    tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("timed out")
        .expect("stream ended")
        .expect("stream error")
}

fn sse(body: String, hold_open: bool) -> Sse {
    Sse { body, hold_open }
}

#[tokio::test]
async fn resumes_with_last_event_id_and_adopts_gap_cursor() {
    let script = Script {
        sse: [
            sse(state_frame("c", 30.0) + &message_frame(5, "@notary status"), false),
            sse(
                state_frame("c", 30.0)
                    + "event: gap\ndata: {\"from_sequence\":5,\"resumed_at\":10,\"reason\":\"restart\"}\n\n",
                false,
            ),
            sse(state_frame("c", 30.0) + &message_frame(11, "later"), true),
        ]
        .into(),
        ..Script::default()
    };
    let mock = Mock::start(script).await;
    let mut sub = mock.client().subscribe(StreamSpec::all_addressed());

    assert!(matches!(next(&mut sub).await, Item::Connected(s) if s.generation == Some(3)));
    let Item::Event(e) = next(&mut sub).await else {
        panic!("expected event")
    };
    assert_eq!(e.sequence, Some(5));
    let EventKind::Message(m) = e.kind else {
        panic!("expected message")
    };
    assert_eq!(
        (m.message_id.as_str(), m.body.as_str()),
        ("m5", "@notary status")
    );

    assert!(matches!(next(&mut sub).await, Item::Connected(_)));
    assert!(matches!(next(&mut sub).await, Item::Gap(g) if g.resumed_at == Some(10)));
    assert!(matches!(next(&mut sub).await, Item::Connected(_)));
    assert!(matches!(next(&mut sub).await, Item::Event(e) if e.sequence == Some(11)));

    let opens = mock.seen_path("/agents/ag1/events");
    assert_eq!(opens.len(), 3);
    let conn = sub.connection_id().to_owned();
    for o in &opens {
        assert_eq!(o.param("connection_id").as_deref(), Some(conn.as_str()));
        assert_eq!(o.param("scope").as_deref(), Some("all"));
        assert_eq!(o.param("filter").as_deref(), Some("addressed"));
        assert_eq!(o.header("authorization").as_deref(), Some("Bearer key1"));
        assert_eq!(o.header("accept").as_deref(), Some("text/event-stream"));
    }
    assert_eq!(opens[0].header("last-event-id"), None);
    assert_eq!(opens[1].header("last-event-id").as_deref(), Some("5"));
    assert_eq!(opens[2].header("last-event-id").as_deref(), Some("10"));
}

#[tokio::test]
async fn heartbeat_404_reopens_the_same_connection() {
    let script = Script {
        sse: [
            sse(state_frame("c", 0.05), true),
            sse(state_frame("c", 30.0), true),
        ]
        .into(),
        beat_status: 404,
        ..Script::default()
    };
    let mock = Mock::start(script).await;
    let mut sub = mock.client().subscribe(StreamSpec::all_addressed());
    assert!(matches!(next(&mut sub).await, Item::Connected(_)));
    assert!(matches!(next(&mut sub).await, Item::Connected(_)));

    let beat = &mock.seen_path("/agents/ag1/connection/beat")[0];
    assert_eq!(beat.json()["connection_id"], sub.connection_id());
    assert_eq!(beat.json()["generation"], 3);
    let opens = mock.seen_path("/agents/ag1/events");
    assert_eq!(
        opens[0].param("connection_id"),
        opens[1].param("connection_id")
    );
}

#[tokio::test]
async fn heartbeat_run_returns_on_409() {
    let mock = Mock::start(Script {
        beat_status: 409,
        ..Script::default()
    })
    .await;
    let client = mock.client();
    let err = tokio::time::timeout(
        Duration::from_secs(5),
        heartbeat::run(&client, "c1", Some(1), Duration::from_millis(10)),
    )
    .await
    .unwrap();
    assert!(heartbeat::means_reopen(&err));
}

#[tokio::test]
async fn evicted_reopens_but_taken_over_is_terminal() {
    let script = Script {
        sse: [
            sse(
                state_frame("c", 30.0)
                    + "event: evicted\ndata: {\"code\":\"heartbeat_lapsed\",\"reason\":\"x\"}\n\n",
                true,
            ),
            sse(
                state_frame("c", 30.0)
                    + "event: evicted\ndata: {\"code\":\"taken_over\",\"reason\":\"y\"}\n\n",
                true,
            ),
        ]
        .into(),
        ..Script::default()
    };
    let mock = Mock::start(script).await;
    let mut sub = mock.client().subscribe(StreamSpec::all_addressed());
    assert!(matches!(next(&mut sub).await, Item::Connected(_)));
    assert!(matches!(next(&mut sub).await, Item::Connected(_)));
    let end = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .unwrap();
    assert!(matches!(end, Some(Err(Error::TakenOver))));
    assert!(sub.next().await.is_none());
}

#[tokio::test]
async fn refused_open_is_terminal() {
    let mock = Mock::start(Script {
        open_status: 403,
        ..Script::default()
    })
    .await;
    let mut sub = mock.client().subscribe(StreamSpec::room_addressed("r&x=1"));
    let end = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .unwrap();
    assert!(matches!(end, Some(Err(ref e)) if e.status() == Some(403)));
    assert!(sub.next().await.is_none());
    let open = &mock.seen_path("/agents/ag1/events")[0];
    assert_eq!(open.param("scope").as_deref(), Some("single"));
    assert_eq!(open.param("rooms").as_deref(), Some("r%26x%3D1"));
    assert_eq!(open.param("x"), None);
}

const TAKEN: &str = r#"{"detail":{"code":"taken_over","message":"another client holds it"}}"#;

#[tokio::test]
async fn beat_409_taken_over_is_terminal_even_with_a_tiny_interval() {
    let script = Script {
        sse: [sse(state_frame("c", 1e-9), true)].into(),
        beat_status: 409,
        beat_body: Some(TAKEN.into()),
        ..Script::default()
    };
    let mock = Mock::start(script).await;
    let mut sub = mock.client().subscribe(StreamSpec::all_addressed());
    assert!(matches!(next(&mut sub).await, Item::Connected(_)));
    let end = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .unwrap();
    assert!(matches!(end, Some(Err(Error::TakenOver))));
    assert!(sub.next().await.is_none());
    assert_eq!(mock.seen_path("/agents/ag1/events").len(), 1);

    let err = heartbeat::run(&mock.client(), "c", Some(1), Duration::ZERO).await;
    assert!(matches!(err, Error::TakenOver));
}

#[tokio::test]
async fn reopens_back_off_and_grow_while_connections_die_young() {
    let script = Script {
        sse: [
            sse(state_frame("c", 30.0), false),
            sse(state_frame("c", 30.0), false),
            sse(state_frame("c", 30.0), true),
        ]
        .into(),
        ..Script::default()
    };
    let mock = Mock::start(script).await;
    let mut sub = mock.client().subscribe(StreamSpec::all_addressed());
    for _ in 0..3 {
        assert!(matches!(next(&mut sub).await, Item::Connected(_)));
    }
    let opens = mock.seen_path("/agents/ag1/events");
    let first = opens[1].at - opens[0].at;
    let second = opens[2].at - opens[1].at;
    assert!(first >= Duration::from_millis(250), "{first:?}");
    assert!(second >= Duration::from_millis(500), "{second:?}");
}

#[tokio::test]
async fn slow_consumer_does_not_starve_the_heartbeat() {
    let mut body = state_frame("c", 0.1);
    for seq in 1..=300 {
        body.push_str(&message_frame(seq, "x"));
    }
    let script = Script {
        sse: [sse(body, true)].into(),
        ..Script::default()
    };
    let mock = Mock::start(script).await;
    let mut sub = mock.client().subscribe(StreamSpec::all_addressed());
    assert!(matches!(next(&mut sub).await, Item::Connected(_)));
    tokio::time::sleep(Duration::from_millis(800)).await;
    let beats = mock.seen_path("/agents/ag1/connection/beat").len();
    assert!(beats >= 4, "only {beats} beats while the consumer stalled");
    assert!(matches!(next(&mut sub).await, Item::Event(e) if e.sequence == Some(1)));
}

#[tokio::test]
async fn silent_stream_is_reopened() {
    let script = Script {
        sse: [
            sse(state_frame("c", 30.0), true),
            sse(state_frame("c", 30.0), true),
        ]
        .into(),
        ..Script::default()
    };
    let mock = Mock::start(script).await;
    let client = mock
        .client()
        .with_stream_idle_timeout(Duration::from_millis(200));
    let mut sub = client.subscribe(StreamSpec::all_addressed());
    assert!(matches!(next(&mut sub).await, Item::Connected(_)));
    assert!(matches!(next(&mut sub).await, Item::Connected(_)));
    assert_eq!(mock.seen_path("/agents/ag1/events").len(), 2);
}

#[tokio::test]
async fn subscription_changed_is_surfaced() {
    let frame = "event: subscription_changed\ndata: {\"rooms\":[],\"reason\":\"claimed\"}\n\n";
    let script = Script {
        sse: [sse(state_frame("c", 30.0) + frame, true)].into(),
        ..Script::default()
    };
    let mock = Mock::start(script).await;
    let mut sub = mock.client().subscribe(StreamSpec::all_addressed());
    assert!(matches!(next(&mut sub).await, Item::Connected(_)));
    assert!(
        matches!(next(&mut sub).await, Item::SubscriptionChanged(c) if c.rooms.is_empty() && c.reason == "claimed")
    );
}
