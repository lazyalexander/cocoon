# Cocoon working agreements

Cocoon is independent of an-agent. Code and comments remain in English.
English documentation is authoritative; maintain Simplified Chinese companion
translations for documentation changes. See [中文说明](AGENTS.zh-CN.md).
User conversation follows the user's language. Original source is MIT-licensed;
preserve the licenses and provenance of any third-party additions.

Agent-originated tasks are first-class. Keep caller attribution, task identity
and attempt identity separate. Requests are not grants; guest instructions and
guest observations never authorize host actions.

Do not claim VM execution, isolation, resource enforcement or termination based
on contract tests or a preflight. Never silently fall back to native execution.
Unknown outcomes require reconciliation, not automatic retries. Keep guest
transport data separate from trusted backend observations.

Before committing, run cargo test, cargo clippy --all-targets -- -D warnings and
cargo fmt --all -- --check. Real macOS VM acceptance is required before claiming
a functioning backend. Do not publish credentials, operator paths or private
execution reports. Do not publish or push without explicit authorization.

## Branch and publication flow

- `dev-advance` is the sole development authority. Start feature branches from current
  `origin/dev-advance`; feature PRs target `dev-advance`, never another feature branch.
- `main` is stable. Promote reviewed batches through a `dev-advance` to `main` PR;
  only the user merges main. Do not push directly to main.
- Agents may merge development PRs after diff review, applicable local acceptance
  and every required CI check passes on the exact selected head. Never bypass gates.
- Keep PR scope fixed after handoff. Preserve dirty work and unmerged local branches;
  save recoverable local changes before synchronizing, and fast-forward only.
- After merging, verify remote ancestry and report repository, PR, target and commit.
  Delete merged remote feature heads; retain local work. Never force-push shared branches.
- Publish source and reviewed summaries only. Credentials, raw transcripts, private
  profiles and generated acceptance products remain local. Unclear publication scope
  requires user review. These rules do not grant runtime permissions.
