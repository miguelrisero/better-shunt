#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::json;
use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};

mod common;
mod response_coding_support;
use response_coding_support::*;

#[tokio::test]
async fn auto_mode_streams_complete_tiny_events_before_the_final_delta_and_trailer() {
    let _env = common::env_lock().await;
    for coding in ["gzip", "deflate", "zstd"] {
        let (upstream, releases) = gated(coding, START).await;
        let gateway = gateway(config(upstream.url.clone(), true)).await;
        let mut response = request(&gateway, coding, true, true, true).await;
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["content-encoding"], coding);
        assert!(response
            .headers()
            .get_all("vary")
            .iter()
            .any(|v| v == "Accept-Encoding"));
        let mut decoder = ClientDecoder::new(coding);
        decoder
            .until(&mut response, |text| text.contains("\n\n"))
            .await;
        assert!(decoder.text.contains("\"model\":\"alias\""));
        assert!(!decoder.text.contains("message_delta"));
        let mut releases = releases.into_iter();
        for count in 1..=2 {
            releases.next().unwrap().send(()).unwrap();
            decoder
                .until(&mut response, |text| {
                    text.matches("event: content_block_delta").count() >= count
                        && text.ends_with("\n\n")
                })
                .await;
            assert!(!decoder.text.contains("message_delta"));
        }
        releases.next().unwrap().send(()).unwrap();
        decoder
            .until(&mut response, |text| {
                text.contains("event: message_stop") && text.ends_with("\n\n")
            })
            .await;
        assert!(decoder.text.contains("safeguard_results"));
        assert!(decoder.text.contains("dangerous_tool_use"));
        // Reading through EOF also validates the encoder trailer.
        while response.chunk().await.unwrap().is_some() {}
    }
}

#[tokio::test]
async fn idle_keepalive_is_inside_the_client_coding_stream() {
    let _env = common::env_lock().await;
    let (upstream, releases) = gated("gzip", START).await;
    let mut cfg = config(upstream.url.clone(), false);
    cfg.server.sse_keepalive_seconds = 1;
    let gateway = gateway(cfg).await;
    let mut response = request(&gateway, "gzip", false, false, true).await;
    let mut decoder = ClientDecoder::new("gzip");
    decoder
        .until(&mut response, |text| {
            text.contains("event: ping") && text.ends_with("\n\n")
        })
        .await;
    assert!(decoder.text.starts_with(START));
    for release in releases {
        release.send(()).unwrap();
    }
    decoder
        .until(&mut response, |text| text.contains("event: message_stop"))
        .await;
    while response.chunk().await.unwrap().is_some() {}
}

#[tokio::test]
async fn compressed_sse_without_safeguards_preserves_plaintext() {
    let _env = common::env_lock().await;
    let expected = format!("{START}{TINY}{END}");
    for coding in ["gzip", "deflate", "zstd"] {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-encoding", coding)
                    .set_body_raw(compressed(coding, expected.as_bytes()), "text/event-stream"),
            )
            .expect(1)
            .mount(&upstream)
            .await;
        let gateway = gateway(config(upstream.uri(), false)).await;
        let response = request(&gateway, coding, false, false, true).await;
        assert_eq!(response.headers()["content-encoding"], coding);
        assert_eq!(decoded(coding, &response.bytes().await.unwrap()), expected);
        upstream.verify().await;
    }
}

#[tokio::test]
async fn json_alias_rewrite_and_third_party_safeguards_see_decoded_json() {
    let _env = common::env_lock().await;
    for coding in ["gzip", "deflate", "zstd"] {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-encoding", coding)
                    .insert_header("request-id", "req_json")
                    .set_body_raw(
                        compressed(
                            coding,
                            br#"{"type":"message","model":"native","content":[]}"#,
                        ),
                        "application/json",
                    ),
            )
            .expect(1)
            .mount(&upstream)
            .await;
        let gateway = gateway(config(upstream.uri(), true)).await;
        let response = request(&gateway, coding, true, true, false).await;
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["request-id"], "req_json");
        assert_eq!(response.headers()["content-encoding"], coding);
        let value: serde_json::Value =
            serde_json::from_str(&decoded(coding, &response.bytes().await.unwrap())).unwrap();
        assert_eq!(value["model"], "alias");
        assert_eq!(value["safeguard_results"][0]["type"], "dangerous_tool_use");
    }
}

#[tokio::test]
async fn compressed_native_verdict_is_preserved() {
    let _env = common::env_lock().await;
    let upstream = MockServer::start().await;
    let verdict = json!([{"type":"dangerous_tool_use", "status":{"type":"available", "tool_uses":{"tool_a":{"type":"allowed"}}}}]);
    let event = format!(
        "event: message_delta\ndata: {}\n\n",
        json!({"type":"message_delta", "delta":{"safeguard_results":verdict}})
    );
    let body = format!("{START}{event}{END}");
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-encoding", "gzip")
                .set_body_raw(compressed("gzip", body.as_bytes()), "text/event-stream"),
        )
        .mount(&upstream)
        .await;
    let gateway = gateway(config(upstream.uri(), false)).await;
    let response = request(&gateway, "gzip", false, true, true).await;
    let text = decoded("gzip", &response.bytes().await.unwrap());
    assert!(text.contains(&event));
}

#[tokio::test]
async fn outbound_offers_drop_brotli_and_keep_supported_quality_values() {
    let _env = common::env_lock().await;
    for (offer, expected) in [
        (
            "gzip, deflate;q=0.5, br, zstd;q=0.7",
            Some("gzip, deflate;q=0.5, zstd;q=0.7"),
        ),
        ("identity", None),
        ("br", None),
    ] {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_raw("{}", "application/json"))
            .expect(1)
            .mount(&upstream)
            .await;
        let gateway = gateway(config(upstream.uri(), false)).await;
        let response = request(&gateway, offer, false, false, false).await;
        assert!(!response.headers().contains_key("content-encoding"));
        assert_eq!(response.text().await.unwrap(), "{}");
        let requests = upstream.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0]
                .headers
                .get("accept-encoding")
                .map(|v| v.to_str().unwrap()),
            expected
        );
    }
}

#[tokio::test]
async fn compressed_refusal_keeps_status_retry_after_and_error_envelope() {
    let _env = common::env_lock().await;
    let body = br#"{"type":"error","error":{"type":"rate_limit_error","message":"slow down"}}"#;
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("content-encoding", "gzip")
                .insert_header("retry-after", "17")
                .insert_header("request-id", "req_refusal")
                .set_body_raw(compressed("gzip", body), "application/json"),
        )
        .expect(1)
        .mount(&upstream)
        .await;
    let gateway = gateway(config(upstream.uri(), false)).await;
    let response = request(&gateway, "gzip", false, false, false).await;
    assert_eq!(response.status(), 429);
    assert_eq!(response.headers()["retry-after"], "17");
    assert_eq!(response.headers()["request-id"], "req_refusal");
    assert_eq!(
        decoded("gzip", &response.bytes().await.unwrap()).as_bytes(),
        body
    );
}

#[tokio::test]
async fn malformed_compressed_stream_ends_with_a_body_error() {
    let _env = common::env_lock().await;
    for coding in ["gzip", "deflate", "zstd"] {
        for bytes in [b"not a coding stream".to_vec(), {
            let mut bytes = compressed(coding, format!("{START}{TINY}").as_bytes());
            bytes.pop();
            bytes
        }] {
            let upstream = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-encoding", coding)
                        .set_body_raw(bytes, "text/event-stream"),
                )
                .mount(&upstream)
                .await;
            let gateway = gateway(config(upstream.uri(), false)).await;
            let client = reqwest::Client::builder().no_proxy().build().unwrap();
            let response = client.post(format!("{}/v1/messages", gateway.url))
                .header("accept-encoding", coding)
                .json(&json!({"model":"native","max_tokens":16,"stream":true,"messages":[{"role":"user","content":"hello"}]}))
                .send().await;
            if let Ok(response) = response {
                assert!(
                    response.bytes().await.is_err(),
                    "invalid stream must not complete cleanly: {coding}"
                );
            }
        }
    }
}
