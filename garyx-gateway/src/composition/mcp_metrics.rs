use serde::Serialize;
use std::collections::HashMap;

/// Aggregated MCP tool duration stats.
#[derive(Debug, Clone, Serialize, Default)]
pub struct McpToolDurationStat {
    pub tool: String,
    pub count: u64,
    pub min_ms: u64,
    pub max_ms: u64,
    pub avg_ms: f64,
    pub total_ms: u128,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct McpToolCallCount {
    pub tool: String,
    pub status: String,
    pub value: u64,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct McpToolMetricsSnapshot {
    pub mcp_tool_calls_total: Vec<McpToolCallCount>,
    pub mcp_tool_duration_ms: Vec<McpToolDurationStat>,
}

#[derive(Default)]
struct ToolDurationAccumulator {
    count: u64,
    min_ms: u64,
    max_ms: u64,
    total_ms: u128,
}

impl ToolDurationAccumulator {
    fn record(&mut self, duration_ms: u64) {
        if self.count == 0 {
            self.min_ms = duration_ms;
            self.max_ms = duration_ms;
        } else {
            self.min_ms = self.min_ms.min(duration_ms);
            self.max_ms = self.max_ms.max(duration_ms);
        }
        self.count += 1;
        self.total_ms += duration_ms as u128;
    }

    fn avg_ms(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.total_ms as f64 / self.count as f64
        }
    }
}

/// In-memory MCP metrics store keyed by tool and status.
#[derive(Default)]
pub struct McpToolMetrics {
    call_counts: std::sync::Mutex<HashMap<(String, String), u64>>,
    duration_stats: std::sync::Mutex<HashMap<String, ToolDurationAccumulator>>,
}

impl McpToolMetrics {
    pub fn record_call(&self, tool: &str, status: &str, duration_ms: u64) {
        if let Ok(mut calls) = self.call_counts.lock() {
            let key = (tool.to_owned(), status.to_owned());
            *calls.entry(key).or_insert(0) += 1;
        }
        if let Ok(mut durations) = self.duration_stats.lock() {
            durations
                .entry(tool.to_owned())
                .or_default()
                .record(duration_ms);
        }
    }

    pub fn snapshot(&self) -> McpToolMetricsSnapshot {
        let mut call_entries = if let Ok(calls) = self.call_counts.lock() {
            calls
                .iter()
                .map(|((tool, status), value)| McpToolCallCount {
                    tool: tool.clone(),
                    status: status.clone(),
                    value: *value,
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        call_entries.sort_by(|a, b| a.tool.cmp(&b.tool).then_with(|| a.status.cmp(&b.status)));

        let mut duration_entries = if let Ok(durations) = self.duration_stats.lock() {
            durations
                .iter()
                .map(|(tool, stat)| McpToolDurationStat {
                    tool: tool.clone(),
                    count: stat.count,
                    min_ms: stat.min_ms,
                    max_ms: stat.max_ms,
                    avg_ms: stat.avg_ms(),
                    total_ms: stat.total_ms,
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        duration_entries.sort_by(|a, b| a.tool.cmp(&b.tool));

        McpToolMetricsSnapshot {
            mcp_tool_calls_total: call_entries,
            mcp_tool_duration_ms: duration_entries,
        }
    }
}
