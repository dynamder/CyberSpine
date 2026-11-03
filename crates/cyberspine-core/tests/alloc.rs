//! How many times loading an image touches the allocator.
//!
//! The loader parses `(start, len)` handles into one arena instead of allocating a
//! `Box<str>` per name, so the cost of a load is a constant number of allocations
//! rather than one per table entry. That property is invisible in a synchronous
//! test and easy to lose — a `to_string()` in a check, a `Vec` that grows instead
//! of reserving — so it is asserted here, against a counting allocator.
//!
//! The test lives in its own binary on purpose: `#[global_allocator]` is
//! per-program, and a counter shared with other tests would measure them too.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use cyberspine_core::binfmt::image::{Image, SensorSource};
use cyberspine_core::binfmt::load::VerifiedImage;
use cyberspine_core::isa::Insn;
use cyberspine_core::isa::OpCode;
use cyberspine_core::rig::Name;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

struct Counting;

// SAFETY: every method forwards to `System` unchanged; the counter is the only
// added behaviour.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn name(index: usize) -> Name {
    Name::new(&format!("sensor_{index}")).expect("a legal identifier")
}

/// An image with `muscles` muscles, each with its read-back, plus one backend
/// sensor, one var of each kind, and `dt`/`tick`.
fn image_with(muscles: usize) -> Image {
    let mut image = Image::new(vec![Insn::new(OpCode::Halt)]);
    for index in 0..muscles {
        image.push_muscle(&name(index));
    }
    image.push_sensor_f(&name(muscles), SensorSource::Backend);
    image.push_sensor_f(&Name::new("dt").expect("legal"), SensorSource::BuiltinDt);
    image.push_sensor_i(&name(muscles + 1), SensorSource::Backend);
    image.push_sensor_i(
        &Name::new("tick").expect("legal"),
        SensorSource::BuiltinTick,
    );
    image.push_var_f(&name(muscles + 2), 0.5);
    image.push_var_i(&name(muscles + 3), -7);
    image
}

fn allocations_to_load(muscles: usize) -> usize {
    let bytes = image_with(muscles).to_bytes();

    ALLOCATIONS.store(0, Ordering::Relaxed);
    let loaded = VerifiedImage::from_bytes(&bytes).expect("a well-formed image");
    let count = ALLOCATIONS.load(Ordering::Relaxed);

    std::hint::black_box(&loaded);
    count
}

#[test]
fn loading_allocates_a_constant_number_of_times() {
    let small = allocations_to_load(4);
    let large = allocations_to_load(4096);

    assert_eq!(
        small, large,
        "loading must not allocate per name: 4 muscles took {small}, 4096 took {large}"
    );
    // One code vector, five table vectors, one name arena, two plan vectors.
    assert!(
        small <= 9,
        "expected the constant load path, got {small} allocations"
    );
}
