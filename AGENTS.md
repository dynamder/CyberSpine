This is a bytecode VM project, intending to give a unify interface to drive real or virtual robots, actuators, etc.

The two tools are the workspace default members. Build them with:

```bash
cargo build --release
```

`target/release/cyinea` assembles a `.csa`; `target/release/cyinevm` runs a `.csp`.
Omit `--backend` to run against the built-in dummy body.

When writing code, you should almost never write comments (doc comments are allowed but should be limited to concise usage guide). Try leveraging the rust type system and a proper naming to make the code self-explanary.

Try to use the powerful rust type system to make the impossible or unwanted things impossible to compile.
