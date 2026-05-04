//! Route resolution for mapping messages to agents.
//!
//! Configuration-driven agent selection with:
//! - Channel, account, and peer-level bindings
//! - Priority-based rule matching
//! - Fallback to default agent

use garyx_models::config::GaryxConfig;
use regex::Regex;
use std::collections::{HashMap, hash_map::Entry};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Criteria for matching a route.
#[derive(Debug, Clone, Default)]
pub struct RouteMatch {
    /// Match specific channel (telegram, etc.)
    pub channel: Option<String>,
    /// Match specific account ID (or "*" for any)
    pub account_id: Option<String>,
    /// Match peer type: dm, group, channel
    pub peer_kind: Option<String>,
    /// Match specific peer ID
    pub peer_id: Option<String>,
    /// Regex pattern to match peer ID
    pub peer_pattern: Option<String>,
    /// Guild/server ID
    pub guild_id: Option<String>,
    /// Slack team ID
    pub team_id: Option<String>,
}

/// A routing rule binding messages to an agent.
#[derive(Debug, Clone)]
pub struct RouteBinding {
    pub match_criteria: RouteMatch,
    pub agent_id: String,
    /// Higher priority rules match first.
    pub priority: i64,
}

/// Result of route resolution.
#[derive(Debug, Clone)]
pub struct ResolvedRoute {
    /// The agent to handle this message.
    pub agent_id: String,
    /// The rule that matched (if any).
    pub matched_rule: Option<RouteBinding>,
    /// Whether this is the default agent.
    pub is_default: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RouteResolutionContext<'a> {
    pub channel: &'a str,
    pub account_id: Option<&'a str>,
    pub peer_kind: Option<&'a str>,
    pub peer_id: Option<&'a str>,
    pub guild_id: Option<&'a str>,
    pub team_id: Option<&'a str>,
}

// ---------------------------------------------------------------------------
// RouteResolver
// ---------------------------------------------------------------------------

/// Resolves which agent should handle a message.
///
/// Resolution priority:
/// 1. Peer-specific bindings (exact peer match)
/// 2. Guild/Team bindings
/// 3. Account bindings (specific bot account)
/// 4. Channel bindings (all accounts on channel)
/// 5. Default agent
pub struct RouteResolver {
    config: GaryxConfig,
    bindings: Vec<RouteBinding>,
    default_agent: String,
    // Cache both valid and invalid patterns to avoid repeated compile attempts.
    pattern_cache: HashMap<String, Option<Regex>>,
}

impl RouteResolver {
    pub fn new(config: GaryxConfig) -> Self {
        let mut resolver = Self {
            config,
            bindings: Vec::new(),
            default_agent: "main".to_owned(),
            pattern_cache: HashMap::new(),
        };
        resolver.load_bindings();
        resolver
    }

    /// Update configuration (for hot reload).
    pub fn update_config(&mut self, config: GaryxConfig) {
        self.config = config;
        self.pattern_cache.clear();
        self.load_bindings();
    }

    /// Get the default agent ID.
    pub fn get_default_agent(&self) -> &str {
        &self.default_agent
    }

    /// List all routing bindings.
    pub fn list_bindings(&self) -> Vec<RouteBinding> {
        self.bindings.clone()
    }

    /// Resolve which agent should handle a message.
    pub fn resolve(
        &mut self,
        channel: &str,
        account_id: Option<&str>,
        peer_kind: Option<&str>,
        peer_id: Option<&str>,
        guild_id: Option<&str>,
        team_id: Option<&str>,
    ) -> ResolvedRoute {
        self.resolve_for(RouteResolutionContext {
            channel,
            account_id,
            peer_kind,
            peer_id,
            guild_id,
            team_id,
        })
    }

    pub fn resolve_for(&mut self, context: RouteResolutionContext<'_>) -> ResolvedRoute {
        for i in 0..self.bindings.len() {
            if Self::matches_binding(
                &self.bindings[i].match_criteria,
                &mut self.pattern_cache,
                context,
            ) {
                return ResolvedRoute {
                    agent_id: self.bindings[i].agent_id.clone(),
                    matched_rule: Some(self.bindings[i].clone()),
                    is_default: false,
                };
            }
        }

        ResolvedRoute {
            agent_id: self.default_agent.clone(),
            matched_rule: None,
            is_default: true,
        }
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    fn load_bindings(&mut self) {
        self.bindings.clear();
        self.default_agent = "main".to_owned();

        let agents_config = &self.config.agents;
        if agents_config.is_empty() {
            return;
        }

        // Get bindings list
        if let Some(bindings_val) = agents_config.get("bindings")
            && let Some(bindings_arr) = bindings_val.as_array()
        {
            for (idx, binding_data) in bindings_arr.iter().enumerate() {
                if let Some(binding) = self.parse_binding(binding_data, idx) {
                    self.bindings.push(binding);
                }
            }
        }

        // Get default agent
        if let Some(default_val) = agents_config.get("default")
            && let Some(default_str) = default_val.as_str()
            && !default_str.is_empty()
        {
            self.default_agent = default_str.to_owned();
        }

        // Sort by priority (higher first)
        self.bindings.sort_by(|a, b| b.priority.cmp(&a.priority));
    }

    fn parse_binding(&self, data: &serde_json::Value, index: usize) -> Option<RouteBinding> {
        let obj = data.as_object()?;

        let agent_id = obj
            .get("agentId")
            .or_else(|| obj.get("agent_id"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())?;

        let match_data = obj
            .get("match")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();

        let peer_data = match_data.get("peer").and_then(|v| v.as_object());

        let peer_kind = peer_data
            .and_then(|p| p.get("kind"))
            .and_then(|v| v.as_str())
            .map(String::from);
        let peer_id = peer_data
            .and_then(|p| p.get("id"))
            .and_then(|v| v.as_str())
            .map(String::from);
        let peer_pattern = peer_data
            .and_then(|p| p.get("pattern"))
            .and_then(|v| v.as_str())
            .map(String::from);

        let route_match = RouteMatch {
            channel: match_data
                .get("channel")
                .and_then(|v| v.as_str())
                .map(String::from),
            account_id: match_data
                .get("accountId")
                .or_else(|| match_data.get("account_id"))
                .and_then(|v| v.as_str())
                .map(String::from),
            peer_kind,
            peer_id,
            peer_pattern,
            guild_id: match_data
                .get("guildId")
                .or_else(|| match_data.get("guild_id"))
                .and_then(|v| v.as_str())
                .map(String::from),
            team_id: match_data
                .get("teamId")
                .or_else(|| match_data.get("team_id"))
                .and_then(|v| v.as_str())
                .map(String::from),
        };

        let priority = obj
            .get("priority")
            .and_then(|v| v.as_i64())
            .unwrap_or(100 - index as i64);

        Some(RouteBinding {
            match_criteria: route_match,
            agent_id: agent_id.to_owned(),
            priority,
        })
    }

    fn matches_binding(
        m: &RouteMatch,
        pattern_cache: &mut HashMap<String, Option<Regex>>,
        context: RouteResolutionContext<'_>,
    ) -> bool {
        // Channel must match if specified
        if let Some(ref mc) = m.channel
            && mc != context.channel
        {
            return false;
        }

        // Account must match if specified (or * for any)
        if let Some(ref ma) = m.account_id
            && ma != "*"
            && context.account_id != Some(ma.as_str())
        {
            return false;
        }

        // Peer kind must match if specified
        if let Some(ref mk) = m.peer_kind
            && context.peer_kind != Some(mk.as_str())
        {
            return false;
        }

        // Peer ID - exact match
        if let Some(ref mp) = m.peer_id
            && context.peer_id != Some(mp.as_str())
        {
            return false;
        }

        // Peer ID - pattern match
        if let Some(ref pattern_str) = m.peer_pattern {
            let Some(pid) = context.peer_id else {
                return false;
            };
            if !Self::pattern_matches(pattern_cache, pattern_str, pid) {
                return false;
            }
        }

        // Guild ID must match if specified
        if let Some(ref mg) = m.guild_id
            && context.guild_id != Some(mg.as_str())
        {
            return false;
        }

        // Team ID must match if specified
        if let Some(ref mt) = m.team_id
            && context.team_id != Some(mt.as_str())
        {
            return false;
        }

        true
    }

    fn pattern_matches(
        pattern_cache: &mut HashMap<String, Option<Regex>>,
        pattern: &str,
        value: &str,
    ) -> bool {
        let cached = match pattern_cache.entry(pattern.to_owned()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let compiled = Regex::new(pattern)
                    .map_err(
                        |e| tracing::warn!(pattern, error = %e, "invalid regex in route pattern"),
                    )
                    .ok();
                entry.insert(compiled)
            }
        };
        cached.as_ref().is_some_and(|re| re.is_match(value))
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests;
