use ainl_semantic_tagger::{
    infer_topic_tags, tag_turn, TagNamespace, PREFERENCE_BREVITY, TONE_INFORMAL,
};

#[test]
fn tag_turn_covers_preferences_tools_and_topics() {
    let tags = tag_turn(
        "keep it short — I need help with cargo and rustc",
        Some("Sure."),
        &["file_read".into()],
    );
    let canonical: Vec<String> = tags.iter().map(|t| t.to_canonical_string()).collect();
    assert!(canonical.iter().any(|c| c == PREFERENCE_BREVITY));
    assert!(tags.iter().any(|t| t.namespace == TagNamespace::Topic && t.value == "rust"));
    assert!(tags.iter().any(|t| t.namespace == TagNamespace::Tool && t.value == "file_read"));
}

#[test]
fn topic_tags_dedupe_slug() {
    let tags = infer_topic_tags("rust rust rust cargo");
    assert_eq!(tags.iter().filter(|t| t.value == "rust").count(), 1);
}

#[test]
fn informal_tone_on_slang() {
    let tags = tag_turn("yo gonna wanna lol yeah", None, &[]);
    assert!(tags.iter().any(|t| {
        t.namespace == TagNamespace::Tone && t.to_canonical_string() == TONE_INFORMAL
    }));
}
