use super::*;
use serde_json::json;

#[test]
fn constraint_shape_and_admits() {
    let c: Constraint = serde_json::from_value(json!({"between": [100, 500]})).unwrap();
    assert!(c.admits(&json!(100)) && c.admits(&json!("500")) && !c.admits(&json!(501)));
    let d: Constraint = serde_json::from_value(json!({"no_later_than": 30})).unwrap();
    assert!(d.admits(&json!(29)) && !d.admits(&json!(31)));
    assert!(Constraint::AtMost(json!("b")).admits(&json!("a")));
    assert!(!Constraint::AtMost(json!(5)).admits(&Value::Null));
    assert_eq!(Constraint::AtLeast(json!("4.25")).numbers(), vec![4.25]);
}

/// The shipped example files, copied under the names the driver loads, with their
/// checksum files. Returns the directory and the key's sha256.
pub fn example_dir(name: &str) -> (std::path::PathBuf, String) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench/prereg");
    let dir = std::env::temp_dir().join(format!("proxenos-bench-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut key_sha = String::new();
    for (from, to) in [
        ("example.scenarios.json", "scenarios"),
        ("example.answer_key.json", "answer_key"),
    ] {
        let bytes = std::fs::read(src.join(from)).unwrap();
        let sha = sha256_hex(&bytes);
        std::fs::write(dir.join(format!("{to}.json")), &bytes).unwrap();
        std::fs::write(
            dir.join(format!("{to}.sha256")),
            format!("{sha}  {to}.json\n"),
        )
        .unwrap();
        if to == "answer_key" {
            key_sha = sha;
        }
    }
    (dir, key_sha)
}

#[test]
fn example_files_load_and_changed_files_are_refused() {
    let (dir, sha) = example_dir("hash");
    let pre = load(&dir, &sha).unwrap();
    assert!(pre.example() && pre.key_sha == sha);
    assert_eq!(pre.scenarios.scenarios.len(), 2);
    assert!(pre.key.scenarios["EX02"].zopa.is_none());
    assert!(matches!(
        load(&dir, &"0".repeat(64)),
        Err(BenchError::KeyHash { .. })
    ));

    let sc = dir.join("scenarios.json");
    let edited = std::fs::read_to_string(&sc)
        .unwrap()
        .replace("fair price", "good price");
    std::fs::write(&sc, edited).unwrap();
    let err = load(&dir, &sha).err().unwrap().to_string();
    assert!(err.contains("scenarios.sha256"), "{err}");

    let key = dir.join("answer_key.json");
    let edited = std::fs::read_to_string(&key)
        .unwrap()
        .replace("5.75", "5.85");
    std::fs::write(&key, edited).unwrap();
    assert!(matches!(load(&dir, &sha), Err(BenchError::KeyHash { .. })));
    std::fs::remove_dir_all(dir).unwrap();
}
