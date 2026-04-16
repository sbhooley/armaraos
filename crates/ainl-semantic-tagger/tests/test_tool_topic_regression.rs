use ainl_semantic_tagger::{infer_topic_tags, tag_tool_names, TagNamespace};

#[test]
fn tool_aliases_canonicalize_to_expected_values() {
    let tags = tag_tool_names(&[
        "shell_exec".to_string(),
        "python3".to_string(),
        "web_search".to_string(),
        "mcp_server_call".to_string(),
    ]);
    let vals: Vec<_> = tags.iter().map(|t| t.value.as_str()).collect();
    assert_eq!(vals, vec!["bash", "python_repl", "search_web", "mcp"]);
    assert!(tags.iter().all(|t| t.namespace == TagNamespace::Tool));
}

#[test]
fn unknown_tool_name_sanitizes_and_falls_back_confidence() {
    let tags = tag_tool_names(&["  Weird Tool@Name  ".to_string()]);
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].value, "weird_tool_name");
    assert!((tags[0].confidence - 0.5).abs() < f32::EPSILON);
}

#[test]
fn topic_inference_prefers_exact_token_confidence() {
    let exact = infer_topic_tags("rust cargo");
    let substring = infer_topic_tags("thrusting cargoes");

    let exact_rust = exact
        .iter()
        .find(|t| t.namespace == TagNamespace::Topic && t.value == "rust")
        .map(|t| t.confidence)
        .unwrap_or(0.0);
    let substring_rust = substring
        .iter()
        .find(|t| t.namespace == TagNamespace::Topic && t.value == "rust")
        .map(|t| t.confidence)
        .unwrap_or(0.0);

    assert!(exact_rust >= 0.85 - f32::EPSILON);
    assert!(substring_rust <= 0.70 + f32::EPSILON);
}
