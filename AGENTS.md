## Project Intention

This is a bytecode VM project, intending to give a unify interface to drive real or virtual robots, actuators, etc.

## Primary rule
You are not allowed to modify the AGENTS.md under any circumstances.

You should almost never write code comments. Try use clear naming and module structure, plus leveraging the rust type system to make the code self-explanary. You should always try to make illegal things impossible to express, like using type-state pattern to eliminate illegal state transition. Only for those extremely tricky things you should write a concise comment to notice others. Doc comments are recommended, but they should be targeted on how to use the api and the common pitfalls when using them, and the doc comments should be concise.

Try you best to keep everything single-sourced, whatever it is code or doc.

Remember to update the doc to reflect the current codebase if you make some changes.

When error-handling, every error should be exposed properly to the user.

## Exploring

When exploring the codebase, you should first evaluate its scale using tools like `scc`, if it's huge, use subagents to do the exploration.

## Designing

When you are planning a new feature's implementation or a restructure plan, try think about the plan that consistent with the project claimed philosophy and good maintainability with lowest tech-debt, not propose a plan fully because it's the smallest modification or quickest fix.

## Development

Before any restructure task, first snapshot the baseline test and example behavior， and verify the restructured code produce the same. After you verify them, clean the temporary snapshot.

If you are writing a new feature, you should always search relevant code and try to reuse them when possible and doesn't contradiced with the primary rule.

Git commit in small step, conventional commit, but don't prose about the commit message. The commit message should only be a few sentences.

A test that pins bytes (the snapshots under `tests/snapshots/`) is a contract. CI never does. A change that touches both
`src/` and a `.snap` file is a contract change. You should almost rarely change a contract, if you are sure you need to, say in the PR or directly to the user why each changed line is expected.

When you finish a new feature, there should always be a human-runnable and human-distinguishable executable for users to run, see and check.

## Maintaining

If you are doing any maintaining work, you should first view the github issue. If you find anything seems broken, give a reproduce sample and fire an issue to github. When you are doing anything with an issue, you should use a pr for your modification. 

Keep the issue and pr message you write concise, only cover the necessary messages to illustrate.

## Tools

The two tools are the workspace default members(It's also the deployment artifact). Build them with:

```bash
cargo build --release
```

`target/release/cyinea` assembles a `.csa`; `target/release/cyinevm` runs a `.csp`.
Omit `--backend` to run against the built-in dummy body.

The checks are one command, and it is the same list CI runs:

```bash
cargo xtask check   # fmt, clippy, no_std, every feature combination
cargo xtask test    # build the cdylib fixtures, then the workspace tests
cargo xtask ci      # both
cargo xtask doctor  # build environment and worktree hygiene
```

Tests load the fixture libraries from the profile directory; they do not build
them. `cargo xtask test` takes care of that.

Some checks are too slow to gate a pull request and run nightly instead
(`.github/workflows/nightly.yml`): the same tests with `PROPTEST_CASES=4096` on
three hosts, `cargo mutants --in-diff` over the day's commits (settings in
`.cargo/mutants.toml`), and a replay of the fuzz corpus. `cargo xtask fixtures`
builds the shared libraries on its own, which is what the mutation run needs
because it does not go through `cargo xtask test`.
