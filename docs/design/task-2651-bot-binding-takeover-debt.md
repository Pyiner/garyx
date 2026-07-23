# Task 2651 Adjacent Bot-Binding Takeover Debt

Status: follow-up product and protocol work; explicitly outside `#TASK-2651`.

`#TASK-2651` repairs main-endpoint resolution so opening an already-bound bot
does not accidentally enter a new-thread draft. It does not change the
single-holder binding mutation itself.

Today, sending the first message from an intentional bot draft may move the
bot endpoint from its existing thread to the newly created thread. The
single-holder invariant is preserved, but the prior holder can lose its
binding without an explicit takeover confirmation.

A separate task should decide and specify:

- whether a draft targeting an endpoint with an existing holder must require
  explicit confirmation before its first dispatch;
- which gateway preflight contract reports the current holder and prevents a
  stale confirmation from moving a newer binding;
- how Mac and mobile present the affected existing thread and the takeover
  consequence;
- which audit or recovery affordance is required after a confirmed move.

Use synthetic bot, endpoint, and thread identities in all fixtures.
