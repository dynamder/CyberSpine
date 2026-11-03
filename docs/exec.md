# cyber-spine 执行核心实现规范

状态：v1 实现约束
适用范围：`Instance` 的状态与生命周期、`resume` 的执行循环、49 条指令的落地语义、fuel、`WAIT` 换算、命令缓冲区
依赖：`docs/isa.md` 定义**语义**，本文件定义**实现必须满足的判据与边界**。两者冲突时以 `isa.md` 为准。

---

## 0. 范围

本文件只描述「一条已通过加载期校验的镜像 + 一份 sensor 快照 + `now` → 推进到挂起 / `HALT` / fault」。

**不在范围内**（各自有归属）：

| 不在这里 | 归属 |
|---|---|
| 调度：谁本 tick 该跑、装载序 | 宿主 |
| 命令合并 `(layer, 装载序)` | 宿主 |
| sensor 采样时机与来源 | 宿主 / 后端 |
| 名字 → 物理下标、`imm` 重写 | 装配期（汇编器读 manifest，`isa.md` §13.2） |
| `.csp` 解析与校验 | `load.rs`，已完成 |
| 后端、RPC、动态库 | 宿主层（见 [`backend.md`](backend.md)） |

---

## 1. 唯一不变量

> **`resume` 是纯函数**：给定 `(实例状态, 快照, now)`，输出完全确定；内部零分配、零 I/O、零时钟读取、无全局状态。

推论，全部是可检查的：

1. 没有 `now()`、没有 `Instant`、没有文件、没有 socket。
2. `WAIT` 不睡眠，只算一个数写进 `wake_at` 然后返回。
3. `FEXCITE` 只写实例自己的缓冲区，不触碰任何人。
4. 一条镜像的执行可以在没有后端、没有物理世界的 PC 上完整重放。
5. 「实例之间除了命令贡献和 sensor 之外没有交互通道」（`isa.md` §2.4）在实现上是**结构性的**：`Instance` 不持有任何引用，程序按调用传入，实例之间没有共享的可变状态。

---

## 2. 状态布局

```rust
pub struct Instance {
    pc: usize,                // 指令下标，不是字节偏移
    regs: Registers,          // [f32; 64] + [i32; 64]

    vars_f: Box<[f32]>,       // 初值来自镜像
    vars_i: Box<[i32]>,

    pending:   Box<[f32]>,    // b1：FEXCITE 写这里
    published: Box<[f32]>,    // b2：FLUSH 之后才可见
    pend_mask: Box<[u64]>,
    publ_mask: Box<[u64]>,    // 不清零 —— G10 的「保持」就是免费的

    wake_at: u64,
    status: Status,
    fault: Option<Fault>,     // 便于宿主在移除前记录
}

pub enum Status { Suspended, Halted, Faulted }
```

`Program` **不是字段**，而是 `resume` 的参数。实例只拥有状态；宿主因此可以把「镜像 + 实例」并排持有，不需要 arena、`unsafe` 或 `Arc`（thumbv6m 没有 CAS）。

**内存账**（30 肌肉 / 8 浮点变量 / 4 整数变量）：寄存器 512 B + 变量 48 B + 命令 240 B + 掩码 16 B ≈ **816 B/实例**。

**掩码用 `[u64]` 而不是 `[bool]`**：长度是 `ceil(muscle_count / 64)`，30 个肌肉只占一个 word。

**每实例 6 次分配是 v1 的取舍。** 单块 arena 是后续优化，接口不变。

**`Program`** 是镜像的只读派生视图：`code: &[Insn]`、`sensor_f_count`、`sensor_i_count`、`muscle_count`、`var_f_count`、`var_i_count`、`hz`、`fuel`、`max_wait`。它不含任何运行期状态。

### 2.1 寄存器与硬连零

```rust
#[inline] fn w_f(&mut self, rd: RegId, v: f32) { if rd.get() != 0 { self.regs.f[rd.get() as usize] = v; } }
```

`f0` / `i0` 恒为 0：不写就永不改变，读取直接拿到 0。**写被静默丢弃**（`isa.md` §2.2），不报错、不 fault。

---

## 3. 返回类型

```rust
pub enum Step {
    Yielded,
    Waited { until: u64 },   // 恢复条件 wake_at <= now，判断权在宿主
    Halted,
    Faulted(Fault),
    Dead,                    // 实例已终止，本次调用什么都没做
}

pub enum Fault {
    FuelExhausted { pc: usize },   // G12：v1 只有这一种 fault
}
```

`Dead` 不是错误：对已终止实例再 `resume` 是一个可以发生的事实，宿主拿到它就知道自己漏了移除。它把「漏移除」变成可观测行为，而不是 panic 或 UB。

---

## 4. 执行循环

```rust
impl Instance {
    pub fn resume(&mut self, prog: &Program, sensors: &Sensors<'_>, now: u64) -> Step {
        assert!(sensors.f.len() >= prog.sensor_f_count());   // 宿主违约，不是程序 bug
        assert!(sensors.i.len() >= prog.sensor_i_count());
        if self.status != Status::Suspended { return Step::Dead; }

        let mut fuel = prog.fuel();             // 每次恢复重新装满（§7.1 按恢复计量）
        loop {
            if fuel == 0 { return self.fault(Fault::FuelExhausted { pc: self.pc }); }
            fuel -= 1;

            let insn = prog.code()[self.pc];
            match self.exec(prog, insn, sensors, now) {
                Flow::Next         => self.pc += 1,
                Flow::Jump(target) => self.pc = target,
                Flow::Yield        => { self.pc += 1; self.status = Status::Suspended; return Step::Yielded; }
                Flow::Wait(at)     => { self.pc += 1; self.wake_at = at; self.status = Status::Suspended;
                                        return Step::Waited { until: at }; }
                Flow::Halt         => { self.status = Status::Halted; return Step::Halted; }
            }
        }
    }
}
```

三条结构约束：

1. **`pc` 只由循环推进**。指令体只返回 `Flow`，绝不自己改 `pc`。「先 `pc += 1` 再挂起」这条约定（§4.8）因此只写一次。
2. **`fuel` 在取指前扣**。`fuel = 0` 的镜像会在 `pc = 0` 处零条指令就 fault（§7.3）。
3. **挂起的 `pc` 已经是下一条**。恢复点在 `YIELD` / `WAIT` 之后（§4.8）。

### 4.1 索引检查策略（v1）

**镜像派生的下标一律用安全索引**：`code[pc]`、`vars[m]`、`sensors.f[imm]`、`sensors.i[imm]`。

理由：加载期已经保证这些下标合法，安全索引在这里的作用不是「防止程序越界」，而是**把「加载期的承诺被破坏」变成 panic 而不是 UB**。等实现稳定、测试矩阵齐备之后，换成 `unsafe { get_unchecked }` 是一处纯机械改动，不影响任何语义。

**唯一的例外是寄存器**：`exec/registers.rs` 的四处访问用 `get_unchecked`。那里的下标不是镜像派生的，而是 `RegId`——一个只能由 `RegId::new`（拒绝 ≥ 64）产生的 newtype。安全性由类型不变量保证，不依赖加载期的承诺；四个 `unsafe` 块各自带着这条 SAFETY 注记。

---

## 5. 指令实现要点

### 5.1 直接映射（无坑）

| 指令 | 实现 |
|---|---|
| `FMOVI` | `w_f(rd, f32::from_bits(imm))` —— 位精确，不经数值转换 |
| `IMOVI` | `w_i(rd, imm as i32)` |
| `FMOV` / `IMOV` | 复制 |
| `FADD`…`FDIV` | `a op b` |
| `IADD` / `ISUB` / `IMUL` | `wrapping_add` / `wrapping_sub` / `wrapping_mul` |
| `ITOF` | `i as f32`（round-to-nearest-even） |
| `FTOI` | `f as i32` —— Rust 的饱和转换**恰好就是**规范要的：向零截断、NaN → 0、越界饱和。不要自己写 |
| `FVGET` / `FVSET` / `IVGET` / `IVSET` | 直取直存 |
| `FEXCITE` | `pending[m] = v; pend_mask.set(m);` —— **不碰 `published`** |
| `NOP` / `HALT` | — |

### 5.2 必须手写的六个

```rust
FNEG:  f32::from_bits(a.to_bits() ^ 0x8000_0000)   // 对 NaN 与 ±0 同样成立
FABS:  f32::from_bits(a.to_bits() & 0x7fff_ffff)
```

`FMIN` / `FMAX`（`isa.md` §4.2 的表就是判据）：

```rust
fn fmin(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() { f32::NAN }
    else if a == b { f32::from_bits(a.to_bits() | b.to_bits()) }   // ±0 → -0
    else if a < b { a } else { b }
}

fn fmax(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() { f32::NAN }
    else if a == b { f32::from_bits(a.to_bits() & b.to_bits()) }   // ±0 → +0
    else if a > b { a } else { b }
}
```

**禁止** `f32::min` / `f32::max`：那是 `minimumNumber` 语义。

```rust
FMADD: { let p = a * b; p + c }          // 两次舍入；绝不使用 mul_add
FLERP: { let t = b - a; let u = t * c; a + u }
FCLAMP: fmin(fmax(x, lo), hi)            // 复合定义：lo > hi 时结果是 hi
```

### 5.3 控制流

偏移算术只存在于 `jump.rs`。执行期用一个无检版本，与 `jump_target` 共用同一个内部算式：

```rust
#[inline] fn target_raw(pc: usize, off: i32) -> i128 { pc as i128 + 1 + off as i128 }

pub fn jump_target(pc: usize, off: i32) -> Option<usize>;                // 加载期，带检查
#[inline] pub fn jump_target_unchecked(pc: usize, off: i32) -> usize;    // 执行期
```

算术用 `i128` 而不是 `i64`：`pc + 1` 在 `usize::MAX` 处会溢出，`i64` 在那里变成负数，于是「无检版本」与带检版本对同一条偏移给出不同答案。`i128` 对任何 `usize` 与任何 `i32` 都精确。

条件跳转统一成「不跳 `Flow::Next`，跳 `Flow::Jump`」：

```rust
Flt => if f(ra) < f(rb) { Jump(t) } else { Next },
IJZ => if i(ra) == 0  { Jump(t) } else { Next },
```

浮点比较直接用 `<` / `<=` / `>` / `>=`：NaN 参与时 Rust 返回 `false`，与 G5「任一操作数为 NaN 时不跳转」逐字一致，不需要任何特殊处理。

---

## 6. fuel

- 头段 `fuel` 是**每次恢复**的指令预算，`resume` 开始时重新装满。
- 每条**已执行**的指令消耗 1，等权重，含 `HALT`、`NOP`、`FLUSH`、`YIELD`、`WAIT`、以及条件成立与否都会执行的跳转本身。
- 在 fuel 归零后的**下一条指令执行之前**触发 fault，`pc` 指向那条没执行的指令。
- 相同输入必须死在同一个 `pc`：这是可重放的行为，不是错误。

`fuel = 0` 是合法声明，含义是「任何指令都跑不了」。

---

## 7. `WAIT` 的换算

### 7.1 为什么是 `ceil`

VM 的时间只有 **tick**。`hz = 60` 表示 1 秒 60 个 tick，`dt = 1/60 秒`。`WAIT` 的参数是秒，所以

```
k = 秒数 ÷ dt = 秒数 × hz
```

实例只能在 tick 边界醒来，所以 `k` 必须**向上取整**：

| 取整 | `WAIT 0.01` @ 60Hz（0.6 tick） | 后果 |
|---|---|---|
| floor | 0 | 下一个 tick 就恢复 = 根本没等，退化成 `YIELD` |
| round | 1 | 0.6 看起来对，但 1.4 → 1 会**提前** 0.4 tick |
| ceil | 1 | 不会早于要求的时长，最多晚不到 1 个 tick |

「宁可晚，不可早」是唯一正确的方向：早意味着等的时间不够。

### 7.2 `wake_at` 与恢复条件

执行 `WAIT` 时记一个绝对值，不做倒计时。`now = 100` 时执行 `WAIT 0.3`（hz = 60）：

```
k = ceil(0.3 × 60) = 19
wake_at = 100 + 19 = 119
```

> **是 19 不是 18。** 镜像里的 `0.3` 是 f32 位模式，实际值 `0.300000012`，
> 乘 60 得 `18.0000007`，`ceil` 进到 19。这正是 §7.4 说的「误差被 `ceil`
> 放大成整整一个 tick」——方向是**晚**，不是早。
> `1/hz` 同理：`f32(1/60)` 略大于 1/60，等 2 个 tick；只有 `hz` 为 2 的幂时
> `1/hz` 才是精确的 1 个 tick。

宿主的规则只有一条：`wake_at <= now` 就恢复。

| now | 跑不跑 | 说明 |
|---|---|---|
| 100 | — | 刚执行完 `WAIT`，本 tick 到此为止 |
| 101 … 118 | 不跑 | 睡着的 18 个 tick |
| 119 | **跑** | `119 <= 119`，从 `WAIT` 的下一条指令继续 |

`WAIT` 自己不睡；「睡」是宿主不叫它。

### 7.3 实现

```rust
fn ceil_ticks(secs: f32, hz: u32) -> u64 {
    let x = secs as f64 * hz as f64;                 // f64：见 7.4
    if x.is_nan() || x <= 0.0 { return 0; }          // 0 / 负数 / NaN 一起挡掉
    let i = x as u64;                                // 饱和转换
    if x > i as f64 { i.saturating_add(1) } else { i }   // 小数部分非零则进一
}
```

`k` 用 `now.saturating_add(k)` 加到 `now` 上。

`k = 0` 只在 `WAIT 0` 时出现，语义等同于 `YIELD`（本 tick 已处理过它，下一次判断发生在 `now + 1`）。

### 7.4 为什么用 f64

`secs` 是镜像里的 f32 位模式，转 f64 精确；`× hz` 在 f64 里相对误差约 1e-16。

f32 的相对误差约 1e-7，而 **`ceil` 会把任何一点误差放大成整整一个 tick**：真实乘积 `18.0000001` 可能被算成 `18.0`，于是等 18 个 tick 而不是 19 个。同一份字节码在两个平台上差一个 tick，确定性当场就破了。

`ceil` 本身也不能转调 libm（no_std 里也没有）：上面 `i + (小数为非零)` 就是它的手写版。

### 7.5 与 `dt` 的关系

`isa.md` §4.8 写的是 `ceil(seconds / dt)`，本文件写 `ceil(seconds × hz)`——同一个式子，因为 `dt = 1/hz`。用乘法是因为少一次舍入来源。`dt` 在执行期不参与任何计算；它只是 `dt` sensor 的值。

---

## 8. `FLUSH`

```rust
for m in 0..muscle_count {
    if pend_mask.test(m) {
        published[m] = pending[m];
        publ_mask.set(m);
    }
}
pend_mask.clear_all();
```

- `published` 的值与 `publ_mask` **都不清空**——G10 的「保持」全部在这里。
- 必须有两份值数组（一份值 + 两个掩码做不到）：`FEXCITE m,0.5; FLUSH; FEXCITE m,0.9` 之后已发布必须是 0.5。
- `FLUSH` 不挂起、不推进时间、不触碰后端。

---

## 9. v1 明确不做的事

| 不做 | 替代 |
|---|---|
| `get_unchecked` 取消**镜像派生**下标的边界检查 | 安全索引（§4.1）；寄存器例外，安全性由 `RegId` 的类型不变量保证 |
| 按位置位遍历 `FLUSH` | `0..muscle_count` 线性扫描 |
| 单块 arena 分配 | 每实例 6 块 `Box<[T]>` |
| 指令预解码缓存、批量位运算 | 一个大 `match` 放在一个 `exec` 函数里 |
| `#[inline(always)]` 撒花 | 不撒 |

唯一保留的两处「不朴素」是 f64 的 `WAIT` 换算与位运算的 `floor`/`sqrt`：那是**正确性**需要（no_std 没有这些函数、f32 会破坏确定性），不是性能优化。

---

## 10. 测试矩阵

| 类别 | 内容 |
|---|---|
| 逐指令 | 49 条全部有语义用例，但按族组织而不是一条一段：`unary`/`binary`/`ternary`/`compare` 构造器 + 每组一个用例，`FNEG`/`FMIN`/`FMAX`/`FMADD`/`FLERP`/`FCLAMP`/`FTOI` 各自单独钉边界；断言寄存器 / 变量 / 掩码 |
| 位级 | `FMIN`/`FMAX` 的 ±0/NaN 矩阵；`FNEG`/`FABS` 对 NaN；`FMADD` 的「融合了就会不同」的输入；`FTOI` 饱和表 |
| `FFRAC` | `-1e-30 → next_down(1.0)`、`±0`、`±Inf → NaN`、`1.0 → 0.0` |
| `FSIN`/`FCOS` | 定点输入表 → 固定位模式；网格测试对照 host libm 报最大 ULP |
| `FSQRT` | `#[ignore]` 的 2³² 全穷举（与 `f32::sqrt` 逐位比较） |
| fuel | 恰好 N 条后 fault 在同一 `pc`；`fuel = 0` 在 `pc = 0` 零条指令 fault |
| `WAIT` | `0` / `1/64@64Hz = 1` / `0.3@60Hz = 19` / 上界 / 饱和 |
| 挂起存活 | 寄存器与变量跨 `YIELD`/`WAIT` 不变 |
| `FLUSH` | §8.2 的两个数组例子；`publish` 保持；`FEXCITE` 不动 `published` |
| 纯函数性 | 同输入跑两遍，命令字节流逐位相同 |
| no_std | `--target thumbv7em-none-eabihf` 编过 |

---

## 11. 后续

- 单块 arena 分配。
- 取消边界检查（有测试矩阵兜底之后）。

执行的输入只多了一样东西：镜像头段的 rig 指纹。它在 `load` 之前被宿主比对过一次（`isa.md` §13.3），执行期看不到它，也不需要看到。
