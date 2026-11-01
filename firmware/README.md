# cyberspine-firmware

A bare-metal Cortex-M4 image that runs `cyberspine-core` and reports through
semihosting. It is the check that the execution core is not accidentally
host-only: the loader, the floating-point kernel, fuel, `WAIT` and `FLUSH` all
run on a microcontroller with no OS, no libm and no allocator crate.

## Run

```bash
rustup target add thumbv7em-none-eabihf
cd firmware
cargo build --release
qemu-system-arm -M mps2-an386 -cpu cortex-m4 \
    -semihosting-config enable=on,target=native \
    -nographic -kernel target/thumbv7em-none-eabihf/release/cyberspine-firmware
```

A passing run prints six `ok:` lines and exits 0. A failing assertion panics,
prints its source location, and exits 1, so CI can run this unchanged.

## What runs on the device

| check | covers |
|---|---|
| `loader` | `Image::from_bytes` over a real `.csp`, blake3 included |
| `arithmetic` | `FADD`/`FMUL`/`FSQRT`/`FTOI`/`FSIN`/`FFRAC`, bit-exact |
| `control` | a backward `IJNZ` counted loop |
| `commands` | `FEXCITE` + `FLUSH` read back through `published` |
| `suspend` | `YIELD`, then `WAIT 0.3` at 60 Hz = 19 ticks (`docs/exec.md` §7.2) |
| `fuel` | exhaustion faults at the pc that did not execute |

`sqrt(2)` is asserted as the exact bit pattern `0x3FB504F3` on both the host and
the device, and `FFRAC(-1e-30)` as `next_down(1.0)`: the determinism contract is
checked across architectures, not just argued.

## Notes

- `cortex-m-rt`'s reset code is what enables the FPU. Without it every VFP
  instruction is a `NOCP` usage fault — that is what a hand-rolled startup hits
  first, and it is why this crate does not hand-roll one.
- The global allocator is a 64 KiB bump allocator (`src/allocator.rs`): the core
  makes six allocations per instance and two vectors per loaded image.
- This is a separate workspace, like `fuzz/`, so `cargo test` at the repository
  root never tries to build it for the host.
