P-1 stream-sync fixtures.

These JSONL snippets are sanitized captures from live Garyx committed transcript
frames. IDs, text, process IDs, and paths are synthetic placeholders so the public
repo does not contain personal data. Content and run-state frames use the single
`committed_message` shape; local request responses such as `stream_input` remain
unseqed control acknowledgements.

Files:

- `transcript-with-tool.jsonl`: durable transcript records with gapless `seq`
  and a `tool_use` / `tool_result` pair.
- `stream-events-with-user-ack.jsonl`: committed content/control frames for a
  run where committed `user_ack` arrives before the local `stream_input`
  response.
- `stream-lifecycle.jsonl`: committed content/control events covering
  `run_start` -> content -> `run_complete`.
