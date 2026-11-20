//! The gateway itself: two pure line transforms, one for each direction.

use crate::policy::{Kind, Policy};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write;

/// What the caller should do with a line read from the client.
pub enum Client {
    /// Relay this line to the server.
    Forward(String),
    /// Do not relay; answer the client with this line. An empty string means
    /// every reply was already collected through [`Guard::take_outbox`].
    Block(String),
}

enum Gate {
    Forward(Value),
    Blocked(Value),
}

pub struct Guard {
    policy: Policy,
    /// Request id key -> method, for responses that must be rewritten.
    pending: HashMap<String, String>,
    /// Replies produced while handling a batch.
    outbox: Vec<String>,
    audit: Option<std::fs::File>,
    server: String,
}

impl Guard {
    pub fn new(policy: Policy) -> Self {
        Self {
            policy,
            pending: HashMap::new(),
            outbox: Vec::new(),
            audit: None,
            server: String::new(),
        }
    }

    /// Attach an audit sink and the server name recorded in it.
    pub fn with_audit(mut self, server: impl Into<String>, file: Option<std::fs::File>) -> Self {
        self.server = server.into();
        self.audit = file;
        self
    }

    /// Replies a client still owes for blocked batch members. Drain after
    /// every [`Guard::client`] call.
    pub fn take_outbox(&mut self) -> Vec<String> {
        std::mem::take(&mut self.outbox)
    }

    pub fn client(&mut self, line: &str) -> Client {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return Client::Forward(line.to_owned());
        };
        match value {
            Value::Array(items) => {
                let mut kept = Vec::with_capacity(items.len());
                let mut blocked = 0usize;
                for item in items {
                    match self.gate_client(item) {
                        Gate::Forward(item) => kept.push(item),
                        Gate::Blocked(reply) => {
                            blocked += 1;
                            self.outbox.push(reply.to_string());
                        }
                    }
                }
                if blocked == 0 {
                    Client::Forward(line.to_owned())
                } else if kept.is_empty() {
                    Client::Block(String::new())
                } else {
                    Client::Forward(Value::Array(kept).to_string())
                }
            }
            other => match self.gate_client(other) {
                Gate::Forward(_) => Client::Forward(line.to_owned()),
                Gate::Blocked(reply) => Client::Block(reply.to_string()),
            },
        }
    }

    pub fn server(&mut self, line: &str) -> Option<String> {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return Some(line.to_owned());
        };
        match value {
            Value::Array(items) => {
                let mut changed = false;
                let rewritten: Vec<Value> = items
                    .into_iter()
                    .map(|item| {
                        let (value, item_changed) = self.rewrite_server(item);
                        changed |= item_changed;
                        value
                    })
                    .collect();
                if changed {
                    Some(Value::Array(rewritten).to_string())
                } else {
                    Some(line.to_owned())
                }
            }
            other => {
                let (value, changed) = self.rewrite_server(other);
                if changed {
                    Some(value.to_string())
                } else {
                    Some(line.to_owned())
                }
            }
        }
    }

    fn gate_client(&mut self, value: Value) -> Gate {
        let Some(method) = value.get("method").and_then(Value::as_str).map(str::to_owned) else {
            // A response from the client, not a request.
            return Gate::Forward(value);
        };

        let guarded = match method.as_str() {
            "prompts/get" => Some((Kind::Prompt, "name")),
            "tools/call" => Some((Kind::Tool, "name")),
            "resources/read" | "resources/subscribe" | "resources/unsubscribe" => {
                Some((Kind::Resource, "uri"))
            }
            _ => None,
        };

        if let Some((kind, field)) = guarded {
            let name = value
                .get("params")
                .and_then(|params| params.get(field))
                .and_then(Value::as_str);
            if let Some(name) = name {
                if !self.policy.allows(kind, name) {
                    self.record(&method, name, "block");
                    let id = value.get("id").cloned().unwrap_or(Value::Null);
                    return Gate::Blocked(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {
                            "code": -32602,
                            "message": format!("{name} is not available (blocked by mcp-guard)"),
                        },
                    }));
                }
            }
        }

        if let Some(id) = value.get("id") {
            if !id.is_null() {
                if let Some(key) = id_key(id) {
                    self.pending.insert(key, method);
                }
            }
        }
        Gate::Forward(value)
    }

    fn rewrite_server(&mut self, mut value: Value) -> (Value, bool) {
        if value.get("method").is_some() {
            // A request or notification started by the server.
            return (value, false);
        }

        let request = value
            .get("id")
            .and_then(id_key)
            .and_then(|key| self.pending.remove(&key));
        let Some(method) = request else {
            return (value, false);
        };

        if method == "initialize" {
            let Some(capabilities) = value
                .get_mut("result")
                .and_then(|result| result.get_mut("capabilities"))
                .and_then(Value::as_object_mut)
            else {
                return (value, false);
            };
            let mut changed = false;
            for (kind, key) in [
                (Kind::Prompt, "prompts"),
                (Kind::Tool, "tools"),
                (Kind::Resource, "resources"),
            ] {
                if self.policy.forbids_everything(kind) && capabilities.remove(key).is_some() {
                    self.record("initialize", key, "capability-removed");
                    changed = true;
                }
            }
            return (value, changed);
        }

        let (kind, key, field) = match method.as_str() {
            "prompts/list" => (Kind::Prompt, "prompts", "name"),
            "tools/list" => (Kind::Tool, "tools", "name"),
            "resources/list" => (Kind::Resource, "resources", "name"),
            "resources/templates/list" => (Kind::Resource, "resourceTemplates", "uriTemplate"),
            _ => return (value, false),
        };

        if !self.policy.guards(kind) {
            return (value, false);
        }

        let Some(items) = value
            .get_mut("result")
            .and_then(|result| result.get_mut(key))
            .and_then(Value::as_array_mut)
        else {
            return (value, false);
        };

        let before = items.len();
        items.retain(|item| match item.get(field).and_then(Value::as_str) {
            Some(name) => self.policy.allows(kind, name),
            None => true,
        });
        let kept = items.len();
        let changed = kept != before;
        if changed {
            self.record(&method, &format!("{kept}/{before} kept"), "filter");
        }
        (value, changed)
    }

    fn record(&mut self, method: &str, name: &str, decision: &str) {
        let Some(file) = self.audit.as_mut() else {
            return;
        };
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default();
        let line = json!({
            "ts": ts,
            "server": self.server,
            "method": method,
            "name": name,
            "decision": decision,
        });
        let _ = writeln!(file, "{line}");
    }
}

/// Canonical key for a JSON-RPC id, so `1` and `1.0` name the same request.
fn id_key(id: &Value) -> Option<String> {
    match id {
        Value::String(text) => Some(format!("s:{text}")),
        Value::Number(number) => match number.as_i64() {
            Some(integer) => Some(format!("n:{integer}")),
            None => number.as_f64().map(|float| format!("n:{float}")),
        },
        Value::Bool(flag) => Some(format!("b:{flag}")),
        Value::Null => None,
        other => Some(format!("j:{other}")),
    }
}
