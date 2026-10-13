//! Non-thinking requests for known optional-thinking text translation routes.
//!
//! A Chat Completions URL alone does not establish support for extra fields.
//! Keep unknown gateways and mandatory-thinking models on their existing wire
//! contract. Never silently switch models or retry with thinking enabled.

use serde_json::{json, Value};

pub(super) fn disable(body: &mut Value, endpoint: &url::Url) {
    let model = body["model"].as_str().unwrap_or_default();
    let host = endpoint.host_str().unwrap_or_default();
    let field = match host {
        "api.deepseek.com" => Some(("thinking", json!({"type": "disabled"}))),
        "api.moonshot.cn" | "api.moonshot.ai" if named(model, &["kimi-k2.5"]) => {
            Some(("thinking", json!({"type": "disabled"})))
        }
        host if alibaba_host(host) && alibaba_optional_thinking(model) => {
            Some(("enable_thinking", json!(false)))
        }
        "api.siliconflow.cn" | "api.siliconflow.com" if siliconflow_optional_thinking(model) => {
            Some(("enable_thinking", json!(false)))
        }
        "api.openai.com" if openai_optional_thinking(model) => {
            Some(("reasoning_effort", json!("none")))
        }
        "generativelanguage.googleapis.com" if gemini_optional_thinking(model) => {
            Some(("reasoning_effort", json!("none")))
        }
        "openrouter.ai" if openrouter_optional_thinking(model) => {
            Some(("reasoning", json!({"enabled": false})))
        }
        _ => None,
    };
    if let Some((key, value)) = field {
        body[key] = value;
    }
}

fn alibaba_host(host: &str) -> bool {
    matches!(
        host,
        "dashscope.aliyuncs.com" | "dashscope-intl.aliyuncs.com" | "dashscope-us.aliyuncs.com"
    ) || host.ends_with(".maas.aliyuncs.com")
}

// Allow aliases and dated snapshots, not similarly named model variants:
// gpt-5.2-pro and qwen3-max-thinking must not inherit a disable parameter.
fn named(model: &str, aliases: &[&str]) -> bool {
    aliases.iter().any(|alias| {
        model == *alias
            || model.strip_prefix(alias).is_some_and(|suffix| {
                let bytes = suffix.as_bytes();
                bytes.len() == 11
                    && bytes[0] == b'-'
                    && bytes[5] == b'-'
                    && bytes[8] == b'-'
                    && bytes
                        .iter()
                        .enumerate()
                        .all(|(index, byte)| matches!(index, 0 | 5 | 8) || byte.is_ascii_digit())
            })
    })
}

fn alibaba_optional_thinking(model: &str) -> bool {
    // This early Max snapshot and the preview are mandatory-thinking models.
    model != "qwen3.7-max-2026-05-17"
        && named(
            model,
            &[
                "qwen-plus",
                "qwen-plus-latest",
                "qwen-flash",
                "qwen-turbo",
                "qwen3-max",
                "qwen3.5-plus",
                "qwen3.5-flash",
                "qwen3.6-plus",
                "qwen3.6-flash",
                "qwen3.7-plus",
                "qwen3.7-flash",
                "qwen3.7-max",
                "qwen3.8-max",
                "qwen3.8-flash",
                "qwen3-235b-a22b",
                "qwen3-32b",
                "qwen3-30b-a3b",
                "qwen3-14b",
                "qwen3-8b",
                "qwen3.5-397b-a17b",
                "qwen3.5-122b-a10b",
                "qwen3.5-27b",
                "qwen3.5-35b-a3b",
                "qwen3.6-35b-a3b",
                "qwen3.8-27b",
                "deepseek-v3.1",
                "deepseek-v3.2",
                "deepseek-v3.2-exp",
                "deepseek-v4-pro",
                "deepseek-v4-flash",
                "deepseek-v4.1-flash",
            ],
        )
}

fn siliconflow_optional_thinking(model: &str) -> bool {
    let model = model.strip_prefix("Pro/").unwrap_or(model);
    matches!(
        model,
        "Qwen/Qwen3-235B-A22B"
            | "Qwen/Qwen3-32B"
            | "Qwen/Qwen3-30B-A3B"
            | "Qwen/Qwen3-14B"
            | "Qwen/Qwen3-8B"
            | "Qwen/Qwen3-4B"
            | "deepseek-ai/DeepSeek-V3.1"
            | "deepseek-ai/DeepSeek-V3.2"
    )
}

fn openai_optional_thinking(model: &str) -> bool {
    named(
        model,
        &[
            "gpt-5.1",
            "gpt-5.2",
            "gpt-5.4",
            "gpt-5.4-mini",
            "gpt-5.4-nano",
            "gpt-5.5",
            "gpt-5.6",
            "gpt-5.6-sol",
            "gpt-5.6-terra",
            "gpt-5.6-luna",
            "gpt-6-sol",
            "gpt-6-luna",
        ],
    )
}

fn gemini_optional_thinking(model: &str) -> bool {
    matches!(
        model,
        "gemini-2.5-flash"
            | "gemini-2.5-flash-lite"
            | "gemini-2.5-flash-preview-04-17"
            | "gemini-2.5-flash-preview-05-20"
            | "gemini-2.5-flash-lite-preview-06-17"
    )
}

fn openrouter_optional_thinking(model: &str) -> bool {
    match model.split_once('/') {
        Some(("openai", model)) => openai_optional_thinking(model),
        Some(("qwen", model)) => alibaba_optional_thinking(model),
        Some(("deepseek", model)) => named(
            model,
            &[
                "deepseek-v3.1",
                "deepseek-v3.2",
                "deepseek-v4-pro",
                "deepseek-v4-flash",
            ],
        ),
        Some(("moonshotai", model)) => named(model, &["kimi-k2.5"]),
        Some(("google", model)) => gemini_optional_thinking(model),
        _ => false,
    }
}
