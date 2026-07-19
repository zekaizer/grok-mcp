//! Live e2e evaluation (ignored by default).
//!
//! ```text
//! cargo test -p grok-server live_e2e_eval -- --ignored --nocapture
//! ```

#![cfg(test)]

use rmcp::handler::server::wrapper::Parameters;

use crate::GrokMcpServer;
use crate::tools::ask_grok::AskGrokArgs;
use crate::tools::auth_status::AuthStatusArgs;
use crate::tools::x_search::XSearchArgs;
use grok_client::{ClientConfig, GrokClient};

fn server() -> GrokMcpServer {
    let client = GrokClient::new(ClientConfig::from_env()).expect("client");
    GrokMcpServer::new(None, client)
}

#[tokio::test]
#[ignore = "live xAI network — e2e evaluation loop"]
async fn live_e2e_eval() {
    let s = server();

    // --- auth ---
    let auth = s
        .auth_status(Parameters(AuthStatusArgs {
            include_account_hints: true,
        }))
        .await
        .expect("auth_status");
    let auth = auth.0;
    eprintln!(
        "\n=== auth_status ===\n{}",
        serde_json::to_string_pretty(&auth).unwrap()
    );
    assert!(auth.ok);
    assert!(
        auth.authenticated,
        "need credentials: grok-mcp auth import/login"
    );

    // --- ask_grok (cheap baseline) ---
    let ask = s
        .ask_grok(Parameters(AskGrokArgs {
            prompt: "Reply with exactly one word: pong".into(),
            system: None,
            depth: Some("quick".into()),
            model: None,
            max_output_tokens: Some(64),
            timeout_secs: None,
            debug: None,
        }))
        .await
        .expect("ask_grok");
    let ask = ask.0;
    eprintln!(
        "\n=== ask_grok ===\n{}",
        serde_json::to_string_pretty(&ask).unwrap()
    );
    assert!(ask.ok);
    assert_eq!(ask.status, "completed");
    assert!(ask.text.as_ref().is_some_and(|t| !t.is_empty()));
    assert_eq!(ask.cost_hint.as_deref(), Some("low"));

    // --- x_search digest (scout) ---
    let dig = s
        .x_search(Parameters(XSearchArgs {
            query: "from:elonmusk since:2026-07-01 until:2026-07-02".into(),
            result: Some("digest".into()),
            depth: Some("quick".into()),
            model: None,
            max_items: Some(5),
            max_output_tokens: Some(1024),
            timeout_secs: Some(120),
            debug: None,
        }))
        .await
        .expect("x_search digest");
    let dig = dig.0;
    eprintln!(
        "\n=== x_search digest ===\n{}",
        serde_json::to_string_pretty(&dig).unwrap()
    );
    assert!(dig.ok);
    assert_eq!(dig.status, "completed");
    assert_eq!(dig.result_mode.as_deref(), Some("digest"));
    assert!(dig.digest.is_some(), "digest mode must return digest");
    assert!(
        dig.evidence_status.is_none(),
        "digest mode: no evidence_status"
    );
    assert!(dig.fidelity.is_none(), "digest mode: no fidelity block");

    // --- x_search evidence (full text path) ---
    let ev = s
        .x_search(Parameters(XSearchArgs {
            query: "from:elonmusk since:2026-07-01 until:2026-07-02".into(),
            result: Some("evidence".into()),
            depth: Some("standard".into()),
            model: None,
            max_items: Some(5),
            max_output_tokens: Some(4096),
            timeout_secs: Some(180),
            debug: None,
        }))
        .await
        .expect("x_search evidence must be Ok (empty is success)");
    let ev = ev.0;
    eprintln!(
        "\n=== x_search evidence ===\n{}",
        serde_json::to_string_pretty(&ev).unwrap()
    );
    assert!(ev.ok, "empty evidence must not hard-fail");
    assert_eq!(ev.status, "completed");
    assert_eq!(ev.result_mode.as_deref(), Some("evidence"));
    assert!(
        ev.evidence_status.is_some(),
        "evidence mode requires evidence_status"
    );
    let est = ev.evidence_status.as_deref().unwrap();
    assert!(
        matches!(est, "empty" | "partial" | "complete"),
        "unexpected evidence_status={est}"
    );
    assert!(
        ev.fidelity.is_some(),
        "fidelity block required for evidence"
    );
    assert!(
        ev.fidelity
            .as_ref()
            .unwrap()
            .guarantee
            .contains("best_effort"),
        "guarantee text"
    );
    let posts = ev.posts.as_ref().cloned().unwrap_or_default();
    if est == "empty" {
        assert!(posts.is_empty());
        assert!(
            ev.digest.is_some(),
            "empty evidence should still surface a digest summary"
        );
        eprintln!("NOTE: evidence empty for this query — graceful path OK");
    } else {
        assert!(!posts.is_empty());
        for p in &posts {
            assert!(!p.text.trim().is_empty(), "post text non-empty");
            // Heuristic: intentional short digests often end with …
            if p.text_complete {
                assert!(
                    !p.text.trim_end().ends_with('…') && !p.text.trim_end().ends_with("..."),
                    "text_complete=true but ellipsis: {}",
                    p.text
                );
            }
            eprintln!(
                "  post @{author} complete={c} len={n} url={u}",
                author = p.author,
                c = p.text_complete,
                n = p.text.chars().count(),
                u = p.url
            );
        }
    }

    // --- x_search impossible query → empty success ---
    let none = s
        .x_search(Parameters(XSearchArgs {
            query:
                "from:this_handle_should_not_exist_zzzxxyy_98765 since:2099-01-01 until:2099-01-02"
                    .into(),
            result: Some("evidence".into()),
            depth: Some("quick".into()),
            model: None,
            max_items: Some(3),
            max_output_tokens: Some(512),
            timeout_secs: Some(90),
            debug: None,
        }))
        .await
        .expect("impossible query must still succeed");
    let none = none.0;
    eprintln!(
        "\n=== x_search evidence (no matches expected) ===\n{}",
        serde_json::to_string_pretty(&none).unwrap()
    );
    assert!(none.ok);
    assert_eq!(none.evidence_status.as_deref(), Some("empty"));
    assert!(
        none.posts.as_ref().is_some_and(|p| p.is_empty())
            || none.posts.is_none()
            || none.posts.as_ref().unwrap().is_empty()
    );
    assert!(
        none.digest
            .as_ref()
            .map(|d| !d.summary.is_empty())
            .unwrap_or(false),
        "empty path should include digest summary for the host"
    );

    eprintln!("\n=== EVAL SUMMARY ===");
    eprintln!("auth: ok authenticated={}", auth.authenticated);
    eprintln!(
        "ask_grok: cost={:?} text_len={}",
        ask.cost_hint,
        ask.text.as_ref().map(|t| t.len()).unwrap_or(0)
    );
    eprintln!(
        "x_search digest: posts={} cost={:?}",
        dig.posts.as_ref().map(|p| p.len()).unwrap_or(0),
        dig.cost_hint
    );
    eprintln!(
        "x_search evidence: status={:?} posts={} cost={:?} truncated={:?}",
        ev.evidence_status,
        posts.len(),
        ev.cost_hint,
        ev.truncated
    );
    eprintln!(
        "x_search empty: status={:?} digest={:?}",
        none.evidence_status,
        none.digest.as_ref().map(|d| d.summary.clone())
    );
    eprintln!("PASS structural e2e checks");
}
