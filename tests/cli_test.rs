use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_wechat-oa"))
}

fn temporary_draft() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("wechat-oa-cli-test-{}-{nonce}", std::process::id()));
    fs::create_dir_all(root.join("assets")).unwrap();
    fs::write(
        root.join("draft.json"),
        r#"{
            "schema_version": 1,
            "article_type": "news",
            "title": "Test article",
            "content_file": "content.html",
            "cover_file": "assets/cover.jpg"
        }"#,
    )
    .unwrap();
    fs::write(
        root.join("content.html"),
        r#"<p>Body</p><img src="assets/body.png">"#,
    )
    .unwrap();
    fs::write(root.join("assets/cover.jpg"), [0xff, 0xd8, 0xff, 0xd9]).unwrap();
    fs::write(root.join("assets/body.png"), b"\x89PNG\r\n\x1a\n").unwrap();
    root
}

fn temporary_picture_draft() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "wechat-oa-cli-picture-test-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(root.join("assets")).unwrap();
    fs::write(
        root.join("draft.json"),
        r#"{
            "schema_version": 1,
            "article_type": "newspic",
            "title": "Test picture post",
            "content_file": "content.txt",
            "image_files": ["assets/01.jpg", "assets/02.png"]
        }"#,
    )
    .unwrap();
    fs::write(root.join("content.txt"), "Two test pictures").unwrap();
    fs::write(root.join("assets/01.jpg"), [0xff, 0xd8, 0xff, 0xd9]).unwrap();
    fs::write(root.join("assets/02.png"), b"\x89PNG\r\n\x1a\n").unwrap();
    root
}

#[test]
fn schema_is_available_without_credentials() {
    let output = binary().args(["draft", "schema"]).output().unwrap();

    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value["oneOf"][0]["properties"]["article_type"]["const"],
        "news"
    );
    assert_eq!(
        value["oneOf"][1]["properties"]["article_type"]["const"],
        "newspic"
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn validates_a_local_bundle_without_credentials() {
    let root = temporary_draft();
    let output = binary()
        .args(["draft", "validate"])
        .arg(&root)
        .output()
        .unwrap();
    let _ = fs::remove_dir_all(&root);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["valid"], true);
    assert_eq!(value["article_type"], "news");
    assert_eq!(value["local_images"][0], "assets/body.png");
}

#[test]
fn validates_a_local_picture_bundle_without_credentials() {
    let root = temporary_picture_draft();
    let output = binary()
        .args(["draft", "validate"])
        .arg(&root)
        .output()
        .unwrap();
    let _ = fs::remove_dir_all(&root);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["valid"], true);
    assert_eq!(value["article_type"], "newspic");
    assert_eq!(value["images"][0], "assets/01.jpg");
    assert_eq!(value["images"][1], "assets/02.png");
    assert!(value.get("cover").is_none());
}

#[test]
fn rejects_a_picture_bundle_without_images() {
    let root = temporary_picture_draft();
    fs::write(
        root.join("draft.json"),
        r#"{
            "schema_version": 1,
            "article_type": "newspic",
            "title": "Empty picture post",
            "content_file": "content.txt",
            "image_files": []
        }"#,
    )
    .unwrap();
    let output = binary()
        .args(["draft", "validate"])
        .arg(&root)
        .output()
        .unwrap();
    let _ = fs::remove_dir_all(&root);

    assert!(!output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("between 1 and 20 images")
    );
}

#[test]
fn argument_errors_are_json_on_stderr() {
    let output = binary()
        .args(["draft", "list", "--count", "21"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(value["error"]["type"], "invalid_arguments");
}

#[test]
fn analytics_is_not_exposed_by_the_cli() {
    let output = binary().arg("--help").output().unwrap();

    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(!help.contains("analytics"));
}
