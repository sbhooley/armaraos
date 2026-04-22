//! Format a compact “do not repeat” block from [`super::search::FailureRecallHit`] rows.

use crate::search::FailureRecallHit;

/// Build markdown-flavored text for injection into a system or recall segment.
#[must_use]
pub fn format_failure_prevention_block(title: &str, hits: &[FailureRecallHit]) -> String {
    if hits.is_empty() {
        return String::new();
    }
    let mut s = String::new();
    s.push_str("### ");
    s.push_str(title);
    s.push_str("\n\n");
    s.push_str("Recent similar failures in memory (do not repeat):\n\n");
    for (i, h) in hits.iter().take(8).enumerate() {
        s.push_str(&format!("{}. ", i + 1));
        if let Some(t) = &h.tool_name {
            s.push('`');
            s.push_str(t);
            s.push_str("` ");
        }
        s.push_str(&h.message);
        s.push_str(" _(source: ");
        s.push_str(&h.source);
        s.push_str(")_\n");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::FailureRecallHit;
    use uuid::Uuid;

    #[test]
    fn format_includes_message_and_source() {
        let hits = vec![FailureRecallHit {
            id: Uuid::nil(),
            source: "loop_guard:block".into(),
            message: "repeated tool".into(),
            tool_name: Some("x".into()),
            score: 1.0,
        }];
        let t = format_failure_prevention_block("Caution", &hits);
        assert!(t.contains("Caution"));
        assert!(t.contains("repeated tool"));
        assert!(t.contains("loop_guard"));
    }
}
