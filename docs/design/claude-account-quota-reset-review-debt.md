# Claude Account Quota Reset Review Debt

Status: debt register discovered while validating #TASK-2574. The item below
is recorded for independent scheduling and is not part of the account quota
reset-time implementation.

## D1. Desktop smoke fixture can boot without an enabled agent

`npm run test:smoke` successfully builds, packages, installs, and launches the
desktop renderer against its isolated mock Gateway, but the current fixture can
reach the home screen with `No enabled agents`. The smoke script then times out
waiting for its synthetic `Smoke Thread` row during the `wait-thread-list`
stage.

#TASK-2574 changes only the Provider settings account-quota presentation and
does not touch agent availability, bootstrap, recent threads, or the smoke
Gateway. A follow-up should reproduce this on current `main` and align the mock
Gateway's agent/config bootstrap state with the production contract before the
thread-list assertions run.
