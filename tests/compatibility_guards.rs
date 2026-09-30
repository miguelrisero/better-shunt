//! Public regression fixtures: no credentials, network, or business tool data.
use serde_json::{json, Value};
use shunt::{
    config::ResponsesFlavor,
    model::{inbound_responses::chat_request, responses::translate_request_value},
    routing::{AdapterKind, Route},
};

const EMAIL: &str = r"^(?!\.)(?!.*\.\.)([A-Za-z0-9_'+\-\.]*)[A-Za-z0-9_+-]@([A-Za-z0-9][A-Za-z0-9\-]*\.)+[A-Za-z]{2,}$";

fn schema() -> Value {
    json!({"type":"object", "properties": {
        "email":{"type":"string", "format":"email", "pattern":EMAIL},
        "code":{"type":"string", "pattern":"^[0-9]{6}$"},
        "nested":{"anyOf":[{"pattern":r"\p{Cc}"},{"pattern":"^ok$"}]}
    }, "required":["email"], "additionalProperties":false,
    "default":{"pattern":"(?=literal)","empty":[],"nullable":null}})
}

fn assert_schema(actual: &Value) {
    let mut expected = schema();
    expected["properties"]["email"]
        .as_object_mut()
        .unwrap()
        .remove("pattern");
    expected["properties"]["nested"]["anyOf"][0]
        .as_object_mut()
        .unwrap()
        .remove("pattern");
    assert_eq!(
        *actual, expected,
        "only incompatible schema keywords may change"
    );
}

fn translate(request: &Value, native: bool) -> Value {
    let route = Route {
        provider: "codex".into(),
        adapter: AdapterKind::Responses,
        model: "gpt-6-astra".into(),
        upstream_model: "gpt-6-astra".into(),
        effort: None,
        service_tier: None,
    };
    translate_request_value(
        request,
        &route,
        ResponsesFlavor::Chatgpt,
        native,
        None,
        true,
    )
}

#[test]
fn neon_email_is_sanitized_on_eager_and_revealed_tool_paths() {
    for native in [false, true] {
        let request = json!({"model":"gpt-6-astra", "max_tokens":256,
            "messages":[{"role":"user","content":"Public email fixture."}],
            "tools":[{"name":"email_echo","input_schema":schema()}]});
        let original = request.clone();
        let output = translate(&request, native);
        assert_schema(&output["tools"][0]["parameters"]);
        assert_eq!(output["tools"][0]["strict"], false);
        assert_eq!(request, original);

        let request = json!({"model":"gpt-6-astra", "max_tokens":256,
        "messages":[
            {"role":"assistant","content":[{"type":"tool_use","id":"search_1","name":"ToolSearch","input":{"query":"email"}}]},
            {"role":"user","content":[{"type":"tool_result","tool_use_id":"search_1","content":[{"type":"tool_reference","tool_name":"email_echo"}]}]}
        ], "tools":[
            {"name":"ToolSearch","input_schema":{"type":"object","properties":{"query":{"type":"string"}}}},
            {"name":"email_echo","defer_loading":true,"input_schema":schema()}
        ]});
        let original = request.clone();
        let output = translate(&request, native);
        let tools = if native {
            output["input"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["type"] == "tool_search_output")
                .unwrap()["tools"]
                .as_array()
                .unwrap()
        } else {
            output["tools"].as_array().unwrap()
        };
        let tool = tools.iter().find(|v| v["name"] == "email_echo").unwrap();
        assert_schema(&tool["parameters"]);
        assert_eq!(tool["strict"], false);
        assert_eq!(request, original);
    }
}

#[test]
fn inbound_chat_sanitizes_schema_without_mutating_request_or_arguments() {
    let request = json!({"model":"fixture", "input":[
        {"role":"user","content":"Public fixture"},
        {"type":"function_call","name":"email_echo","call_id":"call_1", "arguments":r#"{"empty":[],"nullable":null}"#},
        {"type":"function_call_output","call_id":"call_1","output":"ok"}
    ], "tools":[{"type":"function","name":"email_echo", "strict":false, "parameters":schema()}]});
    let original = request.clone();
    let output = chat_request::translate_request(&request, "fixture-upstream").unwrap();
    assert_schema(&output["tools"][0]["function"]["parameters"]);
    assert_eq!(output["tools"][0]["function"]["strict"], false);
    assert_eq!(
        output["messages"][1]["tool_calls"][0]["function"]["arguments"],
        r#"{"empty":[],"nullable":null}"#
    );
    assert_eq!(request, original);
}

#[test]
fn conservative_regex_filter_preserves_literals_and_rejects_incompatible_syntax() {
    for (pattern, keep) in [
        (r"a(?=b)", false),
        (r"a(?!b)", false),
        (r"(?<=a)b", false),
        (r"(?<!a)b", false),
        (r"(a)\1", false),
        (r"\p{Cc}", false),
        (r"[", false),
        (r"a{3,2}", false),
        (r"^\(\?=x\)$", true),
        (r"[(?=!<)]", true),
        (r"[](?=!<]", true),
        (r"[^](?=!<]", true),
        (r"(?:a|b)+", true),
        (r"^[a-z]+$", true),
    ] {
        let request = json!({"model":"gpt-6-astra", "messages":[{"role":"user","content":"fixture"}],
            "tools":[{"name":"fixture", "input_schema":{"type":"object", "properties":{"value":{"type":"string","pattern":pattern}}}}]});
        let output = translate(&request, false);
        assert_eq!(
            output["tools"][0]["parameters"]["properties"]["value"]
                .get("pattern")
                .is_some(),
            keep,
            "pattern {pattern:?}"
        );
    }
}
