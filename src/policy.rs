//! Which parts of an MCP server's surface a client may see.
//!
//! A category that is absent from the policy is unguarded; a category that is
//! present is an allowlist, so anything the server adds later stays hidden
//! until it is named here.

use serde::Deserialize;
use std::path::Path;

/// The three guarded MCP surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Prompt,
    Tool,
    Resource,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Prompt => "prompt",
            Kind::Tool => "tool",
            Kind::Resource => "resource",
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    pub prompts: Option<Vec<String>>,
    pub tools: Option<Vec<String>>,
    pub resources: Option<Vec<String>>,
}

impl Policy {
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    pub fn load(path: &Path) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        Self::from_json(&text).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{}: {e}", path.display()),
            )
        })
    }

    fn list(&self, kind: Kind) -> Option<&[String]> {
        match kind {
            Kind::Prompt => self.prompts.as_deref(),
            Kind::Tool => self.tools.as_deref(),
            Kind::Resource => self.resources.as_deref(),
        }
    }

    /// True when the category is unguarded, empty, or the name is listed.
    pub fn allows(&self, kind: Kind, name: &str) -> bool {
        match self.list(kind) {
            None => true,
            Some(patterns) => patterns.iter().any(|p| matches(p, name)),
        }
    }

    /// True when the category is guarded and can never match anything.
    pub fn forbids_everything(&self, kind: Kind) -> bool {
        matches!(self.list(kind), Some(patterns) if patterns.is_empty())
    }

    /// True when the category is guarded at all.
    pub fn guards(&self, kind: Kind) -> bool {
        self.list(kind).is_some()
    }
}

/// Whole-value glob: `*` matches any run of characters, everything else is
/// literal.
fn matches(pattern: &str, value: &str) -> bool {
    if !pattern.contains('*') {
        return pattern == value;
    }

    let pattern: Vec<char> = pattern.chars().collect();
    let value: Vec<char> = value.chars().collect();
    let (mut p, mut v) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;

    while v < value.len() {
        if p < pattern.len() && pattern[p] == value[v] {
            p += 1;
            v += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, v));
            p += 1;
        } else if let Some((sp, sv)) = star {
            p = sp + 1;
            v = sv + 1;
            star = Some((sp, sv + 1));
        } else {
            return false;
        }
    }

    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}
