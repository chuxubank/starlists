use std::path::PathBuf;
use std::process::Command;

fn bin() -> PathBuf {
    assert_cmd_bin()
}

fn assert_cmd_bin() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_BIN_EXE_stars"));
    assert!(path.exists(), "missing {}", path.display());
    path
}

fn tmp_db() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("stars.db");
    (dir, db)
}

#[test]
fn help_lists_core_commands() {
    let out = Command::new(bin())
        .arg("--help")
        .output()
        .expect("run help");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success());
    for needle in [
        "doctor", "snapshot", "lists", "repos", "assign", "export", "propose", "plan", "apply",
        "request",
    ] {
        assert!(stdout.contains(needle), "help missing {needle}\n{stdout}");
    }
}

#[test]
fn doctor_json_without_db() {
    let (_dir, db) = tmp_db();
    let out = Command::new(bin())
        .args(["--json", "--db"])
        .arg(&db)
        .arg("doctor")
        .output()
        .expect("run doctor");
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["data"]["db_exists"], false);
    assert!(v["data"]["auth_source"].is_string());
    assert!(v["error"].is_null());
}

#[test]
fn init_and_local_list_crud() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("stars.db");
    let config = dir.path().join("config.toml");

    let init = Command::new(bin())
        .args(["--json", "--db"])
        .arg(&db)
        .arg("--config")
        .arg(&config)
        .arg("init")
        .output()
        .unwrap();
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );

    let create = Command::new(bin())
        .args(["--json", "--db"])
        .arg(&db)
        .args(["lists", "create", "Web Mapping", "--desc", "maps"])
        .output()
        .unwrap();
    assert!(
        create.status.success(),
        "{}",
        String::from_utf8_lossy(&create.stderr)
    );
    let created: serde_json::Value = serde_json::from_slice(&create.stdout).unwrap();
    assert_eq!(created["data"]["slug"], "web-mapping");
    assert_eq!(created["data"]["status"], "draft");

    let lists = Command::new(bin())
        .args(["--json", "--db"])
        .arg(&db)
        .arg("lists")
        .output()
        .unwrap();
    let listed: serde_json::Value = serde_json::from_slice(&lists.stdout).unwrap();
    assert_eq!(listed["data"][0]["name"], "Web Mapping");

    let proposal = dir.path().join("proposal.json");
    std::fs::write(
        &proposal,
        r#"{
          "version": 1,
          "id": "PLAN_FIXTURE",
          "lists": { "create": [{ "slug": "emacs", "name": "Emacs" }] },
          "memberships": []
        }"#,
    )
    .unwrap();
    let imported = Command::new(bin())
        .args(["--json", "--db"])
        .arg(&db)
        .args(["propose", "import"])
        .arg(&proposal)
        .output()
        .unwrap();
    assert!(
        imported.status.success(),
        "{}",
        String::from_utf8_lossy(&imported.stderr)
    );
    let imported_v: serde_json::Value = serde_json::from_slice(&imported.stdout).unwrap();
    assert_eq!(imported_v["data"]["id"], "PLAN_FIXTURE");

    let plan = Command::new(bin())
        .args(["--json", "--db"])
        .arg(&db)
        .args(["plan", "show", "PLAN_FIXTURE"])
        .output()
        .unwrap();
    assert!(
        plan.status.success(),
        "{}",
        String::from_utf8_lossy(&plan.stderr)
    );
    let plan_v: serde_json::Value = serde_json::from_slice(&plan.stdout).unwrap();
    assert_eq!(plan_v["data"]["id"], "PLAN_FIXTURE");
    assert_eq!(plan_v["data"]["list_ops"][0]["op"], "create");
}

#[test]
fn proposal_roundtrip_empty_repo_errors() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("stars.db");
    let init = Command::new(bin())
        .args(["--json", "--db"])
        .arg(&db)
        .arg("init")
        .output()
        .unwrap();
    assert!(init.status.success());

    let proposal = dir.path().join("proposal.json");
    std::fs::write(
        &proposal,
        r#"{
          "version": 1,
          "notes": "fixture",
          "lists": { "create": [{ "slug": "emacs", "name": "Emacs" }] },
          "memberships": [{ "repo": "foo/bar", "lists": ["emacs"] }]
        }"#,
    )
    .unwrap();

    let imported = Command::new(bin())
        .args(["--json", "--db"])
        .arg(&db)
        .args(["propose", "import"])
        .arg(&proposal)
        .output()
        .unwrap();
    assert!(!imported.status.success());
    let v: serde_json::Value = serde_json::from_slice(&imported.stdout).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "not_found");
}
