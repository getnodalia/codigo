use super::*;
use crate::chat::{build, read_reply};
use crate::prompts::{self, Task};
use serde_json::json;

fn cfg(p: Provider) -> Config {
    Config::new(p)
}

#[test]
fn anthropic_requests_carry_the_key_only_in_their_header() {
    let r = build(
        &cfg(Provider::Anthropic),
        Some("sk-ant-secret"),
        "sys",
        &[Message::user("hi")],
    )
    .expect("request");
    assert_eq!(r.url, "https://api.anthropic.com/v1/messages");
    assert!(r
        .headers
        .iter()
        .any(|(n, v)| *n == "x-api-key" && v == "sk-ant-secret"));
    assert!(!r.body.to_string().contains("sk-ant-secret"));
    assert_eq!(r.body["system"], "sys");
    assert_eq!(r.body["messages"][0]["role"], "user");
    assert_eq!(r.body["model"], "claude-sonnet-5-5");
}

#[test]
fn openai_style_requests_put_the_system_text_first() {
    let r = build(
        &cfg(Provider::OpenAi),
        Some("sk-openai"),
        "sys",
        &[
            Message::user("a"),
            Message::assistant("b"),
            Message::user("c"),
        ],
    )
    .expect("request");
    assert_eq!(r.url, "https://api.openai.com/v1/chat/completions");
    assert_eq!(r.body["messages"][0]["role"], "system");
    assert_eq!(r.body["messages"].as_array().map(Vec::len), Some(4));
    assert!(r
        .headers
        .iter()
        .any(|(n, v)| *n == "Authorization" && v == "Bearer sk-openai"));
}

#[test]
fn a_local_model_needs_no_key_and_a_hosted_one_needs_it() {
    assert!(build(&cfg(Provider::Ollama), None, "s", &[Message::user("x")]).is_ok());
    let missing = build(&cfg(Provider::Anthropic), None, "s", &[Message::user("x")]);
    assert!(matches!(missing, Err(Error::Setup(_))));
    assert!(build(&cfg(Provider::Anthropic), Some("k"), "s", &[]).is_err());
}

#[test]
fn a_key_is_never_sent_over_plain_http_except_to_this_computer() {
    let mut c = cfg(Provider::Custom);
    c.model = "m".into();
    for bad in [
        "http://example.com/v1",
        "ftp://example.com",
        "http://localhost.evil.com/v1",
        "https://user:pw@example.com",
        "https://a b",
        "",
    ] {
        c.base_url = bad.into();
        assert!(c.base().is_err(), "{bad}");
    }
    for good in [
        "https://example.com/v1/",
        "http://localhost:11434/v1",
        "http://127.0.0.1:8080",
        "http://[::1]:1234/v1",
    ] {
        c.base_url = good.into();
        assert!(c.base().is_ok(), "{good}");
    }
    c.base_url = "https://example.com/v1/".into();
    assert_eq!(c.base().as_deref(), Ok("https://example.com/v1"));
    c.model = "  ".into();
    assert!(c.model().is_err());
}

#[test]
fn a_long_conversation_drops_its_oldest_messages_and_opens_with_the_person() {
    let big = "x".repeat(150_000);
    let msgs = [
        Message::user("first"),
        Message::assistant(big.clone()),
        Message::user(big),
        Message::assistant("short"),
        Message::user("last"),
    ];
    let r = build(&cfg(Provider::Ollama), None, "s", &msgs).expect("request");
    let sent = r.body["messages"].as_array().expect("list");
    assert_eq!(
        sent.last().map(|m| m["content"].clone()),
        Some(json!("last"))
    );
    assert_eq!(sent[1]["role"], "user");
    assert!(sent.len() < 6);
}

#[test]
fn replies_are_read_from_either_shape_and_empty_ones_are_errors() {
    let a = json!({"content": [{"type": "text", "text": "he"}, {"type": "text", "text": "llo"}]});
    assert_eq!(read_reply(Provider::Anthropic, &a).as_deref(), Ok("hello"));
    let o = json!({"choices": [{"message": {"role": "assistant", "content": "hi"}}]});
    assert_eq!(read_reply(Provider::OpenAi, &o).as_deref(), Ok("hi"));
    for empty in [json!({}), json!({"choices": []}), json!({"content": []})] {
        assert!(read_reply(Provider::OpenAi, &empty).is_err());
        assert!(read_reply(Provider::Anthropic, &empty).is_err());
    }
}

#[test]
fn providers_round_trip_through_settings_and_keys_are_cleaned() {
    for p in Provider::ALL {
        assert_eq!(Provider::from_id(p.id()), p);
    }
    assert_eq!(Provider::from_id("nonsense"), Provider::Anthropic);
    assert_eq!(secret::clean("  sk-1  ").as_deref(), Some("sk-1"));
    assert_eq!(secret::clean("a b"), None);
    assert_eq!(secret::clean("a\nb"), None);
    assert_eq!(secret::clean(""), None);
}

#[test]
fn code_is_fenced_safely_and_the_replacement_is_found_in_an_answer() {
    let q = prompts::ask(Task::Fix, "src/a.rs", "rust", "let x = \"```\";");
    assert!(q.contains("````rust"), "{q}");
    assert!(q.contains("File: src/a.rs"));
    assert_eq!(prompts::clip("héllo", 2), "h");
    assert_eq!(prompts::clip("abc", 10), "abc");

    let reply = "Here you go:\n```rust\nfn a() {}\n\nfn b() {}\n```\nDone.";
    assert_eq!(
        prompts::first_code_block(reply).as_deref(),
        Some("fn a() {}\n\nfn b() {}")
    );
    let cut = "```py\nprint(1)";
    assert_eq!(prompts::first_code_block(cut).as_deref(), Some("print(1)"));
    assert_eq!(prompts::first_code_block("no code"), None);
    let nested = "````md\n```rust\nx\n```\n````\n";
    assert_eq!(
        prompts::first_code_block(nested).as_deref(),
        Some("```rust\nx\n```")
    );
}
