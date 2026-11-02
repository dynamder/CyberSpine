//! The two register files (`docs/isa.md` §2.1, `docs/exec.md` §2.1).

use cyberspine_isa::register::RegId;

/// `f0..f63` and `i0..i63`: cleared when an instance is loaded, kept across
/// every `YIELD` / `WAIT`, destroyed with the instance (`docs/isa.md` §2.3).
///
/// Writes to `f0` / `i0` are dropped and reads of them are zero (`docs/isa.md`
/// §2.2). `set_f` / `set_i` are the only way in, so the hardwired zero holds for
/// every instruction without each of them remembering to check.
pub struct Registers {
    f: [f32; RegId::COUNT],
    i: [i32; RegId::COUNT],
}

impl Default for Registers {
    fn default() -> Self {
        Self {
            f: [0.0; RegId::COUNT],
            i: [0; RegId::COUNT],
        }
    }
}

impl Registers {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_f(&self, id: RegId) -> f32 {
        // SAFETY: a `RegId` is at most `COUNT - 1` by construction — `new`
        // rejects larger values and `from_6bit` is crate-private.
        unsafe { *self.f.get_unchecked(usize::from(id.get())) }
    }

    pub fn get_i(&self, id: RegId) -> i32 {
        // SAFETY: as in `get_f`.
        unsafe { *self.i.get_unchecked(usize::from(id.get())) }
    }

    pub fn set_f(&mut self, id: RegId, value: f32) {
        if id == RegId::ZERO {
            return;
        }
        // SAFETY: as in `get_f`.
        unsafe { *self.f.get_unchecked_mut(usize::from(id.get())) = value };
    }

    pub fn set_i(&mut self, id: RegId, value: i32) {
        if id == RegId::ZERO {
            return;
        }
        // SAFETY: as in `get_f`.
        unsafe { *self.i.get_unchecked_mut(usize::from(id.get())) = value };
    }
}
