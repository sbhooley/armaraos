//! Topic keyword tagging.

use crate::tag::{SemanticTag, TagNamespace};
use std::collections::HashMap;

struct TopicDict {
    slug: &'static str,
    keywords: &'static [&'static str],
}

const TOPICS: &[TopicDict] = &[
    TopicDict {
        slug: "rust",
        keywords: &[
            "rust",
            "cargo",
            "crate",
            "borrow checker",
            "tokio",
            "async fn",
            "trait",
            "lifetime",
            "rustc",
            "clippy",
            "serde",
        ],
    },
    TopicDict {
        slug: "trading",
        keywords: &[
            "trading",
            "backtest",
            "strategy",
            "alpha",
            "equity",
            "futures",
            "options",
            "portfolio",
            "candle",
            "orderbook",
            "bid",
            "ask",
            "market data",
            "gru",
            "lstm",
            "quant",
        ],
    },
    TopicDict {
        slug: "ai_agents",
        keywords: &[
            "agent",
            "ainl",
            "armaraos",
            "llm",
            "prompt",
            "orchestration",
            "delegation",
            "tool call",
            "memory",
            "persona",
        ],
    },
    TopicDict {
        slug: "graph",
        keywords: &[
            "graph",
            "node",
            "edge",
            "graph store",
            "sqlite",
            "episodic",
            "semantic node",
            "procedural",
            "persona node",
        ],
    },
    TopicDict {
        slug: "debugging",
        keywords: &[
            "debug",
            "panic",
            "error",
            "stack trace",
            "breakpoint",
            "reproduce",
            "minimal example",
            "repro",
        ],
    },
    TopicDict {
        slug: "infrastructure",
        keywords: &[
            "docker",
            "kubernetes",
            "k8s",
            "cloudflare",
            "workers",
            "aws",
            "deploy",
            "ci",
            "github actions",
            "pipeline",
        ],
    },
    TopicDict {
        slug: "gaming",
        keywords: &[
            "minecraft",
            "roblox",
            "starfield",
            "no man's sky",
            "server",
            "plugin",
            "papermc",
            "mod",
            "game",
        ],
    },
    TopicDict {
        slug: "personalization",
        keywords: &[
            "persona",
            "personalization",
            "user preference",
            "adapt",
            "behavior",
            "remember",
            "tailor",
        ],
    },
    TopicDict {
        slug: "tooling",
        keywords: &[
            "cli",
            "shell",
            "bash",
            "python",
            "repl",
            "script",
            "automation",
            "mcp",
            "tool",
        ],
    },
    TopicDict {
        slug: "memory",
        keywords: &[
            "memory",
            "recall",
            "retrieval",
            "store",
            "persist",
            "graph memory",
            "ainl-memory",
        ],
    },
];

fn keyword_match_confidence(lower: &str, kw: &str) -> Option<f32> {
    let kl = kw.to_lowercase();
    if kl.is_empty() {
        return None;
    }
    if kl.contains(char::is_whitespace) {
        return if lower.contains(kl.as_str()) {
            Some(0.85)
        } else {
            None
        };
    }
    let is_token = lower.split(|c: char| !c.is_alphanumeric()).any(|t| t == kl);
    if is_token {
        return Some(0.85);
    }
    if lower.contains(kl.as_str()) {
        return Some(0.70);
    }
    None
}

/// Deterministic keyword tagging for broad topics. One tag per topic slug; confidence is the max
/// across that slug's keyword hits (`0.85` exact / phrase, `0.70` substring for single-token keys).
pub fn infer_topic_tags(text: &str) -> Vec<SemanticTag> {
    let lower = text.to_lowercase();
    let mut best: HashMap<&'static str, f32> = HashMap::new();

    for topic in TOPICS {
        for kw in topic.keywords {
            if let Some(c) = keyword_match_confidence(&lower, kw) {
                best.entry(topic.slug)
                    .and_modify(|v| *v = v.max(c))
                    .or_insert(c);
            }
        }
    }

    best.into_iter()
        .map(|(slug, confidence)| SemanticTag {
            namespace: TagNamespace::Topic,
            value: slug.to_string(),
            confidence,
        })
        .collect()
}
