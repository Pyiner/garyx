# Gateway Runtime

- Code changes do not affect the running gateway until the binary is built,
  installed, and the managed gateway is restarted.
- On macOS, do not treat a matching hash as sufficient after copying a locally
  built `garyx` binary into a launchd-managed path such as
  `/opt/homebrew/bin/garyx`.
- Clear removable target-file xattrs, ad-hoc re-sign the installed file with
  the stable identifier `com.garyx.gateway`, or use:

```bash
bash scripts/codesign-macos-cli.sh <path-to-garyx>
```

- Verify the installed binary executes before restarting; otherwise
  launchd/AMFI may kill it with `OS_REASON_CODESIGNING`.
- `com.apple.provenance` can be inherited or protected on Homebrew paths even
  when `xattr -d` returns success, so do not rely on xattr output alone.
- For local macOS gateway development, prefer `scripts/install-local-cli.sh`
  after source changes.
- Release archives, `install.sh`, `garyx update`, and desktop `build:rust`
  should all preserve the same CLI identifier so directory authorization is not
  re-requested just because a new binary was installed.
- `install.sh` installs the signed release binary as-is and must not re-sign it
  after download.
- Restart through the Garyx CLI.
- When continuation is needed in an active agent thread, queue a wake:

```bash
garyx gateway restart --wake thread <thread_id> --wake-message "continue"
```

- Use `--no-wake` only when no continuation is intended.
