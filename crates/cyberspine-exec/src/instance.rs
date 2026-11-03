//! One execution context: state, the `resume` loop and the instruction bodies
//! (`docs/exec.md` §2–§8).

use alloc::boxed::Box;
use alloc::vec;

use crate::program::Program;
use crate::registers::Registers;
use cyberspine_isa::clock::{Hz, Seconds};
use cyberspine_isa::jump::jump_target_unchecked;
use cyberspine_isa::math;
use cyberspine_isa::register::RegId;
use cyberspine_isa::{Insn, OpCode};

/// The per-tick sensor snapshot, frozen for every instance in the tick
/// (`docs/isa.md` §G7).
pub struct Sensors<'s> {
    pub f: &'s [f32],
    pub i: &'s [i32],
}

impl<'s> Sensors<'s> {
    pub fn new(f: &'s [f32], i: &'s [i32]) -> Self {
        Self { f, i }
    }
}

/// Where an instance stopped (`docs/isa.md` §8.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Suspended,
    /// Parked on an `INTR` until the host answers it (`docs/isa.md` §4.8).
    Interrupted,
    Halted,
    Faulted,
}

/// What one `resume` did (`docs/exec.md` §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Yielded,
    Waited {
        until: u64,
    },
    /// The instance raised an interrupt and is parked until the host resolves
    /// it. `pc` is the `INTR` instruction itself; the resume point is `pc + 1`.
    Interrupted {
        reason: u32,
        pc: usize,
    },
    /// `resume` was called while the instance was still parked on an interrupt.
    /// Nothing ran; the host forgot to resolve it.
    Parked,
    Halted,
    Faulted(Fault),
    /// The instance had already terminated; nothing ran.
    Dead,
}

/// `docs/isa.md` §G12: v1 has exactly one way to fault.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Fault {
    #[error("fuel exhausted at instruction {pc}")]
    FuelExhausted { pc: usize },
}

/// The interrupt an instance is parked on (`docs/isa.md` §4.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingInterrupt {
    /// Index into the image's reason table.
    pub reason: u32,
    /// The `INTR` instruction itself.
    pub pc: usize,
}

/// Why an instance could not be released from an interrupt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InterruptError {
    #[error("the instance is not parked on an interrupt")]
    NotInterrupted,
    #[error("the instance is parked on reason {expected}, not {given}")]
    WrongReason { expected: u32, given: u32 },
}

/// One coroutine's state.
///
/// The program is a parameter of [`Instance::resume`] rather than a field, so a
/// host can keep an instance next to the image it runs without a
/// self-reference.
pub struct Instance {
    pc: usize,
    regs: Registers,
    vars_f: Box<[f32]>,
    vars_i: Box<[i32]>,
    pending: Box<[f32]>,
    published: Box<[f32]>,
    pend_mask: Mask,
    publ_mask: Mask,
    wake_at: u64,
    status: Status,
    fault: Option<Fault>,
    interrupt: Option<PendingInterrupt>,
}

impl Instance {
    pub fn new(prog: &Program<'_>) -> Self {
        let muscles = prog.muscle_count();
        Self {
            pc: 0,
            regs: Registers::new(),
            vars_f: prog.vars_f().iter().map(|var| var.init).collect(),
            vars_i: prog.vars_i().iter().map(|var| var.init).collect(),
            pending: vec![0.0; muscles].into_boxed_slice(),
            published: vec![0.0; muscles].into_boxed_slice(),
            pend_mask: Mask::new(muscles),
            publ_mask: Mask::new(muscles),
            wake_at: 0,
            status: Status::Suspended,
            fault: None,
            interrupt: None,
        }
    }

    /// Run until the instance yields, waits, interrupts, halts or faults — or
    /// until the fuel for this resume is spent (`docs/exec.md` §4).
    pub fn resume(&mut self, prog: &Program<'_>, sensors: &Sensors<'_>, now: u64) -> Step {
        assert!(
            sensors.f.len() >= prog.sensor_f_count(),
            "host contract: {} float sensors sampled, {} declared",
            sensors.f.len(),
            prog.sensor_f_count()
        );
        assert!(
            sensors.i.len() >= prog.sensor_i_count(),
            "host contract: {} integer sensors sampled, {} declared",
            sensors.i.len(),
            prog.sensor_i_count()
        );
        match self.status {
            Status::Suspended => {}
            // Parked on an interrupt: the host has to answer it first
            // (`docs/isa.md` §4.8). This is not an error and not a terminal
            // state, so it gets its own answer rather than `Dead`.
            Status::Interrupted => return Step::Parked,
            Status::Halted | Status::Faulted => return Step::Dead,
        }

        let mut fuel = prog.fuel();
        loop {
            if fuel == 0 {
                return self.fail(Fault::FuelExhausted { pc: self.pc });
            }
            fuel -= 1;

            let insn = prog.code()[self.pc];
            match self.exec(prog, insn, sensors, now) {
                Flow::Next => self.pc += 1,
                Flow::Jump(target) => self.pc = target,
                Flow::Yield => {
                    self.pc += 1;
                    self.status = Status::Suspended;
                    return Step::Yielded;
                }
                Flow::Wait(until) => {
                    self.pc += 1;
                    self.wake_at = until;
                    self.status = Status::Suspended;
                    return Step::Waited { until };
                }
                Flow::Interrupt(reason) => {
                    let pc = self.pc;
                    self.pc += 1;
                    self.wake_at = now;
                    self.status = Status::Interrupted;
                    self.interrupt = Some(PendingInterrupt { reason, pc });
                    return Step::Interrupted { reason, pc };
                }
                Flow::Halt => {
                    self.status = Status::Halted;
                    return Step::Halted;
                }
            }
        }
    }

    /// Release an instance parked on an interrupt (`docs/isa.md` §4.8).
    ///
    /// The reason must match the one the host was told about, so a host that
    /// mixes up two interrupts is refused rather than releasing the wrong one.
    pub fn resume_interrupted(&mut self, reason: u32) -> Result<(), InterruptError> {
        if self.status != Status::Interrupted {
            return Err(InterruptError::NotInterrupted);
        }
        let pending = self
            .interrupt
            .expect("an `Interrupted` instance always carries its pending interrupt");
        if pending.reason != reason {
            return Err(InterruptError::WrongReason {
                expected: pending.reason,
                given: reason,
            });
        }
        self.interrupt = None;
        self.status = Status::Suspended;
        Ok(())
    }

    /// The interrupt this instance is parked on, if any.
    pub fn interrupt(&self) -> Option<PendingInterrupt> {
        self.interrupt
    }

    pub fn status(&self) -> Status {
        self.status
    }

    /// The tick at which a `WAIT` becomes runnable; the host resumes when
    /// `wake_at <= now` (`docs/exec.md` §7.2).
    pub fn wake_at(&self) -> u64 {
        self.wake_at
    }

    pub fn fault(&self) -> Option<Fault> {
        self.fault
    }

    pub fn pc(&self) -> usize {
        self.pc
    }

    pub fn muscle_count(&self) -> usize {
        self.published.len()
    }

    /// The published command for one muscle, or `None` if this instance has never
    /// flushed a value for it (`docs/isa.md` §8.2).
    ///
    /// `None` is semantics, not an error: "this instance has no opinion about
    /// that muscle" is a state every merge step handles.
    ///
    /// # Panics
    ///
    /// `muscle >= self.muscle_count()`. That is a broken internal invariant, not
    /// a caller error: a `VerifiedImage` cannot address a muscle outside its own
    /// table (I5), and `Host::load` admits only images whose muscle table equals
    /// the host's scale.
    pub fn published(&self, muscle: usize) -> Option<f32> {
        self.publ_mask.test(muscle).then(|| self.published[muscle])
    }

    pub fn registers(&self) -> &Registers {
        &self.regs
    }

    pub fn vars_f(&self) -> &[f32] {
        &self.vars_f
    }

    pub fn vars_i(&self) -> &[i32] {
        &self.vars_i
    }

    fn fail(&mut self, fault: Fault) -> Step {
        self.status = Status::Faulted;
        self.fault = Some(fault);
        Step::Faulted(fault)
    }

    #[inline]
    fn rf(&self, id: RegId) -> f32 {
        self.regs.get_f(id)
    }

    #[inline]
    fn ri(&self, id: RegId) -> i32 {
        self.regs.get_i(id)
    }

    #[inline]
    fn wf(&mut self, id: RegId, value: f32) {
        self.regs.set_f(id, value);
    }

    #[inline]
    fn wi(&mut self, id: RegId, value: i32) {
        self.regs.set_i(id, value);
    }

    fn jump_if(&self, taken: bool, insn: Insn) -> Flow {
        if taken {
            Flow::Jump(jump_target_unchecked(self.pc, insn.imm_i32()))
        } else {
            Flow::Next
        }
    }

    fn exec(&mut self, prog: &Program<'_>, insn: Insn, sensors: &Sensors<'_>, now: u64) -> Flow {
        match insn.op {
            OpCode::Fmovi => {
                self.wf(insn.rd, f32::from_bits(insn.imm));
                Flow::Next
            }
            OpCode::Imovi => {
                self.wi(insn.rd, insn.imm_i32());
                Flow::Next
            }
            OpCode::Fmov => {
                self.wf(insn.rd, self.rf(insn.ra));
                Flow::Next
            }
            OpCode::Imov => {
                self.wi(insn.rd, self.ri(insn.ra));
                Flow::Next
            }

            OpCode::Fadd => {
                self.wf(insn.rd, self.rf(insn.ra) + self.rf(insn.rb));
                Flow::Next
            }
            OpCode::Fsub => {
                self.wf(insn.rd, self.rf(insn.ra) - self.rf(insn.rb));
                Flow::Next
            }
            OpCode::Fmul => {
                self.wf(insn.rd, self.rf(insn.ra) * self.rf(insn.rb));
                Flow::Next
            }
            OpCode::Fdiv => {
                self.wf(insn.rd, self.rf(insn.ra) / self.rf(insn.rb));
                Flow::Next
            }
            OpCode::Fmin => {
                self.wf(insn.rd, math::fmin(self.rf(insn.ra), self.rf(insn.rb)));
                Flow::Next
            }
            OpCode::Fmax => {
                self.wf(insn.rd, math::fmax(self.rf(insn.ra), self.rf(insn.rb)));
                Flow::Next
            }

            OpCode::Fneg => {
                self.wf(insn.rd, math::fneg(self.rf(insn.ra)));
                Flow::Next
            }
            OpCode::Fabs => {
                self.wf(insn.rd, math::fabs(self.rf(insn.ra)));
                Flow::Next
            }
            OpCode::Fsqrt => {
                self.wf(insn.rd, math::sqrt(self.rf(insn.ra)));
                Flow::Next
            }
            OpCode::Fsin => {
                self.wf(insn.rd, math::sin(self.rf(insn.ra)));
                Flow::Next
            }
            OpCode::Fcos => {
                self.wf(insn.rd, math::cos(self.rf(insn.ra)));
                Flow::Next
            }
            OpCode::Ffrac => {
                self.wf(insn.rd, math::frac(self.rf(insn.ra)));
                Flow::Next
            }

            OpCode::Fmadd => {
                let value = math::madd(self.rf(insn.ra), self.rf(insn.rb), self.rf(insn.rc));
                self.wf(insn.rd, value);
                Flow::Next
            }
            OpCode::Flerp => {
                let value = math::lerp(self.rf(insn.ra), self.rf(insn.rb), self.rf(insn.rc));
                self.wf(insn.rd, value);
                Flow::Next
            }
            OpCode::Fclamp => {
                let value = math::clamp(self.rf(insn.ra), self.rf(insn.rb), self.rf(insn.rc));
                self.wf(insn.rd, value);
                Flow::Next
            }

            OpCode::Iadd => {
                self.wi(insn.rd, self.ri(insn.ra).wrapping_add(self.ri(insn.rb)));
                Flow::Next
            }
            OpCode::Isub => {
                self.wi(insn.rd, self.ri(insn.ra).wrapping_sub(self.ri(insn.rb)));
                Flow::Next
            }
            OpCode::Imul => {
                self.wi(insn.rd, self.ri(insn.ra).wrapping_mul(self.ri(insn.rb)));
                Flow::Next
            }
            OpCode::Itof => {
                self.wf(insn.rd, self.ri(insn.ra) as f32);
                Flow::Next
            }
            OpCode::Ftoi => {
                self.wi(insn.rd, self.rf(insn.ra) as i32);
                Flow::Next
            }

            OpCode::Jmp => Flow::Jump(jump_target_unchecked(self.pc, insn.imm_i32())),
            OpCode::Ijz => self.jump_if(self.ri(insn.ra) == 0, insn),
            OpCode::Ijnz => self.jump_if(self.ri(insn.ra) != 0, insn),
            OpCode::Flt => self.jump_if(self.rf(insn.ra) < self.rf(insn.rb), insn),
            OpCode::Fle => self.jump_if(self.rf(insn.ra) <= self.rf(insn.rb), insn),
            OpCode::Fgt => self.jump_if(self.rf(insn.ra) > self.rf(insn.rb), insn),
            OpCode::Fge => self.jump_if(self.rf(insn.ra) >= self.rf(insn.rb), insn),
            OpCode::Ilt => self.jump_if(self.ri(insn.ra) < self.ri(insn.rb), insn),
            OpCode::Ile => self.jump_if(self.ri(insn.ra) <= self.ri(insn.rb), insn),
            OpCode::Igt => self.jump_if(self.ri(insn.ra) > self.ri(insn.rb), insn),
            OpCode::Ige => self.jump_if(self.ri(insn.ra) >= self.ri(insn.rb), insn),
            OpCode::Ieq => self.jump_if(self.ri(insn.ra) == self.ri(insn.rb), insn),
            OpCode::Ine => self.jump_if(self.ri(insn.ra) != self.ri(insn.rb), insn),

            OpCode::Fsense => {
                let value = sensors.f[insn.imm_index()];
                self.wf(insn.rd, value);
                Flow::Next
            }
            OpCode::Isense => {
                let value = sensors.i[insn.imm_index()];
                self.wi(insn.rd, value);
                Flow::Next
            }

            OpCode::Yield => Flow::Yield,
            OpCode::Wait => {
                let seconds = Seconds::new(insn.imm_f32()).expect("the loader validated WAIT");
                let ticks = ceil_ticks(seconds, prog.hz());
                Flow::Wait(now.saturating_add(ticks))
            }
            // The loader checked that `imm` names a reason row (I9); execution
            // only carries it out.
            OpCode::Intr => Flow::Interrupt(insn.imm),

            OpCode::Fexcite => {
                let muscle = insn.imm_index();
                let value = self.rf(insn.ra);
                self.pending[muscle] = value;
                self.pend_mask.set(muscle);
                Flow::Next
            }

            OpCode::Fvget => {
                let value = self.vars_f[insn.imm_index()];
                self.wf(insn.rd, value);
                Flow::Next
            }
            OpCode::Fvset => {
                let index = insn.imm_index();
                let value = self.rf(insn.ra);
                self.vars_f[index] = value;
                Flow::Next
            }
            OpCode::Ivget => {
                let value = self.vars_i[insn.imm_index()];
                self.wi(insn.rd, value);
                Flow::Next
            }
            OpCode::Ivset => {
                let index = insn.imm_index();
                let value = self.ri(insn.ra);
                self.vars_i[index] = value;
                Flow::Next
            }

            OpCode::Halt => Flow::Halt,
            OpCode::Nop => Flow::Next,
            OpCode::Flush => {
                for muscle in 0..self.published.len() {
                    if self.pend_mask.test(muscle) {
                        self.published[muscle] = self.pending[muscle];
                        self.publ_mask.set(muscle);
                    }
                }
                self.pend_mask.clear_all();
                Flow::Next
            }
        }
    }
}

/// What an instruction body tells the loop to do — the only way `pc` moves is
/// still the loop (`docs/exec.md` §4).
enum Flow {
    Next,
    Jump(usize),
    Yield,
    Wait(u64),
    Interrupt(u32),
    Halt,
}

/// Bit `i` says "muscle `i` has a value" — one word per 64 muscles, never cleared
/// by `published` (`docs/isa.md` §G10).
struct Mask(Box<[u64]>);

impl Mask {
    fn new(bits: usize) -> Self {
        Self(vec![0; bits.div_ceil(64)].into_boxed_slice())
    }

    #[inline]
    fn set(&mut self, bit: usize) {
        self.0[bit / 64] |= 1u64 << (bit % 64);
    }

    #[inline]
    fn test(&self, bit: usize) -> bool {
        self.0[bit / 64] & (1u64 << (bit % 64)) != 0
    }

    fn clear_all(&mut self) {
        self.0.fill(0);
    }
}

/// `ceil(seconds × hz)`, in f64 because a 1e-7 relative error becomes a whole
/// tick on the wrong side of the ceiling (`docs/exec.md` §7.3–§7.4).
fn ceil_ticks(seconds: Seconds, hz: Hz) -> u64 {
    let ticks = f64::from(seconds.get()) * f64::from(hz.get());
    let whole = ticks as u64;
    if ticks > whole as f64 {
        whole.saturating_add(1)
    } else {
        whole
    }
}
