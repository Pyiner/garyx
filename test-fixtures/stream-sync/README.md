P-1 stream-sync fixtures.

These JSONL snippets are sanitized captures from live Garyx transcript and stream
frames. IDs, text, process IDs, and paths are synthetic placeholders so the public
repo does not contain personal data. The field shape, ordering, event names, and
seq/no-seq split mirror the captured runtime data.

Files:

- `transcript-with-tool.jsonl`: durable transcript records with gapless `seq`
  and a `tool_use` / `tool_result` pair.
- `stream-events-with-user-ack.jsonl`: chat WebSocket stream/control frames for
  a run where `user_ack` arrives before the local `stream_input` response.
- `stream-lifecycle.jsonl`: mixed lifecycle + committed events covering
  `run_start` -> `committed_message` -> `run_complete`.
