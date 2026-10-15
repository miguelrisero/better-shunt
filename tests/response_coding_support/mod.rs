#![allow(dead_code)]

use std::{
    io::{Read, Write},
    time::Duration,
};

use axum::{body::Body, http::Response};
use bytes::Bytes;
use futures_util::{stream, StreamExt};
use serde_json::json;
use shunt::{
    config::{Config, ModelConfig, UpstreamConfig},
    server,
};
use tokio::{sync::oneshot, task::JoinHandle};

pub const START: &str = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_test\",\"model\":\"native\",\"usage\":{\"input_tokens\":7,\"output_tokens\":0}}}\n\n";
pub const TINY: &str = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\n";
pub const END: &str = "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":3}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";

pub struct Server {
    pub url: String,
    task: JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub fn config(url: String, alias: bool) -> Config {
    let mut config = Config::default();
    config.providers.clear();
    config.upstreams = vec![serde_json::from_value::<UpstreamConfig>(json!({
        "name":"native", "kind":"anthropic", "base_url":url, "retry":{"max_retries":0}
    }))
    .unwrap()];
    config.server.default_provider = "native".into();
    config.server.sse_keepalive_seconds = 0;
    config.models = vec![ModelConfig {
        id: if alias { "alias" } else { "native" }.into(),
        display_name: None,
        upstream_model: Some([("native".into(), "native".into())].into_iter().collect()),
        router: None,
        stage_router: None,
        subagents: None,
    }];
    config
}

pub async fn gateway(config: Config) -> Server {
    let (app, _, _) = server::build_router(config).unwrap();
    serve(app).await
}

pub async fn serve(app: axum::Router) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Server {
        url: format!("http://{addr}"),
        task,
    }
}

pub async fn request(
    gateway: &Server,
    coding: &str,
    alias: bool,
    safeguards: bool,
    streaming: bool,
) -> reqwest::Response {
    let mut body = json!({"model": if alias { "alias" } else { "native" }, "max_tokens": 16, "stream": streaming, "messages": [{"role":"user", "content":"hello"}]});
    if safeguards {
        body["safeguards"] =
            json!([{"type":"dangerous_tool_use", "classifier_context":{"cwd":"/tmp"}}]);
    }
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!("{}/v1/messages", gateway.url))
        .header("accept-encoding", coding)
        .json(&body)
        .send()
        .await
        .unwrap()
}

enum Encoder {
    Gzip(flate2::write::GzEncoder<Vec<u8>>),
    Deflate(flate2::write::ZlibEncoder<Vec<u8>>),
    Zstd(zstd::stream::write::Encoder<'static, Vec<u8>>),
}

impl Encoder {
    fn new(coding: &str) -> Self {
        match coding {
            "gzip" => Self::Gzip(flate2::write::GzEncoder::new(
                Vec::new(),
                flate2::Compression::default(),
            )),
            "deflate" => Self::Deflate(flate2::write::ZlibEncoder::new(
                Vec::new(),
                flate2::Compression::default(),
            )),
            "zstd" => Self::Zstd(zstd::stream::write::Encoder::new(Vec::new(), 3).unwrap()),
            _ => panic!("unknown fixture coding"),
        }
    }

    fn push(&mut self, bytes: &[u8]) -> Vec<u8> {
        match self {
            Self::Gzip(encoder) => {
                encoder.write_all(bytes).unwrap();
                encoder.flush().unwrap();
                std::mem::take(encoder.get_mut())
            }
            Self::Deflate(encoder) => {
                encoder.write_all(bytes).unwrap();
                encoder.flush().unwrap();
                std::mem::take(encoder.get_mut())
            }
            Self::Zstd(encoder) => {
                encoder.write_all(bytes).unwrap();
                encoder.flush().unwrap();
                std::mem::take(encoder.get_mut())
            }
        }
    }

    fn finish(self) -> Vec<u8> {
        match self {
            Self::Gzip(encoder) => encoder.finish().unwrap(),
            Self::Deflate(encoder) => encoder.finish().unwrap(),
            Self::Zstd(encoder) => encoder.finish().unwrap(),
        }
    }
}

pub fn compressed(coding: &str, bytes: &[u8]) -> Vec<u8> {
    let mut encoder = Encoder::new(coding);
    let mut bytes = encoder.push(bytes);
    bytes.extend(encoder.finish());
    bytes
}

pub fn decoded(coding: &str, bytes: &[u8]) -> String {
    let mut output = String::new();
    match coding {
        "gzip" => {
            flate2::read::MultiGzDecoder::new(bytes)
                .read_to_string(&mut output)
                .unwrap();
        }
        "deflate" => {
            flate2::read::ZlibDecoder::new(bytes)
                .read_to_string(&mut output)
                .unwrap();
        }
        "zstd" => {
            output = String::from_utf8(zstd::stream::decode_all(bytes).unwrap()).unwrap();
        }
        _ => panic!("unknown fixture coding"),
    }
    output
}

/// A coding stream with event-level flushes, byte-split transport pieces, and
/// independent gates before each tiny event and before the terminal frames.
pub async fn gated(
    coding: &'static str,
    prefix: &'static str,
) -> (Server, Vec<oneshot::Sender<()>>) {
    let mut encoder = Encoder::new(coding);
    let mut pieces = vec![(encoder.push(prefix.as_bytes()), None)];
    let mut releases = Vec::new();
    for event in [TINY, TINY, END] {
        let (release, gate) = oneshot::channel();
        releases.push(release);
        pieces.push((encoder.push(event.as_bytes()), Some(gate)));
    }
    pieces.push((encoder.finish(), None));
    assert!(pieces.iter().map(|(bytes, _)| bytes.len()).sum::<usize>() < 64 * 1024);
    let pieces = std::sync::Arc::new(std::sync::Mutex::new(Some(pieces)));
    let app = axum::Router::new().route(
        "/v1/messages",
        axum::routing::post(move || {
            let pieces = pieces.lock().unwrap().take().unwrap();
            async move {
                let body =
                    stream::iter(pieces)
                        .then(|(bytes, gate)| async move {
                            if let Some(gate) = gate {
                                let _ = gate.await;
                            }
                            stream::iter(bytes.into_iter().map(|byte| {
                                Ok::<_, std::convert::Infallible>(Bytes::from(vec![byte]))
                            }))
                        })
                        .flatten();
                Response::builder()
                    .header("content-type", "text/event-stream")
                    .header("content-encoding", coding)
                    .body(Body::from_stream(body))
                    .unwrap()
            }
        }),
    );
    (serve(app).await, releases)
}

/// Client-side incremental decoding. The codec retains state across network
/// pieces; the caller can require a complete event while upstream remains gated.
pub struct ClientDecoder {
    kind: ClientDecode,
    pub text: String,
}

enum ClientDecode {
    Gzip(flate2::write::GzDecoder<Vec<u8>>),
    Deflate(flate2::write::ZlibDecoder<Vec<u8>>),
    Zstd(zstd::stream::write::Decoder<'static, Vec<u8>>),
}

impl ClientDecoder {
    pub fn new(coding: &str) -> Self {
        Self {
            kind: match coding {
                "gzip" => ClientDecode::Gzip(flate2::write::GzDecoder::new(Vec::new())),
                "deflate" => ClientDecode::Deflate(flate2::write::ZlibDecoder::new(Vec::new())),
                "zstd" => {
                    ClientDecode::Zstd(zstd::stream::write::Decoder::new(Vec::new()).unwrap())
                }
                _ => panic!("unknown client coding"),
            },
            text: String::new(),
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        let out = match &mut self.kind {
            ClientDecode::Gzip(decoder) => {
                decoder.write_all(bytes).unwrap();
                decoder.flush().unwrap();
                std::mem::take(decoder.get_mut())
            }
            ClientDecode::Deflate(decoder) => {
                decoder.write_all(bytes).unwrap();
                decoder.flush().unwrap();
                std::mem::take(decoder.get_mut())
            }
            ClientDecode::Zstd(decoder) => {
                decoder.write_all(bytes).unwrap();
                decoder.flush().unwrap();
                std::mem::take(decoder.get_mut())
            }
        };
        self.text.push_str(std::str::from_utf8(&out).unwrap());
    }

    pub async fn until(
        &mut self,
        response: &mut reqwest::Response,
        predicate: impl Fn(&str) -> bool,
    ) {
        tokio::time::timeout(Duration::from_secs(4), async {
            while !predicate(&self.text) {
                let bytes = response.chunk().await.unwrap().expect("body ended before complete event");
                self.push(&bytes);
            }
        }).await.unwrap_or_else(|error| panic!("decoded event must arrive before upstream gate opens: {error}; decoded so far: {:?}", self.text));
    }
}
