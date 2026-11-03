# CyberSpine

CyberSpine is a tiny asm-like language with a minimal toolchain that let you drive a digital life(e.g. Live2D models, game characters) through a unified and cacheable way.

## Why CyberSpine

One important things to know first is that CyberSpine is not something that aims to be drive faster, it has a very dedicated use that basically let AI drives a body directly.

A unified model can surely reduce the burden for ai to understand basic sdk principles. And we compile the csa to binary executable for a tiny vm, so we can quickly reuse actions compiled before.

Another benefit is self-iterating. AI will get feedback from the vm exexuted result, improve the csa code and recompile. This basically formed a close-loop and let common LLM easily grant the ability to iterate on actions. You can even use a decision one model such as Jev to achieve a quick response in real circumstance, just like how spine and single reflection works.

## Demo

`demos/maze` is the first end-to-end example: a 21×15 grid maze whose path is
marked on the floor (red = turn left, blue = turn right). The action is a `.csa`
program; the body is a Rust `Backend` implementation, and the two only meet
through the rig's muscle and sensor names.

```bash
cargo run -p cyberspine-demo-maze
```

See [`demos/maze/README.md`](demos/maze/README.md) for what the demo
demonstrates and what to expect when it runs.

## Bodies

A body — the thing being driven — is a Rust `Backend` implementation, and it can
be packaged three ways:

- **linked**: depend on the body's crate and hand it to `Host::new`.
- **dynamic library**: build the body as a `cdylib` and load it at run time.
- **RPC**: run the body in its own process and connect over TCP.

All three share one wire protocol, and a body written for one is one `export!`
away from the other. See [`docs/backend.md`](docs/backend.md); the dev tool runs
an image against any of them:

```bash
csp-dev run --image action.csp --backend rpc:127.0.0.1:7777
csp-dev inspect --backend dylib:bodies/robot.dll
```

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
