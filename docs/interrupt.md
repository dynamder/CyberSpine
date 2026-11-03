# 中断：设计规范

状态：**S1 已实现**；S2 / S3 设计冻结、尚未实现
依赖：[`isa.md`](isa.md) §4.8、[`binfmt.md`](binfmt.md) §6.6、[`exec.md`](exec.md) §3、[`runtime.md`](runtime.md)
范围：guest 发起的中断、原因模板、宿主裁决 API、仲裁器边界、动态动作集合

---

## 0. 一句话

中断是 **CSA 程序主动向宿主提出、必须由宿主裁决才能继续的挂起**。它带着可插值的文本原因
离开 VM；用户配置因此从"一个 backend"变成"backend + 一个仲裁器"，仲裁器与 backend 平级，
同样有 linked / dylib / rpc 三种打包；裁决只有继续、停止、切换到另一个动作三种。

| 部分 | 内容 | 状态 |
|---|---|---|
| **S1** | `INTR`、原因模板与插值、镜像 reason 表、`Session` 的 take/resolve/unload、`cyinevm` 多镜像与内置策略 | 已实现 |
| **S2** | `cyberspine-arbiter`、`InterruptEvent` / `Batch` / `Decision`、linked/dylib 打包、`Switch` 与动态动作解析 | 设计冻结 |
| **S3** | RPC 仲裁器、非阻塞工作线程 | 设计冻结 |

---

## 1. 为什么是"guest 发起"

CSA 由 AI 在运行期生成，动作集合与失败类型都不是事先可枚举的。因此中断不能是一个
"码表"式的枚举，而必须是：

- **由程序自己决定何时提出**——只有程序知道"我处理不了"；
- **带足够的上下文**——原因必须能被一个没有参与生成 CSA 的外部程序读懂；
- **回到宿主裁决**——动作是否继续、是否换一个，是集成方的决定，不是 VM 的决定。

外部事件（人、安全逻辑、游戏状态）**不需要**一条 ISA 指令：宿主不 resume 某个实例就是
挂起它。抢占粒度是**一次 `resume`**（一个 fuel 窗口），不是一条指令；硬抢占属于 backend
或安全层，不属于 VM。

---

## 2. 三层模型

```
CSA（guest）                 宿主                        用户配置
INTR reason ──▶ Interrupted ──▶ Interrupt 票据 ──▶ 仲裁器（linked / dylib / rpc）
                    ▲                                      │
                    └──── Continue / Stop / Switch ◀──────┘
```

- VM 只负责**挂起并交出一张票据**；
- `Session` 负责**渲染原因、排队票据、应用裁决**；
- 仲裁器只看到事件、只回三种决定，读改不到实例状态。

---

## 3. 语言层

### 3.1 指令

| 编码 | 助记符 | 操作数 | 语义 |
|---|---|---|---|
| 0x72 | `INTR` | `imm`（reason 表下标） | `pc += 1`；状态转 `Interrupted`；返回 `Step::Interrupted { reason, pc }` |

- fuel 照常扣 1；`INTR` 与 `YIELD` / `WAIT` / `HALT` 一样是挂起点，因此 `FEXCITE` 未
  `FLUSH` 就 `INTR` 是**汇编期硬错误**。
- `pc` 指向 `INTR` 本身，恢复点是 `pc + 1`。
- 不是 fault，不写 fault 日志，不结束实例。

### 3.2 状态机

```
Suspended ──INTR──▶ Interrupted ──resume_interrupted(reason)──▶ Suspended
                         └──────── Stop（宿主卸载）───────────▶ 消失
```

- 未放行时 `resume` 返回 `Step::Parked`：不耗 fuel、不动 pc、不跑指令。
- 放行用 `Instance::resume_interrupted(reason)`；reason 不匹配返回 `WrongReason`，实例保持挂起。
- **fail-safe**：忘记裁决的宿主只会让实例永远停着，不会把它悄悄放回执行。固件这类不实现
  裁决的宿主，默认行为是"停"，不是"继续"。
- 挂起期间实例仍参与命令合并（保持最后发布向量与层权）；`Stop` 是卸载，贡献随之消失。

### 3.3 不变量

`resume` 仍是纯函数。中断只增加了**恢复序列的一个外部输入**：裁决序列。给定相同的
实例状态、sensor 快照、`now`、fuel 与裁决序列，激发流逐位可复现。

---

## 4. 原因模板

### 4.1 语法

原因操作数是**单行双引号字符串**，与 `.rig` 共用词法（只有 `\\` 与 `\"` 是转义）：

```asm
INTR "obstacle: x={f3:.2} speed={f5:.2} state={vi:state} {{v2}}"
```

| 占位符 | 含义 |
|---|---|
| `{fN}` / `{iN}` | 寄存器，`N` 在 0..=63 |
| `{vf:name}` / `{vi:name}` | 实例变量；装配期校验已声明并改写为下标 |
| `{fN:.K}` / `{vf:name:.K}` | 浮点精度，`K` 在 0..=9 |
| `{{` / `}}` | 字面大括号 |

只**取值**，不求值、不做算术。需要计算时，程序先把结果写进寄存器或变量。

### 4.2 装配期

- 变量名解析成下标，镜像里存 `{vf:3}` 这种**存储形式**；
- 相同规范形式只存一份，`INTR` 的 `imm` 是它在 `reasons` 表里的下标；
- 文本、占位符语法、寄存器号、变量名、精度全部检查；单条模板 ≤ 4 KiB（`MAX_REASON_LEN`）。

### 4.3 加载期

- 模板非空、合法 UTF-8、≤ 4 KiB；
- 可解析；变量只按下标引用，且下标在对应变量表内（M7）；
- `INTR.imm < reason_count`（I9）。

### 4.4 渲染

- **在宿主**，于 `Session` 观察到 `Step::Interrupted` 的瞬间完成。此时实例已冻结，所以取到的
  就是中断点的值。
- f32 用最短往返 `Display`，`.K` 用定点；i32 十进制；越界变量渲染为 `<missing>`，绝不 panic。
- 事件同时携带：
  - `format`：模板原文，**稳定的判类 key**（程序化 handler 用它匹配）；
  - `reason`：渲染结果，给人 / LLM 读。

渲染文本是运行产物，不属于 ISA 契约；重放时按实际文本记录。

---

## 5. 镜像格式

- 固定头 offset 60：`reason_count: u32`（原 `reserved1`）。
- `reasons` 表追加在 `vars_i` 之后，元素是 NUL 结尾 UTF-8 文本（不是名字，允许空格、标点、非 ASCII）。
- 装配器按规范形式去重；加载器不要求表内唯一（那是写作者的规范形式，不是可读性规则）。
- 预发布阶段不 bump `format_version`，不提供旧镜像兼容分支。

细节见 [`binfmt.md`](binfmt.md) §6.6 / §9.2 M7 / §9.3 I9。

---

## 6. 宿主 API（S1，已实现）

```rust
pub struct Interrupt {
    pub instance: InstanceId,
    pub pc: usize,          // INTR 本身；恢复点是 pc + 1
    pub tick: u64,
    pub format: Box<str>,   // 模板原文
    pub reason: Box<str>,   // 渲染结果
    reason_id: u32,
    seq: u64,
}

pub enum Verdict { Continue, Stop }

impl Session {
    pub fn take_interrupts(&mut self) -> Vec<Interrupt>;
    pub fn resolve(&mut self, ticket: Interrupt, verdict: Verdict) -> Result<(), DecideError>;
    pub fn unload(&mut self, id: InstanceId) -> bool;
    pub fn parked(&self) -> Vec<InstanceId>;
}
```

- **票据（token）语义**：`Interrupt` 无法在 `Session` 之外构造（`seq` 私有，只有 `Session` 产出）。
  `resolve` 校验 `pending_seq`，同一张票只能用一次；旧票据返回 `Stale`。
- `take_interrupts` 只清投递队列，**不清挂起状态**：没收到的票据不会让实例提前恢复。
- `Continue` 清 `Interrupted`，下一 tick 从 `INTR` 之后继续；`Stop` 卸载，状态丢弃，身体按最后
  命令保持（G10）。
- `unload` 对运行中、挂起中、已结束的实例都可用，是"停止并换动作"的底层操作。
- `parked` 用于宿主在仲裁器失效时 fail-closed。

实现：[`crates/cyberspine-vm/src/host.rs`](../crates/cyberspine-vm/src/host.rs)。

---

## 7. 仲裁器边界（S2，设计冻结）

### 7.1 类型

```rust
pub struct InterruptEvent {
    pub interrupt: Interrupt,   // 票据本体
    pub action: Box<str>,       // 被打断实例当前动作
    pub actions: Vec<Box<str>>, // 中断瞬间可见的动作快照
    pub image_hash: [u8; 32],   // 动作内容哈希
}

pub enum Verdict {              // cyberspine-arbiter 的三值裁决
    Continue,
    Stop,
    Switch { action: Box<str> },
}

pub struct Decision {
    pub event: InterruptEvent,  // 原样交回的回执
    pub verdict: Verdict,
}

pub struct Batch {
    pub decisions: Vec<Decision>,
    pub failure: Option<ArbiterError>,
}

pub trait Arbiter {
    fn deliver(&mut self, event: InterruptEvent); // 不得阻塞
    fn take(&mut self) -> Batch;                  // 无决定时返回空
}
```

> 命名说明：`Session::resolve` 收的是 `cyberspine-vm::Verdict`，只有 `Continue` / `Stop`；
> `Switch` 是 runtime 对 `unload + load` 的组合，不属于 VM。两个同名类型分处两 crate，
> 这正是"VM 不知道动作"的落点。

### 7.2 契约

1. 每个 `deliver` 过的事件最终**恰好出现一次** `Decision`，并原样带回票据。
2. 实现可以延迟决定；`take` 返回空是正常状态。
3. 内部失败（断连、超时）时，实现为自己的每个未决事件合成 `Stop`，并通过 `failure` 报告一次。
4. 线上回复引用未知 `event_id`（伪造或重放）：丢弃并报告。已被 `unload` 的实例仍会收到
   决定，`resolve` 以 `UnknownInstance` / `Stale` 拒绝——票据的一次性由 `seq` 保证。

### 7.3 为什么非阻塞

挂起期间身体仍需被 tick 驱动——停止 tick 不等于停住身体，只等于停止更新。仲裁器的延迟
（RPC、AI 裁决）因此不能阻塞 tick 循环；这也是 `deliver` / `take` 的形状存在的原因。

### 7.4 打包

| 打包 | 机制 |
|---|---|
| linked | 实现 `Arbiter` |
| dylib | `csp_arbiter_v1`，与 backend ABI 同构的 `create/destroy/call/last_error` |
| rpc | 独立于 body 的连接 + 工作线程；`deliver` 投递、`take` 收裁决 |

线上复用 `CSPB` 帧与 MessagePack，消息为 `Event` / `Decision`（含 `event_id = seq`）。
适配器只保存 `event_id → InterruptEvent` 的表，回复时取回票据，**自己从不构造票据**；
连接失效时把表里所有票据合成 `Stop` 并置 `failure`。

### 7.5 失败策略

- `delivery` / 连接失败：回退到配置的默认策略（默认 `Stop`），session 不终止。
- 未知 action、目标镜像加载失败：报错 + `Stop`（fail-closed）。
- 仲裁器与 backend 同级可信；它只能回三种决定，读改不到实例状态。

---

## 8. 动作空间与 `Switch`（S2）

- **不做启动时固定目录**：动作集合由宿主在运行期维护，AI 可以随时生成并注册新动作。
- 每个事件携带**中断瞬间的可见动作快照**；不做握手目录，也不做 catalog 推送 / epoch 同步。
- 动作引用是**不透明字符串**，由宿主的 `ActionResolver` 解释（注册表 id 或动作目录里的文件名）：

```rust
pub trait ActionResolver {
    fn actions(&self) -> Vec<Box<str>>;
    fn resolve(&self, action: &str) -> Result<VerifiedImage, ActionError>;
}
```

- `Switch` 由 runtime 执行：解析 → 读文件 + loader → **先 `Session::load` 新实例，成功后再
  `unload` 旧实例**。解析或装载失败时旧实例保持挂起，并回退默认策略。
- 不通过 MessagePack 传镜像字节：动作集合在任一瞬时是确定的，用引用 + 宿主解析即可。

---

## 9. 确定性

- `resume` 仍是纯函数；恢复序列多一个外部输入：**裁决序列**。
- 重放需要记录 `(tick, instance, format, reason, decision)`；`--trace` 仍只记每 tick 的命令向量。
- 渲染文本依赖实例状态，属于运行产物，不进入 ISA 契约。

---

## 10. CLI

当前（S1）：

```bash
cyinevm -i idle.csp -i walk.csp --ticks 300
cyinevm -i interrupt.csp --on-interrupt continue --until-halt
```

- `-i` 可重复，顺序 = 装载顺序；共享一个时钟与一个 body。
- `--on-interrupt stop`（默认）卸载被打断的实例；`continue` 下一 tick 继续。
- 中断报告走 stderr（实例、路径、pc、tick、插值后的原因）；退出码不变。

目标（S2）：

```bash
cyinevm -i a.csp -i b.csp --actions actions/ --interrupt dylib:handler.dll
```

---

## 11. 非目标

- 外部抢占指令：宿主不 resume 就是挂起；硬抢占属于 backend / 安全层。
- 把镜像字节交给仲裁器：只传动作引用。
- 固定动作目录：违背 AI 实时生成的设计初衷。
- 裁决回传值 / 收尾机会：`Stop` 直接丢弃状态；需要收尾就换动作。
- 身体可见的中断：body 不需要知道实例被冻结。

---

## 12. 实现位置

| 内容 | 位置 |
|---|---|
| `INTR`、`ImmType::Text` | [`crates/cyberspine-isa/src/lib.rs`](../crates/cyberspine-isa/src/lib.rs) |
| 模板解析 / 渲染 / 校验 | [`crates/cyberspine-binfmt/src/reason.rs`](../crates/cyberspine-binfmt/src/reason.rs) |
| `reasons` 表、头字段、M7/I9 | [`crates/cyberspine-binfmt/src/image.rs`](../crates/cyberspine-binfmt/src/image.rs)、[`load.rs`](../crates/cyberspine-binfmt/src/load.rs) |
| 字符串操作数、模板改写、reason 表 | [`crates/cyberspine-asm/src/assembler.rs`](../crates/cyberspine-asm/src/assembler.rs)、[`text.rs`](../crates/cyberspine-asm/src/assembler/text.rs) |
| 状态机、`resume_interrupted` | [`crates/cyberspine-exec/src/instance.rs`](../crates/cyberspine-exec/src/instance.rs) |
| 渲染、票据、`resolve` / `unload` / `parked` | [`crates/cyberspine-vm/src/host.rs`](../crates/cyberspine-vm/src/host.rs) |
| 多镜像、内置策略 | [`crates/cyberspine-runtime/src/run.rs`](../crates/cyberspine-runtime/src/run.rs)、[`cli.rs`](../crates/cyberspine-runtime/src/cli.rs) |

---

## 13. 设计取舍记录

| 早期方案 | 为什么改 |
|---|---|
| 原因是 u32 码 | CSA 由 AI 实时生成，码表会变成第二份真相；改为文本模板 |
| 原因只带一个寄存器 | 上下文不足；改为任意寄存器 / 变量插值 |
| 中断当普通 YIELD，parked 由宿主记 | 忘记裁决会静默放跑；改为 VM 级 `Interrupted` + `Parked` |
| 仲裁器同步阻塞 | 会停掉身体驱动；改为 `deliver` / `take` 非阻塞 |
| 仲裁器交回镜像字节 | 改为动作引用，由宿主解析 |
| 动作目录启动时固定 | 违背实时生成的设计初衷；改为事件内的瞬时快照 |
| `submit` / `poll` 两方法 | 方向与所有权不清；改为 `deliver` / `take -> Batch`，事件即回执 |
