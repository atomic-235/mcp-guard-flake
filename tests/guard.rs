//! Tests written from the requirement: a stdio MCP gateway that only exposes
//! the surface named by its policy, and that refuses blocked names by value
//! without forwarding the request.

use mcp_guard::{Client, Guard};

fn guard(json: &str) -> Guard {
    Guard::new(serde_json::from_str(json).expect("policy parses"))
}

fn json(line: &str) -> serde_json::Value {
    serde_json::from_str(line).expect("valid json")
}

fn forwarded(outcome: Client) -> String {
    match outcome {
        Client::Forward(line) => line,
        Client::Block(line) => panic!("expected forward, got block: {line}"),
    }
}

fn blocked(outcome: Client) -> String {
    match outcome {
        Client::Block(line) => line,
        Client::Forward(line) => panic!("expected block, got forward: {line}"),
    }
}

/// A policy naming exactly one prompt hides every other prompt from
/// `prompts/list`.
#[test]
fn prompts_list_is_filtered_to_the_allowlist() {
    let mut g = guard(r#"{"prompts":["uncensored-research"]}"#);

    g.client(r#"{"jsonrpc":"2.0","id":1,"method":"prompts/list"}"#);
    let out = g
        .server(
            r#"{"jsonrpc":"2.0","id":1,"result":{"prompts":[{"name":"uncensored-research"},{"name":"nsfw-creative-writing"},{"name":"image-style-explorer"}]}}"#,
        )
        .expect("response is relayed");

    let names: Vec<String> = json(&out)["result"]["prompts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, vec!["uncensored-research"]);
}

/// A blocked prompt cannot be fetched even if the client asks for it by name.
#[test]
fn prompts_get_for_a_hidden_prompt_is_answered_locally() {
    let mut g = guard(r#"{"prompts":["uncensored-research"]}"#);

    let out = blocked(g.client(
        r#"{"jsonrpc":"2.0","id":9,"method":"prompts/get","params":{"name":"nsfw-creative-writing"}}"#,
    ));

    let reply = json(&out);
    assert_eq!(reply["jsonrpc"], "2.0");
    assert_eq!(reply["id"], 9);
    assert_eq!(reply["error"]["code"], -32602);
    assert!(reply["error"]["message"]
        .as_str()
        .unwrap()
        .contains("nsfw-creative-writing"));
    assert!(reply.get("result").is_none());
}

/// An allowed prompt is relayed to the server untouched.
#[test]
fn prompts_get_for_an_allowed_prompt_is_forwarded() {
    let mut g = guard(r#"{"prompts":["uncensored-research"]}"#);
    let line = r#"{"jsonrpc":"2.0","id":3,"method":"prompts/get","params":{"name":"uncensored-research"}}"#;

    assert_eq!(forwarded(g.client(line)), line);
}

/// With no prompt allowed, the server must not advertise the capability,
/// because a client that sees `prompts` in `initialize` will call
/// `prompts/list` and fail against an emptied list.
#[test]
fn initialize_drops_the_prompts_capability_when_no_prompt_is_allowed() {
    let mut g = guard(r#"{"prompts":[],"tools":["*"]}"#);

    g.client(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{}}"#);
    let out = g
        .server(
            r#"{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{"listChanged":true},"prompts":{"listChanged":true},"resources":{}},"serverInfo":{"name":"x","version":"1"}}}"#,
        )
        .expect("initialize result is relayed");

    let caps = &json(&out)["result"]["capabilities"];
    assert!(caps.get("prompts").is_none(), "prompts capability must be removed");
    assert!(caps.get("tools").is_some(), "unrelated capabilities stay");
    assert!(caps.get("resources").is_some(), "unrelated capabilities stay");
}

/// A non-empty prompt allowlist keeps the capability so the filtered list is
/// still discoverable.
#[test]
fn initialize_keeps_the_prompts_capability_when_a_prompt_is_allowed() {
    let mut g = guard(r#"{"prompts":["uncensored-research"]}"#);

    g.client(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{}}"#);
    let out = g
        .server(
            r#"{"jsonrpc":"2.0","id":0,"result":{"capabilities":{"prompts":{"listChanged":true}}}}"#,
        )
        .expect("initialize result is relayed");

    assert!(json(&out)["result"]["capabilities"].get("prompts").is_some());
}

/// The same allowlist machinery applies to tools: the list is filtered and a
/// call to a hidden tool is refused locally.
#[test]
fn tools_are_filtered_and_blocked_by_the_same_policy() {
    let mut g = guard(r#"{"tools":["venice_web_search"]}"#);

    g.client(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
    let out = g
        .server(
            r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"venice_web_search"},{"name":"venice_video_generate"}]}}"#,
        )
        .unwrap();
    assert_eq!(json(&out)["result"]["tools"].as_array().unwrap().len(), 1);

    let out2 = blocked(g.client(
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"venice_video_generate","arguments":{}}}"#,
    ));
    assert_eq!(json(&out2)["error"]["code"], -32602);
    assert_eq!(json(&out2)["id"], 4);
}

/// Lines that are not JSON-RPC are relayed byte for byte, so a server that
/// writes logs on stdout keeps working.
#[test]
fn non_json_lines_pass_through_unchanged() {
    let mut g = guard(r#"{"prompts":["uncensored-research"]}"#);
    let line = "not json at all\twith tabs";

    assert_eq!(forwarded(g.client(line)), line);
    assert_eq!(g.server(line).unwrap(), line);
}

/// Requests outside the guarded surface, notifications, and server-initiated
/// requests are relayed untouched.
#[test]
fn unguarded_traffic_passes_through() {
    let mut g = guard(r#"{"prompts":["uncensored-research"]}"#);

    let notification = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    assert_eq!(forwarded(g.client(notification)), notification);

    let other = r#"{"jsonrpc":"2.0","id":5,"method":"resources/subscribe","params":{"uri":"venice://models"}}"#;
    assert_eq!(forwarded(g.client(other)), other);

    let server_notification = r#"{"jsonrpc":"2.0","method":"notifications/message","params":{"level":"info"}}"#;
    assert_eq!(g.server(server_notification).unwrap(), server_notification);

    let sampling = r#"{"jsonrpc":"2.0","id":77,"method":"sampling/createMessage","params":{}}"#;
    assert_eq!(g.server(sampling).unwrap(), sampling);
}

/// Responses are matched to their request by id value, so an unrelated
/// response with a different id is never rewritten.
#[test]
fn responses_are_matched_by_request_id() {
    let mut g = guard(r#"{"prompts":["uncensored-research"]}"#);

    g.client(r#"{"jsonrpc":"2.0","id":"abc","method":"prompts/list"}"#);
    let unrelated = r#"{"jsonrpc":"2.0","id":"other","result":{"prompts":[{"name":"nsfw-creative-writing"}]}}"#;
    assert_eq!(g.server(unrelated).unwrap(), unrelated);

    let out = g
        .server(r#"{"jsonrpc":"2.0","id":"abc","result":{"prompts":[{"name":"nsfw-creative-writing"}]}}"#)
        .unwrap();
    assert_eq!(json(&out)["result"]["prompts"].as_array().unwrap().len(), 0);
}

/// Resources are part of the same guarded surface.
#[test]
fn resources_are_guarded_by_uri_pattern() {
    let mut g = guard(r#"{"resources":["venice://styles"]}"#);

    let allowed = r#"{"jsonrpc":"2.0","id":11,"method":"resources/read","params":{"uri":"venice://styles"}}"#;
    assert_eq!(forwarded(g.client(allowed)), allowed);

    let denied = g.client(r#"{"jsonrpc":"2.0","id":12,"method":"resources/read","params":{"uri":"venice://characters"}}"#);
    assert_eq!(json(&blocked(denied))["error"]["code"], -32602);
}

/// A JSON-RPC batch is transformed element by element rather than dropped.
#[test]
fn batches_are_transformed_elementwise() {
    let mut g = guard(r#"{"prompts":["uncensored-research"]}"#);

    g.client(r#"[{"jsonrpc":"2.0","id":1,"method":"prompts/list"},{"jsonrpc":"2.0","id":2,"method":"tools/list"}]"#);
    let out = g
        .server(
            r#"[{"jsonrpc":"2.0","id":1,"result":{"prompts":[{"name":"nsfw-creative-writing"},{"name":"uncensored-research"}]}},{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"a"}]}}]"#,
        )
        .unwrap();

    let batch = json(&out);
    assert_eq!(batch[0]["result"]["prompts"].as_array().unwrap().len(), 1);
    assert_eq!(batch[1]["result"]["tools"].as_array().unwrap().len(), 1);
}

/// Omitting a category means "no restriction for that category"; an empty
/// array means "nothing of that category".
#[test]
fn absent_category_allows_everything() {
    let mut g = guard(r#"{"prompts":["uncensored-research"]}"#);

    g.client(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#);
    let out = g
        .server(r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[{"name":"a"},{"name":"b"}]}}"#)
        .unwrap();
    assert_eq!(json(&out)["result"]["tools"].as_array().unwrap().len(), 2);
}

/// Patterns support a trailing or leading `*`, which is how a broad but not
/// blanket policy is written.
#[test]
fn wildcard_patterns_match_prefixes_and_suffixes() {
    let mut g = guard(r#"{"tools":["venice_*","read"]}"#);

    for allowed in ["venice_web_search", "read"] {
        assert_eq!(
            forwarded(g.client(&format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"{allowed}"}}}}"#
            ))),
            format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"{allowed}"}}}}"#
            )
        );
    }

    assert_eq!(
        json(&blocked(g.client(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"edit"}}"#
        )))["error"]["code"],
        -32602
    );
}
