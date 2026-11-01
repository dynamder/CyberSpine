This is a bytecode VM project, intending to give a unify interface to drive real or virtual robots, actuators, etc.

use csp-dev in the /target to explore on the codebase. If it not exist, use the following command to build it.

```bash
cargo build -p agents-dev-cli --release
```

When writing code, you should almost never write comments (doc comments are allowed but should be limited to concise usage guide). Try leveraging the rust type system and a proper naming to make the code self-explanary.

Try to use the powerful rust type system to make the impossible or unwanted things impossible to compile.
