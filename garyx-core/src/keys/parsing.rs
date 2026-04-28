use super::{AGENT_PREFIX, ParsedSessionKey, SUBAGENT_SEGMENT, SessionKeyError};

/// Parse a hierarchical session key.
///
/// Format: `agent:{agentId}:{channel}:{surface}:{peerId}[:thread:{threadId}]`
pub fn parse_hierarchical_key(session_key: &str) -> Result<ParsedSessionKey, SessionKeyError> {
    if !session_key.starts_with(AGENT_PREFIX) {
        return Err(SessionKeyError::new(
            format!("Not a hierarchical key: {session_key}"),
            Some(session_key),
        ));
    }

    if session_key.contains(SUBAGENT_SEGMENT) {
        return parse_subagent_key(session_key);
    }

    let rest = &session_key[AGENT_PREFIX.len()..];
    let parts: Vec<&str> = rest.split(':').collect();
    if parts.len() < 4 {
        return Err(SessionKeyError::new(
            format!("Invalid hierarchical key format: {session_key}"),
            Some(session_key),
        ));
    }

    let thread_id = if parts.len() >= 6 && parts[4] == "thread" {
        Some(parts[5].to_owned())
    } else {
        None
    };

    Ok(ParsedSessionKey {
        agent_id: parts[0].to_owned(),
        session_type: parts[2].to_owned(),
        peer_id: parts[3].to_owned(),
        channel: Some(parts[1].to_owned()),
        surface: Some(parts[2].to_owned()),
        thread_id,
        is_subagent: false,
        subagent_name: None,
        raw_key: session_key.to_owned(),
    })
}

fn parse_subagent_key(session_key: &str) -> Result<ParsedSessionKey, SessionKeyError> {
    let idx = session_key.find(SUBAGENT_SEGMENT).ok_or_else(|| {
        SessionKeyError::new(
            format!("Invalid subagent key format: missing '{SUBAGENT_SEGMENT}' segment"),
            Some(session_key),
        )
    })?;
    let prefix = &session_key[..idx];
    let subagent_name = &session_key[idx + SUBAGENT_SEGMENT.len()..];
    let agent_id = &prefix[AGENT_PREFIX.len()..];

    Ok(ParsedSessionKey {
        agent_id: agent_id.to_owned(),
        session_type: "subagent".to_owned(),
        peer_id: subagent_name.to_owned(),
        channel: None,
        surface: None,
        thread_id: None,
        is_subagent: true,
        subagent_name: Some(subagent_name.to_owned()),
        raw_key: session_key.to_owned(),
    })
}
