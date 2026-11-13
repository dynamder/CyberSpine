# `cyinevm` 运行时命令行

状态：v1
依赖：[`isa.md`](isa.md) §8.4（宿主每 tick 流程）、[`backend.md`](backend.md)（三种后端）、[`binfmt.md`](binfmt.md)（`.csp`）、[`asm.md`](asm.md) §6.1（工具命名）
范围：`cyinevm` 的参数、生命周期、pacing、trace、退出码，以及静态链接自定义 body 的方式

---

## 0. 定位

`cyinea` 把 `.csa` 汇编成 `.csp`；`cyinevm` 把 `.csp` 跑在一具 body 上。两者是独立进程，
不合并成单个二进制（`asm.md` §6.1）。

body 有两种来源：

| 来源 | 是否需要 `--backend` | 何时确定 |
|---|---|---|
| **linked** | 不需要 | 编译期；body 是 `cyinevm` 二进制的一部分 |
| **动态库 / RPC** | 需要 | 运行期；由 `--backend dylib:…` / `rpc:…` 指定 |

`dylib` 与 `rpc` 是 `cyberspine-runtime` 的 feature；linked body 不需要它们。

---

## 1. 用法

```text
cyinevm -i <IMAGE> [-i <IMAGE> ...] [-b <SPEC>] [--on-interrupt stop|continue]
        [--ticks N | --until-halt] [--unpaced] [--trace] [-q]
```

| 参数 | 含义 |
|---|---|
| `-i, --image <IMAGE>` | 要运行的 `.csp`；重复给出即按顺序装载到同一个 session，必填 |
| `-b, --backend <SPEC>` | `dylib:<path>` 或 `rpc:<host>:<port>`；省略时使用官方 linked dummy |
| `--on-interrupt <STOP\|CONTINUE>` | 内置中断策略：`stop`（默认，卸载该实例）或 `continue`（下一 tick 继续） |
| `--ticks <N>` | 恰好运行 N 个 tick 后退出；`0` 表示只加载不运行；与 `--until-halt` 互斥 |
| `--until-halt` | 所有实例结束后退出；与 `--ticks` 互斥 |
| `--unpaced` | 不按真实时间睡眠；必须与 `--ticks` 一起使用 |
| `--trace` | 每个 tick 向 stdout 写一行命令向量 |
| `-q, --quiet` | 不打印启动摘要；dummy warning、fault 与中断报告仍然打印 |

多个 `-i` 共享同一个时钟与同一个 body：每个镜像必须与第一个镜像的 `hz` 和 shape 一致，
装载顺序就是恢复顺序与合并顺序（`isa.md` §8.4）。`--ticks 0` 会装载全部镜像再退出，
适合一次性校验。

退出码：

| 码 | 含义 |
|---|---|
| `0` | `--ticks`/`--until-halt` 正常结束、Ctrl-C、trace 管道关闭；中断本身不是错误 |
| `1` | 镜像读取/校验/加载失败、后端握手或 tick 失败、fault、feature 缺失、信号处理器安装失败 |
| `2` | 命令行用法错误（clap 的约定） |

---

## 2. 后端

### 2.1 省略 `--backend`：官方 linked dummy

官方 `cyinevm` 编译进一个零 body（`linked::Dummy`）。它的 shape 从镜像的名字表重建：
muscles、`SensorSource::Backend` 的 `sensors_f` / `sensors_i`。

dummy 不可能是镜像的 rig：镜像只有指纹，没有 manifest 文本，而 `BoundFingerprint` 刻意
没有从 `[u8; 32]` 构造的入口。因此这条路径使用 `Host::permissive()`，并在 stderr 打印：

```text
warning: no --backend given; using the built-in dummy body (rig check disabled)
```

这条 warning **不受 `--quiet` 影响**。如果需要严格 rig 校验，请使用外部 body 或静态链接
真实 body（§7）。

### 2.2 `dylib:<path>`

加载一个导出 `csp_backend_v1` 的 `cdylib`。需要 `dylib` feature。

### 2.3 `rpc:<host>:<port>`

连接到实现 v1 线协议的 body。需要 `rpc` feature。地址也接受 `rpc://host:port` 的写法。

### 2.4 `dummy:` 不是 spec

dummy 是内置的，不需要也不接受 `dummy:` 前缀；`cyinevm -b dummy:…` 是用法错误，错误信息
会提示省略 `--backend`。

---

## 3. 生命周期

### 3.1 默认：常驻到 Ctrl-C

默认模式按镜像头段的 `hz` 持续 tick，直到收到 Ctrl-C。

**HALT 不会结束进程。** 动作结束后实例被移除，合并向量变成全 `None`，但宿主继续
`sample` / `commit` / `step`；这是 `isa.md` §8.4 的“保持”语义：停止 tick 等于停止驱动身体。

**中断不会结束进程。** `INTR` 让该实例挂起，`cyinevm` 在 tick 之后裁决：内置仲裁器用
`--on-interrupt stop`（默认）把它卸载、`--on-interrupt continue` 让它下一 tick 从 `INTR`
之后继续；`--interrupt dylib:<path>` 或 `--interrupt rpc:<host>:<port>` 换成外部仲裁器，
`--actions <DIR>` 给它一个可 `load` 的动作目录。事件本身（实例、路径、pc、tick、插值后的
原因）打印到 stderr，例如：

```text
interrupt: instance 0 (actions/idle.csp) at pc 3 tick 12: obstacle: x=0.32 y=0.90
```

退出码仍是 `0`：中断是程序与宿主之间的正常通信，不是错误。`--decisions <PATH>` 把每一轮
裁决记成 JSON Lines，`--replay <PATH>` 用记录代替仲裁器重跑同一段轨迹。模型与边界见
[`interrupt.md`](interrupt.md)。maze 的默认运行是同一套 `cyberspine-host` 循环的完整示例
（无路标路口 → 撞墙探测 → `reverse.csa` 退后 → `maze_left/right` 变体），见
[`interrupt.md`](interrupt.md) §14 与 [`demos/maze/README.md`](../demos/maze/README.md)。

Ctrl-C 由 `ctrlc` 捕获，设置停止标志；当前循环结束后进程以 `0` 退出。

### 3.2 `--ticks N`

恰好运行 N 个 tick 后退出。`--ticks 0` 只加载、不运行，适合校验镜像和 body 是否匹配。

这个模式会停止驱动身体；它面向测试、CI 和一次性动作，不是“保持姿态”的用法。

### 3.3 `--until-halt`

所有实例都结束后退出。它同样会停止驱动身体；文档和 `--help` 都明确这一点。

因为实时模式通过 `Session::advance` 追帧，`--until-halt` 在帧边界检查，最多可能多跑
一个补帧上限（当前 8 tick）。

### 3.4 `--unpaced`

不睡眠，连续 tick。只允许与 `--ticks` 一起使用，避免无界忙等。

---

## 4. Pacing

- 默认与 `--until-halt`：每帧睡眠 `dt = 1/hz`，把真实经过的时间交给
  `Session::advance`；accumulator 的余量留在宿主，永不进入 VM。
- `--ticks` 实时模式：按 deadline 精确跑 N 次 `Session::tick`；tick 变慢时由 deadline
  自然追赶，不改变任何 tick 的内容。
- 追赶上限是 `Session` 的 `max_ticks_per_frame`（默认 8）；超过时丢弃余量并计数，
  这是宿主策略，不进入 VM（`isa.md` §6.3）。
- `--unpaced`：直接 `Session::tick`，不读真实时间。

虚拟时间只由镜像的 `hz` 定义；实时走快或走慢不改变任何 tick 的语义。

---

## 5. trace

`--trace` 向 stdout 写 TSV；启动摘要和 fault 走 stderr，因此管道只包含数据。

```text
tick	brow_l	brow_r
0	0.5	-
1	0.5	-
2	-	-
```

- 第一行是表头；列顺序 = 镜像的 muscles 顺序。
- 每行第一个字段是 `now`（tick 计数）。
- `-` 表示本 tick 没有实例发布这条肌肉（合并向量的 `None`），**不是 0**。
- 浮点按 Rust `f32` 的 Display 输出：`0.5`、`-0`、`NaN`、`inf`、`-inf`。
- trace 是观察格式，不是重放格式：NaN payload/符号不属于 ISA 契约（`isa.md` G9），
  不进入 trace。
- 内部 ring 容量为 64，每个 tick/帧排空一次；`trace_dropped` 非 0 时 stderr 警告。
- 下游关闭管道（例如 `| head`）时，`cyinevm` 以 `0` 退出。

---

## 6. Feature gate

`cyberspine-runtime` 的 features：

```toml
default = ["dylib", "rpc"]
dylib   = ["cyberspine-backend/dylib", "cyberspine-arbiter/dylib"]
rpc     = ["cyberspine-backend/rpc", "cyberspine-arbiter/rpc"]
```

默认构建的 `cyinevm` 同时包含两种外部后端和两种外部仲裁器。缺少某个 feature 时，指定
对应后端或仲裁器会在打开前失败：

```text
error: this `cyinevm` was built without the `rpc` feature, so it cannot connect to a remote body
help: rebuild with `cargo build -p cyberspine-runtime --features rpc`
```

`dylib` 同理。`dummy` 和静态链接 body 不需要这两个 feature。

宿主侧的 `Registry` / `Interrupts` / `Report` / 动作目录 / 决策日志在
[`cyberspine-host`](../crates/cyberspine-host)；`cyberspine-runtime` 只做 CLI 接线和 feature 选择。
maze 示例复用同一份循环；它的 `cyberspine-host` 不依赖这两个 feature（body 的三种打包仍由
`cyberspine-backend` 的 feature 决定）。

只构建静态版本：

```bash
cargo build -p cyberspine-runtime --no-default-features
```

这样产出的 `cyinevm` 不含 `libloading` / `tokio`。`cyberspine-backend` 的共享 wire/server
模块仍会编译，但 `dylib`/`rpc` 的实现及其依赖被排除。

---

## 7. 静态链接自定义 body

官方 linked body 是 `linked::Dummy`；`linked::Official` 是指向它的类型别名。要链接真实
body，实现 `LinkedBody` 并把别名改成自己的类型：

```rust
use cyberspine_runtime::linked::{LinkedBody, Notice};
use cyberspine_runtime::VerifiedImage;
use cyberspine_vm::{BackendError, Strict};

pub struct MyBody;

impl LinkedBody for MyBody {
    type Body = MyBackend;
    type Policy = Strict;
    const NOTICE: Notice = Notice::Silent;

    fn open(_: &VerifiedImage) -> Result<MyBackend, BackendError> {
        MyBackend::new().map_err(BackendError::new)
    }
}

pub type Official = MyBody;
```

要点：

- `Body` 和 `Policy` 是关联类型，linked 路径没有 `Box<dyn Backend>`，也没有 `bool` 开关。
- 真实 body 使用 `Strict`；官方 dummy 使用 `Permissive` 并带 `Notice::Warn`。
- `Notice::Silent` 不打印 warning；`Notice::Warn(text)` 在启动时打印到 stderr。
- 用 `--no-default-features` 构建，得到不含 `dylib`/`rpc` 代码的 `cyinevm`。

---

## 8. 非目标（v1）

- `--permissive`：外部 body 始终 strict；dummy 的 permissive 是类型 `Policy` 决定的，
  不是命令行开关。
- `inspect` / 反汇编：加载失败的报错已经包含 rig 与 shape；调试信息是 `asm.md` §8 的独立任务。
- 自动重连、热插拔、动作调度：属于上层，不属于运行时。

---

## 9. 例子

```bash
# 官方 dummy：校验镜像能否运行，跑 120 tick 并输出 trace
cyinevm -i action.csp --ticks 120 --unpaced --trace > trace.tsv

# 两个镜像装进同一个 session；顺序就是装载顺序
cyinevm -i idle.csp -i walk.csp --ticks 300

# 中断默认停掉实例；改成继续
cyinevm -i action.csp --on-interrupt continue --until-halt

# 动态库 body
cyinevm -i action.csp -b dylib:bodies/robot.dll

# RPC body，常驻到 Ctrl-C
cyinevm -i action.csp -b rpc:127.0.0.1:7777

# 静态链接 body 的构建
cargo build -p cyberspine-runtime --no-default-features
```
