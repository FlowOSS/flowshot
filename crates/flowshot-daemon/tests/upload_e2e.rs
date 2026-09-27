//! Todo 38 flow 9: the upload wiremock e2e through the REAL executor
//! post-capture stage (`run_post`), plus the unconfigured-provider
//! degradation (the CLI-side exit-2 hint path is asserted live in the
//! flow-09 script; this covers the daemon-side `Deferred` mapping).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use flowshot_core::geometry::LogicalRect;
use flowshot_daemon::execute::post::run_post;
use flowshot_daemon::execute::{ExecCtx, ExecOutcome};
use flowshot_daemon::request::CaptureRequest;
use flowshot_ui::{Completion, CompletionKind, ExportedImage};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn imgur_success_json() -> serde_json::Value {
    serde_json::json!({
        "data": {
            "link": "https://i.imgur.com/abc123.png",
            "deletehash": "del_abc123"
        },
        "success": true,
        "status": 200
    })
}

struct TempHome(PathBuf);

impl TempHome {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("flowshot-t38-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn config_path(&self) -> PathBuf {
        self.0.join("flowshot.toml")
    }

    fn write_config(&self, client_id: &str) {
        std::fs::write(
            self.config_path(),
            format!(
                r#"config_version = 2

[capture]
save_last_region = false

[save]
actions = []

[upload]
provider = "imgur"
client_id = "{client_id}"
copy_url = false

[daemon]
notifications = false
"#
            ),
        )
        .unwrap();
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn completion() -> Completion {
    Completion {
        kind: CompletionKind::Accept,
        selection: LogicalRect::from_raw(0.0, 0.0, 2.0, 2.0),
        image: ExportedImage {
            width: 2,
            height: 2,
            rgba: vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
            ],
        },
    }
}

#[tokio::test]
async fn upload_runs_through_the_executor_against_the_stub() {
    // Given a wiremock Imgur and a configured client id injected via the
    // ExecCtx base-url override,
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/3/image"))
        .respond_with(ResponseTemplate::new(200).set_body_json(imgur_success_json()))
        .mount(&server)
        .await;
    let home = TempHome::new("upload");
    home.write_config("test-client-id");
    let ctx = ExecCtx {
        config_path: Some(home.config_path()),
        state: None,
        notifier: None,
        upload_base_url: Some(server.uri()),
    };
    let (config, config_path) = ctx.load_config();
    let request = CaptureRequest {
        upload: true,
        ..CaptureRequest::default()
    };
    // When the executor's post-capture stage runs,
    let outcome = run_post(
        completion(),
        &request,
        &config,
        config_path.as_deref(),
        &ctx,
    )
    .await
    .unwrap();
    // Then the report carries the stub's URL and the history file recorded
    // it next to the QA config.
    let ExecOutcome::Done(report) = outcome else {
        panic!("expected a completed post-capture run");
    };
    assert!(
        report.outcomes.iter().any(|outcome| matches!(
            outcome,
            flowshot_actions::clipboard::ActionOutcome::Uploaded { url }
                if url == "https://i.imgur.com/abc123.png"
        )),
        "outcomes: {:?}",
        report.outcomes
    );
    let history = home.0.join("upload-history.json");
    assert!(history.exists(), "upload history was not recorded");
    let contents = std::fs::read_to_string(&history).unwrap();
    assert!(contents.contains("abc123"), "history: {contents}");
}

#[tokio::test]
async fn unconfigured_provider_defers_with_a_warning() {
    // Given an EMPTY client id (the Amendment-#3 no-freeloading default),
    let server = MockServer::start().await;
    let home = TempHome::new("upload-unconfigured");
    home.write_config("");
    let ctx = ExecCtx {
        config_path: Some(home.config_path()),
        state: None,
        notifier: None,
        upload_base_url: Some(server.uri()),
    };
    let (config, config_path) = ctx.load_config();
    let request = CaptureRequest {
        upload: true,
        ..CaptureRequest::default()
    };
    // When the post-capture stage runs,
    let outcome = run_post(
        completion(),
        &request,
        &config,
        config_path.as_deref(),
        &ctx,
    )
    .await
    .unwrap();
    // Then the upload is Deferred (never a silent success, never a panic);
    // the CLI-side usage rejection (exit 2 + settings hint) fires BEFORE
    // this stage for direct invocations (flow-09 script asserts it live).
    let ExecOutcome::Done(report) = outcome else {
        panic!("expected a completed post-capture run");
    };
    assert!(
        report.outcomes.iter().any(|outcome| matches!(
            outcome,
            flowshot_actions::clipboard::ActionOutcome::Deferred(_)
        )),
        "outcomes: {:?}",
        report.outcomes
    );
}
