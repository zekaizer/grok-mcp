//! Live smoke (ignored by default):
//! `cargo test -p grok-server --test live_ask -- --ignored --nocapture`

#[tokio::test]
#[ignore = "live xAI network"]
async fn ask_grok_two_plus_two() {
    use grok_auth::load_valid_record;
    use grok_client::{
        ClientConfig, CreateResponseRequest, GrokClient, ReasoningParam, extract_output_text,
    };
    use serde_json::json;

    let client = GrokClient::new(ClientConfig::from_env()).expect("client");
    let rec = load_valid_record(client.http(), None)
        .await
        .expect("import credentials first: grok-mcp auth import");
    let body = client
        .create_response(
            &rec.access_token,
            &CreateResponseRequest {
                model: client.resolve_model(None),
                input: json!("In one short sentence: what is 2+2?"),
                instructions: Some(
                    "Answer concisely. Prefer short paragraphs or bullets. No preamble.".into(),
                ),
                tools: None,
                max_output_tokens: Some(128),
                reasoning: Some(ReasoningParam {
                    effort: "low".into(),
                }),
                stream: false,
            },
        )
        .await
        .expect("responses api");
    let text = extract_output_text(&body);
    eprintln!("text={text}");
    assert!(
        text.contains('4') || text.to_ascii_lowercase().contains("four"),
        "unexpected: {text}"
    );
}
