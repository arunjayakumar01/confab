//! Resource-name shape checks. Client-supplied names end up in URLs sent to
//! Google with the client's own token, so they must stay inside the path
//! segments we expect (`spaces/{id}`, `spaces/{id}/messages/{id}`).

fn segment_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && s != "."
        && s != ".."
}

/// `spaces/{id}`
pub fn valid_space(name: &str) -> bool {
    match name.strip_prefix("spaces/") {
        Some(id) => segment_ok(id),
        None => false,
    }
}

/// `spaces/{id}/messages/{id}`
pub fn valid_message(name: &str) -> bool {
    let parts: Vec<&str> = name.split('/').collect();
    parts.len() == 4
        && parts[0] == "spaces"
        && parts[2] == "messages"
        && segment_ok(parts[1])
        && segment_ok(parts[3])
}

/// `spaces/{id}/threads/{id}`; `None` is always fine.
pub fn valid_thread(name: Option<&str>) -> bool {
    match name {
        None => true,
        Some(n) => {
            let parts: Vec<&str> = n.split('/').collect();
            parts.len() == 4
                && parts[0] == "spaces"
                && parts[2] == "threads"
                && segment_ok(parts[1])
                && segment_ok(parts[3])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes() {
        assert!(valid_space("spaces/AAAqX_1-2"));
        assert!(!valid_space("spaces/"));
        assert!(!valid_space("spaces/../v1/x"));
        assert!(!valid_space("spaces/a/b"));
        assert!(!valid_space("users/1"));
        assert!(valid_message("spaces/A/messages/B.C"));
        assert!(!valid_message("spaces/A/messages/"));
        assert!(!valid_message("spaces/A/messages/B/reactions/C"));
        assert!(valid_thread(None));
        assert!(valid_thread(Some("spaces/A/threads/T")));
        assert!(!valid_thread(Some("spaces/A/messages/T")));
    }
}
