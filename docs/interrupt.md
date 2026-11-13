# 中断：设计规范

状态：**S1、S2、S2+、S3 全部已实现；一个可运行示例见 §14**
依赖：[`isa.md`](isa.md) §4.8、[`binfmt.md`](binfmt.md) §6.6、[`exec.md`](exec.md) §3、[`runtime.md`](runtime.md)
范围：guest 发起的中断、原因模板、宿主裁决 API、仲裁器边界、动态动作集合、完整示例

---

## 0. 一句话

中断是 **CSA 程序主动向宿主提出、必须由宿主裁决才能继续的挂起**。它带着可插值的文本原因
离开 VM；用户配置因此从"一个 backend"变成"backend + 一个仲裁器"，仲裁器与 backend 平级，
同样有 linked / dylib / rpc 三种打包。

裁决是**流**而不是单次回答：一条中断可以被裁决多次（`Unfinished`），直到某一次给出原子的
`Continue` / `Stop`（`Finished`）。完成性是裁决外面的一个独立标记；世界的变化由 `Action`
原子操作承载。`Switch` 不是第三种裁决，它是 `Load` + `Stop` 的组合。

一个可运行的例子是 [`demos/maze`](../demos/maze) 的默认运行：一条中断、两轮裁决、一次
`Switch`；无路标路口的 no-progress 探测、退后和左/右选择。见 §14。

| 部分 | 内容 | 状态 |
|---|---|---|
| **S1** | `INTR`、原因模板与插值、镜像 reason 表、`Session` 的 take/unload/parked、`cyinevm` 多镜像与内置策略 | 已实现 |
| **S2** | `Ruling` / `Completion` / `Action`、裁决流、`cyberspine-arbiter`、linked 打包、`--interrupt` / `--actions` | 已实现 |
| **S2+** | dylib 打包、`Switch` 与动态动作解析、`Silence` / `Restore` | 已实现 |
| **S3** | RPC 仲裁器、非阻塞工作线程、`--decisions` / `--replay` | 已实现 |
| **示例** | `demos/maze` 的默认 `handover`：撞墙探测、`reverse.csa`、`maze_left/right` | 已实现 |

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
CSA（guest）              宿主                              用户配置
INTR reason ──▶ Interrupted ──▶ 票据 + 在途表 ──▶ 仲裁器（linked / dylib / rpc）
                    ▲                                   │
                    │                    Unfinished { actions }*（轮）
                    │                                   │
                    └──────── Finished { actions, verdict } ◀┘
```

- VM 只负责**挂起、交出票据、校验票据仍有效**；
- `Session` 负责**渲染原因、排队票据、应用裁决**；
- 宿主只维护**在途票据表**，把仲裁器回来的裁决逐条应用：先 `actions`，再 `completion`；
- 仲裁器只看到事件、只回 `Ruling`，读改不到实例状态。

"多次裁决"是裁决流变长，不是宿主多了一个状态机：宿主从头到尾只知道"这条事件终结了没有"。

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
  唯一的例外是**被静默**的实例（§8.1）：它仍然存活、仍可裁决，只是不再参与合并。

### 3.3 不变量

- 裁决可以是多轮的，但**轮次不进入 VM**：`Unfinished` 本身不改变实例状态，一轮裁决对世界
  的影响全部由该轮的 `Action` 表达，而不是隐含在"这是第几轮"里。

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

## 6. 宿主 API

### 6.1 票据与队列（S1，已实现）

```rust
pub struct Interrupt {
    pub instance: InstanceId,
    pub pc: usize,          // INTR 本身；恢复点是 pc + 1
    pub tick: u64,
    pub format: Box<str>,   // 模板原文
    pub reason: Box<str>,   // 渲染结果
    reason_id: u32,         // 私有
    seq: u64,               // 私有
}

impl Session {
    pub fn take_interrupts(&mut self) -> Vec<Interrupt>;
    pub fn unload(&mut self, id: InstanceId) -> bool;
    pub fn parked(&self) -> Vec<InstanceId>;
}
```

- **票据（token）语义**：`Interrupt` 无法在 `Session` 之外构造（`seq` 私有，只有 `Session` 产出）。
  `resolve`（6.2）校验 `pending_seq`，同一张票只能用一次；旧票据返回 `Stale`。
- `take_interrupts` 只清投递队列，**不清挂起状态**：没收到的票据不会让实例提前恢复。
- `Continue` 清 `Interrupted`，下一 tick 从 `INTR` 之后继续；`Stop` 卸载，状态丢弃，身体按最后
  命令保持（G10）。
- `unload` 对运行中、挂起中、已结束的实例都可用，是"停止并换动作"的底层操作。
- `parked` 用于宿主在仲裁器失效时 fail-closed。

### 6.2 应用裁决（S2，已实现）

```rust
pub enum Ruling {
    Unfinished,
    Finished(Verdict),      // 仍是原子的 Continue | Stop
}

pub enum Resolution {
    Parked(Interrupt),      // 未完成：票据原样交回，实例仍挂起
    Settled,                // 已完成：票据作废
}

impl Session {
    pub fn resolve(&mut self, ticket: Interrupt, ruling: Ruling)
        -> Result<Resolution, DecideError>;
}
```

- **未完成不是新的实例状态**：实例仍是 `Interrupted`，`Unfinished` 不写任何 VM 字段。
- `Unfinished` 只校验、不改状态：实例在本 session 且 `pending_seq == seq`，然后把票据原样交回。
  `seq` 是唯一身份——`pending_seq` 与 `Interrupted` 是同一处设置、同一处清除的，所以
  "seq 对但状态不对"在外部不可达，代码里只有 `debug_assert`。
- 校验的意义是让宿主**下一轮就发现**实例已被 `unload`（`UnknownInstance`），清掉在途表项，
  而不是等到终局才知道。
- 票据一次性不变：只有 `Finished` 清 `pending_seq`（`Continue`）或卸载实例（`Stop`）；
  终局之后旧票仍返回 `Stale`。
- 不需要 `NotParked` 错误：原因同上，那个状态不可达。

实现：[`crates/cyberspine-vm/src/host.rs`](../crates/cyberspine-vm/src/host.rs)。

---

## 7. 仲裁器边界（S2，已实现）

### 7.1 一条中断 = 一条裁决流

宿主投递一次事件，仲裁器回**若干条**裁决，最后一条带 `Finished`：

```
deliver(event) ──▶ arbiter
                      ├─ { actions, completion: Unfinished }   第 1 轮
                      ├─ { actions, completion: Unfinished }   第 2 轮
                      └─ { actions, completion: Finished(v) }   终局
```

每一轮由两个正交的部分组成：`actions` **塑形世界**（`Load` / `Unload` / `Silence` / `Restore`），
`completion` 说明**票据的命有没有定**。链、按 reason 分流、超时、升级全部是仲裁器内部策略；
宿主不参与，也不知道有几轮——它只维护在途票据表，把回来的裁决逐条应用。

### 7.2 类型

```rust
pub struct InstanceView {
    pub reference: InstanceRef,      // 不回卷的数字句柄
    pub action: Box<str>,
    pub layer: u16,
}

pub struct InterruptEvent {
    pub event: EventId,              // = Interrupt::seq()
    pub instance: InstanceRef,       // 提出中断的实例
    pub pc: usize,
    pub tick: u64,
    pub format: Box<str>,            // 稳定判类 key
    pub reason: Box<str>,            // 渲染文本
    pub action: Box<str>,            // 被打断实例当前动作
    pub instances: Vec<InstanceView>,
    pub actions: Vec<Box<str>>,      // 可 Load 的引用（提示，不是白名单）
    pub image_hash: [u8; 32],
}

pub enum Action {
    Load { action: Box<str> },
    Unload { instance: InstanceRef },
    Silence { instance: InstanceRef },
    Restore { instance: InstanceRef },
}

pub enum Verdict { Continue, Stop }  // 只有两个原子结局

pub enum Completion {
    Unfinished,
    Finished(Verdict),
}

pub struct Ruling {
    pub source: Box<str>,            // 观测用，不进重放
    pub note: Box<str>,              // 观测用，不进重放
    pub actions: Vec<Action>,        // 塑形世界，进重放
    pub completion: Completion,      // 终结标记
}

pub struct Decision { pub event: EventId, pub ruling: Ruling }
pub struct Batch { pub decisions: Vec<Decision>, pub failure: Option<ArbiterError> }

pub trait Arbiter {
    fn deliver(&mut self, event: InterruptEvent); // 不得阻塞
    fn take(&mut self, now: u64) -> Batch;        // 无裁决时返回空；now 是宿主时钟
}
```

`InterruptEvent` 带 `EventId` 而不是 `Interrupt`：票据的 `reason_id` / `seq` 是私有字段，
本来就序列化不了；而且这样仲裁器在类型上**不可能持有或伪造票据**。

`instance` 是提出中断的实例，不是票据：仲裁器可以拿它做 `Silence` / `Restore`，`Stop` 也隐式
指向它。没有这个字段时，仲裁器只能拿 `action` 名去 `instances` 里猜提问者，两个实例跑同一动作
就不成立。

> 命名说明：`Session::resolve` 收的是 `cyberspine-vm::Ruling`（一个只有 `Unfinished` /
> `Finished(Verdict)` 的两态枚举）；`cyberspine-arbiter::Ruling` 是同名结构体，多了
> `source` / `note` / `actions`。VM 那一层不认识动作，宿主把 `Completion` 翻译成
> `vm::Ruling`，把 `actions` 自己执行掉——这正是"VM 不知道动作"的落点。

### 7.3 契约

1. 每个 `deliver` 过的事件，最终**恰好出现一条** `Finished`，之前可以有**任意多条** `Unfinished`。
2. 顺序有意义：同一个 `take` 返回的 `Vec` 按序应用；同一事件在一个 batch 里先 `Unfinished`
   后 `Finished` 是合法的（同步内部链不必为了"一轮一 tick"而缓存）。
3. 实现可以延迟决定；`take` 返回空是正常状态——**静默不等于未完成**。
4. `take(now)` 每个 tick 被调用一次。`now` 是宿主时钟：异步仲裁器用它实现超时，重放用它
   按 tick 释放每一轮。
5. 内部失败（断连、超时）时，实现为自己的每个未决事件合成 `Finished(Stop)`，并通过
   `failure` 报告一次；宿主对仍未终结的事件再兜底一次。
6. 回复引用未知 `event`（伪造或重放）：丢弃并报告。已被 `unload` 的实例仍会收到决定，
   `resolve` 以 `UnknownInstance` / `Stale` 拒绝——票据的一次性由 `seq` 保证。
7. **宿主不设超时、不做链尾兜底**：那是仲裁器的策略。宿主的唯一兜底是 `failure`。

### 7.4 为什么非阻塞

挂起期间身体仍需被 tick 驱动——停止 tick 不等于停住身体，只等于停止更新。仲裁器的延迟
（RPC、AI 裁决）因此不能阻塞 tick 循环。

三种打包都满足：`deliver` 一律只做本地入队；`take` 在 linked / dylib 下是一次进程内调用，
在 rpc 下把这件事交给工作线程，宿主侧只读本地队列。

### 7.5 打包

| 打包 | 机制 | 状态 |
|---|---|---|
| linked | 实现 `Arbiter`，直接返回 `Batch` | 已实现 |
| dylib | `csp_arbiter_v1`，与 backend ABI 同构的 `create/destroy/call/last_error` | 已实现 |
| rpc | 独立于 body 的连接 + 工作线程；`deliver` 投递、`take` 收裁决 | 已实现 |

三类协议共享 [`cyberspine-frame`](../crates/cyberspine-frame/src/lib.rs) 的 12 字节帧头，
各自钉自己的 magic：body 是 `CSPB`，仲裁器是 `CSPA`。所以把 body 帧喂给仲裁器会停在 magic，
不会解码成错误的消息。载荷仍是 MessagePack。

一次交换是一帧 `Tick { seq, now, events }` 换一帧 `Rulings { decisions, failure }`；握手是
`Hello` / `Welcome { version, build_id }`。`seq` 让 ABI 难逃的"缓冲不够、重发一次"变成幂等：
仲裁器缓存上一轮的回复，重发同一个 `seq` 不再投递事件。`source` / `note` / 动作引用各有
`MAX_TEXT_LEN`（4 KiB）上限，超限的一律按协议错误拒绝。

### 7.6 失败策略

- `failure`：宿主为所有在途事件合成一份**配置的默认裁决**（`--on-interrupt`，默认 `Stop`），
  session 不终止。
- 动作装载失败：报错并跳过该动作，`completion` 照常执行；`Load` + `Stop` 的 `Switch` 因此
  fail-closed 成"没有新动作、旧实例已卸载"。
- 仲裁器与 backend 同级可信；它只能回 `Ruling`，读改不到实例状态。

---

## 8. 动作空间与 `Switch`（S2+，已实现）

- `Switch` 不是第三种裁决：它是 `actions: [Load { action }]` + `completion: Finished(Stop)`。
  "先 load 后 unload"由"`actions` 在 `completion` 之前执行"保证，不靠约定。
- `Action` 是原子集：每个操作只动一个对象，失败可跳过，宿主可重放，且都不决定票据的生死。
- `Unload` 不得指向被裁决实例——那是 `Stop` 的职责。`Silence` / `Restore` 可以，`event.instance`
  就是被裁决实例的引用；它是非终局的"停身"：实例退出命令合并、保持存活、仍可裁决。
- 动作集合不固定：`--actions <DIR>` 指向一个目录，`ActionResolver::actions()` 列出它；AI 可以
  在运行期往目录里写新动作，下一次中断的快照就会看到它。
- 事件携带三样东西：`instance`（提问者，供 `Silence` / `Restore` 引用）、`instances`（此刻在跑的
  实例，供 `Unload` / `Silence` / `Restore` 引用）和 `actions`（可以 `Load` 的引用）。都是**中断瞬间的
  快照**，不做目录握手、不做 epoch 同步。
- 动作引用是**不透明字符串**，由宿主的 `ActionResolver` 解释：

```rust
pub trait ActionResolver {
    fn actions(&self) -> Vec<Box<str>>;
    fn resolve(&self, action: &str) -> Result<VerifiedImage, ActionError>;
}
```

- CLI 的实现是"目录 + 文件名"：名字必须是单个路径分量（拒绝 `/`、`\`、`..`），再交给
  `read_image`。这是卫生规则，不是安全边界——仲裁器与 body 同级可信。
- `Load` 由宿主执行：解析 → 读文件 + loader → `Session::load`。装载失败时跳过该动作，
  `completion` 照常执行。`cyberspine-host` 里的 `Interrupts::pump` 是这一段的唯一实现，
  `cyinevm` 和 maze 示例共用它。
- 不通过 MessagePack 传镜像字节：动作集合在任一瞬时是确定的，用引用 + 宿主解析即可。

---

## 9. 确定性

- `resume` 仍是纯函数；恢复序列多一个外部输入：**裁决序列**。
- 记录的是**每一轮**裁决：`(event, tick, actions, completion)`。`tick` 是该轮被应用的 tick，
  不是中断发生的 tick——同一事件的多轮可以落在不同 tick 上，副作用时序才可复现。
- 不记录 `source` / `note`：那是观测数据，与 VM 重放无关。
- 重放按 `tick <= now` 逐轮释放；缺失记录 = 该实例保持挂起，与原运行一致。
- `--decisions <PATH>` 写 JSON Lines，`--replay <PATH>` 读回。
- `--trace` 仍只记每 tick 的命令向量；渲染文本属于运行产物，不进入 ISA 契约。

---

## 10. CLI

maze 的默认运行就是 §14 的示例：

```bash
cargo run -p cyberspine-demo-maze   # 无路标路口：探测、退后、左转
```

`cyinevm` 的同一套边界：

```bash
cyinevm -i idle.csp -i walk.csp --ticks 300
cyinevm -i interrupt.csp --on-interrupt continue --until-halt
cyinevm -i interrupt.csp --actions actions/ --interrupt dylib:fallback.dll --decisions run.jsonl
cyinevm -i interrupt.csp --replay run.jsonl --trace
```

- `-i` 可重复，顺序 = 装载顺序；共享一个时钟与一个 body。
- `--interrupt dylib:<path>|rpc:<host>:<port>` 选择仲裁器；省略时用内置单轮仲裁器。
- `--on-interrupt stop|continue` 现在是**兜底默认裁决**：仲裁器整体失效时宿主用它回答。
- `--actions <DIR>` 给仲裁器一个可 `Load` 的动作目录。
- `--decisions <PATH>` 记录每一轮裁决；`--replay <PATH>` 用记录代替仲裁器。
- 内置仲裁器是 `Unfinished` 恒不出现的特例，不是链的简化版。
- 中断报告走 stderr（实例、路径、pc、tick、插值后的原因）；退出码不变。
- maze 与 `cyinevm` 共用 [`cyberspine-host`](../crates/cyberspine-host) 的在途表/动作/兜底逻辑；
  maze 只是把同一份 `Report` 画到屏幕上，并在 `--headless` 时打到 stderr。

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
| 渲染、票据、`resolve` / `unload` / `parked`、静默 | [`crates/cyberspine-vm/src/host.rs`](../crates/cyberspine-vm/src/host.rs) |
| 裁决流、`InterruptEvent` / `Ruling` / `Action` / `Batch` / `Arbiter` | [`crates/cyberspine-arbiter/src/lib.rs`](../crates/cyberspine-arbiter/src/lib.rs) |
| 帧头（`CSPB` / `CSPA` 共用） | [`crates/cyberspine-frame/src/lib.rs`](../crates/cyberspine-frame/src/lib.rs) |
| ABI、dylib、rpc、线上消息 | [`crates/cyberspine-arbiter/src/`](../crates/cyberspine-arbiter/src/) 的 `abi.rs` / `dylib.rs` / `rpc.rs` / `wire.rs` |
| 在途票据表、`failure` 兜底、动作应用 | [`crates/cyberspine-host/src/interrupts.rs`](../crates/cyberspine-host/src/interrupts.rs) |
| 实例表（`InstanceId` ↔ `InstanceRef`） | [`crates/cyberspine-host/src/registry.rs`](../crates/cyberspine-host/src/registry.rs) |
| 宿主观测流 `Report` | [`crates/cyberspine-host/src/report.rs`](../crates/cyberspine-host/src/report.rs) |
| 动作目录 | [`crates/cyberspine-host/src/actions.rs`](../crates/cyberspine-host/src/actions.rs) |
| 决策日志与重放 | [`crates/cyberspine-host/src/decisions.rs`](../crates/cyberspine-host/src/decisions.rs) |
| `--interrupt` / `--actions` / `--decisions` 的 CLI 接线 | [`crates/cyberspine-runtime/src/run.rs`](../crates/cyberspine-runtime/src/run.rs)、[`cli.rs`](../crates/cyberspine-runtime/src/cli.rs) |
| 示例：maze 的无路标路口 | [`demos/maze/src/arbiter.rs`](../demos/maze/src/arbiter.rs)、[`actions.rs`](../demos/maze/src/actions.rs)、[`maze.csa`](../demos/maze/maze.csa)、[`reverse.csa`](../demos/maze/reverse.csa) |

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
| 每个事件恰好一条 `Decision` | 改为 `Unfinished* + Finished`：多轮不必让宿主重投递事件 |
| 链是宿主的配置 | 链下沉为仲裁器实现；宿主只维护在途表，不设超时、不做链尾兜底 |
| `InterruptEvent` 携带票据 | 改为 `EventId` + 只读字段；票据留在宿主，仲裁器不可能持有或伪造 |
| `InterruptEvent` 不带提问者引用 | `Silence` / `Restore` 只能按 action 名猜；补 `instance: InstanceRef`，`Stop` 仍隐式 |
| `Verdict::Switch` 当第三种裁决 | `Switch` 是 `Load` + `Stop`；原子集与完成性拆成两个正交维度 |
| 每个 dylib 调用只执行一次 | ABI 在缓冲不够时会重发同一请求；加 `seq` 缓存，让重发幂等 |
| 重放只记终局裁决 | 一轮 `Unfinished` 也可能带 `actions`；按轮记录、按 tick 释放 |

---

## 14. 一个可运行示例：maze 的无路标路口

默认的 `cargo run -p cyberspine-demo-maze` 就是这条流：**一条中断、两轮裁决、一次
`Switch`**。故事不是“程序主动问方向”，而是它先真的走不动了：无路标、直行是墙，连续几个 tick
只有 `bumps` 增长，才把票据交给宿主。

### 14.1 场景

- 默认迷宫在 `(4,11)` 去掉红标、把 `(5,11)` 封成墙；北边是正解，南边是死路。颜色规则在
  `(4,11)` 失效，`maze.csa` 只能先直行撞墙。
- `maze.csa` 的 `L_stuck`：`.var_i turn = 0` 时进入探测循环，连续 4 个 tick 按 `forward`
  （`moves` 不动、`bumps` 涨到 4），然后
  `INTR "no progress: x=… y=… heading=… bumps=…"`。
- `Junction` 第 1 轮：`Unfinished`，`actions: [Silence(event.instance), Load("reverse.csa")]`，
  note “the unsigned junction: backing off”。`reverse.csa` 是一拍 `backward`，把身体从
  `(4,11)` 退到 `(3,11)`；提问者仍挂起。
- 第 2 轮（`now > interrupt_tick + 3`）：`Finished(Stop)`，
  `actions: [Load("maze_left.csa")]`，note “turning left”。如果左转变体又卡住，仲裁器下一次
  会切到 `maze_right.csa`。
- 两个方向变体是同一份 `maze.csa`，只把 `.var_i turn = 0` 换成 `1` / `2`；`L_turn_left` /
  `L_turn_right` 转一次、把 `turn` 置回 0，然后回到普通颜色规则。默认地图 left 是正解，
  right 是兜底路径。

### 14.2 时序

```text
tick 3   maze: at (4,11), no marker, straight is a wall -> probe
tick 7   probe: forward x4, moves unchanged, bumps=4 -> INTR
         host: take_interrupts -> 1 ticket; take(now=8)
         arbiter: Unfinished + Silence + Load("reverse.csa")
tick 8   reverse: backward one tick -> (3,11); HALT
tick 11  host: take(now=11) -> Finished(Stop) + Load("maze_left.csa")
         maze_left: (3,11) -> (4,11) at the end of the tick
tick 12  maze_left: at (4,11), wall ahead, turn=1 -> turn left
tick 13  maze_left: north into the colour path; ... goal
```

`reverse` 先把身体退开一格，第 2 轮的新实例再从 `(3,11)` 重新进入 `(4,11)`，所以“退后”和
“左/右”各自有独立的 tick 边界，画面能看见。

### 14.3 输出

`--headless` 把 `Report` 流打到 stderr：

```text
interrupt: instance 0 (maze.csa) at pc 49 tick 7: no progress: x=4 y=11 heading=1 bumps=4
interrupt 0: junction: the unsigned junction: backing off
interrupt 0: finished after 1 round(s), last: junction: the unsigned junction: backing off · the unsigned junction: turning left
  goal reached in 40 ticks: 26 moves, 6 turns, 4 bumps
```

屏幕底部固定 3 行裁决日志（在棋盘和 legend 下面，日志增减不会顶动地图）：

```text
  ruling  interrupt 0: no progress: x=4 y=11 heading=1 bumps=4
  ruling  interrupt 0 round 1: the unsigned junction: backing off
  ruling  interrupt 0 finished: the unsigned junction: turning left
```

这 3 行保留到下一次裁决；`--headless` 的 stderr 是完整流。odometer 的构成是：4 个 bump 来自探测，2 个 move 来自退后和重新进入，
turns 仍是 6。

### 14.4 宿主侧

maze 没有自己的仲裁循环：它和 `cyinevm` 共用 [`crates/cyberspine-host`](../crates/cyberspine-host)
的 `Registry` + `Interrupts::pump` + `Report`。`pump` 每个 tick 做四件事：

1. 把 `Session::take_interrupts` 的票据变成 `InterruptEvent`，投给仲裁器，记在在途表；
2. `arbiter.take(now)` 拿回 `Batch`；
3. 每条 `Decision` 先应用 `Action`，再应用 `Completion`；`Unfinished` 重新挂起，`Finished`
   消耗票据；
4. 仲裁器整体失效时，给所有在途票据合成兜底裁决。

maze 只多做了两件事：用内存里的 `ActionResolver` 把 `"reverse.csa"` / `"maze_left.csa"` /
`"maze_right.csa"` 解析成现场汇编的 `VerifiedImage`，以及把 `Report` 画到屏幕上。`cyinevm`
用目录解析动作、把 `Report` 打到 stderr，其余完全一样。

### 14.5 文件

| 内容 | 位置 |
|---|---|
| 无路标地图 | [`demos/maze/world/src/grid.rs`](../demos/maze/world/src/grid.rs) |
| no-progress 探测、方向变体 | [`demos/maze/maze.csa`](../demos/maze/maze.csa) |
| 退后动作 | [`demos/maze/reverse.csa`](../demos/maze/reverse.csa) |
| 两轮仲裁器 | [`demos/maze/src/arbiter.rs`](../demos/maze/src/arbiter.rs) |
| 内存动作解析 | [`demos/maze/src/actions.rs`](../demos/maze/src/actions.rs) |
| 宿主装配 | [`demos/maze/src/session.rs`](../demos/maze/src/session.rs) |
| 测试 | [`demos/maze/tests/arbiter.rs`](../demos/maze/tests/arbiter.rs) |

仲裁器只做 linked。dylib / rpc 打包由 `cyinevm --interrupt dylib:…|rpc:…` 覆盖；宿主侧仍是
同一个 [`cyberspine-host`](../crates/cyberspine-host) 循环。
