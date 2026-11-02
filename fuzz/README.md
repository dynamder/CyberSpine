# Assembler fuzzing

Required: `cargo install cargo-fuzz` and a nightly toolchain.

## Targets

- `assemble` — strict oracle. A panic is a crash; an accepted image that the
  loader rejects is a crash. This is where the panic inventory comes from.
- `assemble_unwind` — accept-path oracle. Records each panic site once and keeps
  fuzzing. There are currently no known panic sites in the parse stage; the
  recording hook is kept as a regression guard while it looks for an accepted
  program whose image fails the loader or is not canonical.

Both targets assert the same contract: **accepted means the image must pass
`Image::from_bytes` and re-serialize to the same bytes** (`docs/binfmt.md` §8/§9).
"Legal code rejected" is covered by the seed corpus and `tests/generate.rs`.

Muscles and observables live in the rig manifest, not in the source
(`docs/asm.md` §3.2), so both targets derive a legal `Rig` from the input first
(`fuzz_targets/rig_for.rs`): it collects every name the byte string mentions,
drops the reserved ones, and keeps each name in one table. Without that, almost
every input would stop at "unknown name" and never reach generation.

## Run

```bash
cargo +nightly fuzz run assemble -- \
    -max_total_time=60 \
    -max_len=4096 \
    -dict="$PWD/fuzz/assemble.dict"

cargo +nightly fuzz run assemble_unwind fuzz/corpus/assemble -- \
    -max_total_time=60 \
    -max_len=4096 \
    -dict="$PWD/fuzz/assemble.dict"
```
