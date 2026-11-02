#![no_main]

//! Fuzz oracle for the **accept path**, with panics recorded but not fatal.
//!
//! `assemble` is the strict target: a panic there is a crash and that is where
//! the panic inventory comes from. This target deliberately does the opposite —
//! it records each panic site once and keeps fuzzing. There are currently no
//! known panic sites in the parse stage, so the recording hook is kept as a
//! regression guard while the accept path is exercised:
//!
//! > if the assembler returns `Ok(Image)`, that image must pass the loader and
//! > be byte-identical when written again.
//!
//! A loader rejection or a non-canonical write is a **false accept**: the
//! assembler called illegal code legal. We abort explicitly because the panic
//! hook has been replaced with our recording hook.

use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Mutex, Once, OnceLock};

use cyberspine_asm::assembler::{generate_image, parse};
use cyberspine_core::binfmt::image::Image;
use libfuzzer_sys::fuzz_target;

mod rig_for;

static INSTALL_HOOK: Once = Once::new();
static PANIC_SITES: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn record_panic(info: &std::panic::PanicHookInfo<'_>) {
    let site = info
        .location()
        .map(|loc| format!("{}:{}:{}", loc.file(), loc.line(), loc.column()))
        .unwrap_or_else(|| "<unknown>".to_owned());

    let sites = PANIC_SITES.get_or_init(|| Mutex::new(HashSet::new()));
    if let Ok(mut sites) = sites.lock() {
        // Print once per site; the fuzzer may take this path millions of times.
        if sites.insert(site.clone()) {
            eprintln!("FUZZ_PANIC {site}");
        }
    }
}

fuzz_target!(|data: &[u8]| {
    INSTALL_HOOK.call_once(|| std::panic::set_hook(Box::new(record_panic)));

    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };

    // Panics are the strict target's job. Here they mean "rejected".
    let produced = catch_unwind(AssertUnwindSafe(|| {
        let asm = parse(source).ok()?;
        generate_image(&asm, &rig_for::rig_for(&asm)).ok()
    }));

    let Some(image) = produced.ok().flatten() else {
        return;
    };

    let bytes = image.to_bytes();
    let reloaded = match Image::from_bytes(&bytes) {
        Ok(reloaded) => reloaded,
        Err(err) => {
            eprintln!("assembler accepted a program the loader rejects: {err}");
            std::process::abort();
        }
    };

    if reloaded.to_bytes() != bytes {
        eprintln!("assembler accepted an image that is not canonical");
        std::process::abort();
    }
});
