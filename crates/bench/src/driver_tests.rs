use super::*;
use crate::prereg::{load, tests::example_dir};

#[tokio::test]
async fn resume_skips_finished_runs_and_plain_run_refuses_to_overwrite() {
    let (dir, sha) = example_dir("resume");
    let pre = load(&dir, &sha).unwrap();
    let results = dir.join("results");
    let finished = |arm: &str, sc: &str| RunRecord {
        key_sha: pre.key_sha.clone(),
        scenarios_sha: pre.scenarios_sha.clone(),
        ..RunRecord::blank(arm, sc)
    };
    for arm in ["A", "D"] {
        for sc in ["EX01", "EX02"] {
            record::write(&results, &finished(arm, sc)).unwrap();
        }
    }
    // No live settings at all: any attempted run fails on a missing setting.
    let live = Live::default();
    let plan = |arms, resume| Plan {
        arms,
        prereg: &pre,
        results: &results,
        resume,
        subset8: false,
        live: &live,
        guide: "g",
    };
    run_arms(&plan(&[Arm::A, Arm::D], true)).await.unwrap();

    let err = run_arms(&plan(&[Arm::B, Arm::A], false))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("--resume") && err.contains("EX02"),
        "preflight lists results: {err}"
    );
    assert!(
        !results.join("B").exists(),
        "nothing ran before the preflight refused"
    );

    for expected in ["1", "2"] {
        let err = run_arms(&plan(&[Arm::B], true)).await.unwrap_err();
        assert!(
            err.to_string().contains("is not set"),
            "B has no result, so it tries to run: {err}"
        );
        let attempts = std::fs::read_to_string(results.join("B/EX01.attempt")).unwrap();
        assert_eq!(
            attempts, expected,
            "each start counts, so a rerun gets a fresh deal id"
        );
    }

    let foreign = RunRecord {
        key_sha: "other".into(),
        ..finished("D", "EX01")
    };
    record::write(&results, &foreign).unwrap();
    let err = run_arms(&plan(&[Arm::D], true)).await.unwrap_err();
    assert!(err.to_string().contains("other pre-registration files"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn deal_numbers_are_distinct_and_four_digits() {
    assert_eq!(deal_number(0, Arm::A, 0, 1).unwrap(), 1001);
    assert_eq!(deal_number(0, Arm::D, 2, 12).unwrap(), 4212);
    assert!(deal_number(0, Arm::A, 10, 1).is_err());
    assert!(deal_number(6000, Arm::D, 0, 1).is_err());
}
