# 迷宫 demo

这个 demo 展示 CyberSpine 的基本工作方式和中断仲裁。

一个小人在 21×15 的迷宫里走。它没有地图，只靠脚下地面的颜色和眼前的墙决定下一步：

- 红色 → 左转
- 蓝色 → 右转
- 没有颜色 → 直行
- `@` → 停下

默认运行里有一个**没有路标的三岔口**：`(4,11)` 没有颜色，直行 `(5,11)` 是墙，左边是正解，
右边是死路。颜色规则在那里失效，程序只能先压着 `forward` 撞几 tick 墙，发现 `moves` 不动、
只有 `bumps` 在涨，然后 `INTR`；仲裁器先让它退后，等几个 tick，再加载一个“左转”或“右转”的
程序变体。

动作程序是 [`maze.csa`](maze.csa) 和 [`reverse.csa`](reverse.csa)，身体是
[`world/src/backend.rs`](world/src/backend.rs) 里的 `MazeBackend`，两者共享的名字表是
[`maze.rig.json`](maze.rig.json)。`maze_left.csa` / `maze_right.csa` 不是独立文件：宿主在装配时
把 `maze.csa` 的 `.var_i turn = 0` 换成 `1` / `2`，得到同一程序的两个方向变体。

## 目录

demo 按职责拆成四个 crate；边界就是 `cargo -p` 的包名：

| 路径 | 包 | 职责 |
|---|---|---|
| [`world/`](world/) | `cyberspine-demo-maze-world` | 身体：迷宫、`MazeBackend`、rig。没有宿主、终端或传输。 |
| [`dylib/`](dylib/) | `cyberspine-demo-maze-dylib` | 把身体编成 `cdylib`；产物只有动态库。 |
| [`rpc/`](rpc/) | `cyberspine-demo-maze-rpc` | 把身体跑成进程：`maze-serve` 监听 TCP。 |
| 根目录 | `cyberspine-demo-maze` | 宿主：`maze.csa` / `reverse.csa`、linked 仲裁器、`Session` 和终端渲染。 |

`world` 是唯一的身体实现；`dylib` 和 `rpc` 都只是它的打包，宿主只认 `Backend`。
`maze.csa`、`reverse.csa` 和 `maze.rig.json` 放在根目录，因为它们是「动作 + 身体契约」这对输入，
不属于任何一层。宿主侧的仲裁循环不是这个 demo 私有的：它来自
[`cyberspine-host`](../../crates/cyberspine-host)，和 `cyinevm` 用的是同一份。

## 这个 demo 展示的工作方式

1. **程序只写名字。** 动作读 `marker_red`、写 `turn_left`；它不知道坐标、墙或迷宫形状，这些名字由 rig 提供。
2. **身体是普通 Rust。** 迷宫、位置、朝向、碰撞都在 `MazeBackend` 里。它每个 tick 被问一次状态、收一次命令、推进一步，双方都看不到对方的内部状态。
3. **交换只发生在 tick 边界。** 程序看到的是一份冻结的 sensor 快照，身体拿到的是一条合并后的命令向量；没有共享内存，也没有回调。
4. **名字在汇编期绑定。** 汇编器按 rig 把名字解析成下标，并把 rig 指纹写进镜像；宿主加载时比对后端指纹，不一致就拒绝运行。
5. **换身体不换动作。** 同一份 `MazeBackend` 有三种打包：链接进本进程、编译成动态库、跑在另一个进程里用 TCP 服务。动作、镜像和画面一模一样，见下一节。
6. **观察者也只能看 rig。** 画面里的位姿、脚下颜色、墙和里程计全部来自 rig；地形和走过的轨迹属于观察者。
7. **中断由 guest 的“走不动了”触发。** 不是程序主动问“我该往哪走”，而是它先真的试了：连续几个 tick 压着 `forward`，`moves` 不变、`bumps` 增加，确认没有进展后才把票据交给宿主。
8. **仲裁器先退后、再决定方向。** 第 1 轮 `Unfinished` 让它退后，等几个 tick；第 2 轮才 `Finished`，加载左/右变体。轮次不进入 VM，身体也不知道发生过仲裁。

## 三种打包

```bash
# 默认：无路标路口 → 退后 → 左转
cargo run -p cyberspine-demo-maze

# 1. 链接：宿主链接 world 的 rlib
cargo run -p cyberspine-demo-maze

# 2. 动态库：dylib crate 编出真正的 cdylib，宿主在运行时加载
cargo build -p cyberspine-demo-maze-dylib
cargo run -p cyberspine-demo-maze -- -b dylib:self

# 3. RPC：身体在自己的进程里监听，宿主连上去
cargo run -p cyberspine-demo-maze-rpc -- --listen 127.0.0.1:7777 # 终端 A
cargo run -p cyberspine-demo-maze -- -b rpc:127.0.0.1:7777         # 终端 B
```

`dylib` 那条路是真正的动态库：`cyberspine-demo-maze-dylib` 只产出一个 `cdylib`，没有可供误链接的
rlib；宿主用 `libloading` 打开它，之后每次调用都过 C ABI 的 `csp_backend_v1`
（[`docs/backend.md`](../../docs/backend.md) §5）。`maze` 二进制与这个库没有编译期依赖。

`dylib:self` 是 demo 的简写：它按平台找到和当前二进制同目录的
`libcyberspine_demo_maze_dylib.so` / `.dylib` / `cyberspine_demo_maze_dylib.dll`。真正的写法仍然是
`dylib:<path>`，和 `cyinevm -b` 一致：

```bash
cargo run -p cyberspine-demo-maze -- -b dylib:target/debug/cyberspine_demo_maze_dylib.dll
```

（Windows 的路径；Linux / macOS 是同一目录下的 `libcyberspine_demo_maze_dylib.so` / `.dylib`。）

`cargo run` 只为二进制编 rlib，所以先跑一次 `cargo build -p cyberspine-demo-maze-dylib` 才会有动态库；
缺库时错误信息会给出期望的路径。

`maze-serve` 打完一行 `maze body serving on ...` 就等宿主连接。一个进程服务一次运行：
协议没有 reset，身体会保留上一次被驱动到的位置。

三种模式都只是换 `Backend`，宿主和动作不知道区别。加 `--headless` 可以不画画面、直接跑完并打印汇总，
CI 和脚本用这个：

```bash
cargo run -p cyberspine-demo-maze -- --headless
```

## 中断与仲裁

`(4,11)` 是那个没有路标的三岔口：

- 从起点向东走 3 格到达 `(4,11)`；
- 脚下没有红/蓝标记，直行 `(5,11)` 是墙；
- 北边是正解，南边是死路。

### 1. no-progress 探测

`maze.csa` 的 `L_stuck` 先检查 `.var_i turn`：普通镜像 `turn = 0`，于是进入探测循环：

```text
L_stuck:
    按 forward，FLUSH，YIELD，释放
    probe += 1
    if probe < 4: 回到 L_stuck
    读 x/y/heading/bumps
    probe = 0
    INTR "no progress: x={i1} y={i2} heading={i3} bumps={i6}"
```

4 个 tick 里身体只涨 `bumps`（`moves` 不动），这就是“发现多 tick 没有行进”。原因文本把
`(4,11)`、朝向和 bump 次数一起交出去，宿主不需要再看 body。

### 2. 第 1 轮：退后

`Junction` 的第 1 轮是 `Unfinished`：

- `Silence { instance: event.instance }`：提问者先别发声；
- `Load { action: "reverse.csa" }`：一个单 tick 的 `backward` 动作，把身体从 `(4,11)` 退到 `(3,11)`；
- note 是“the unsigned junction: backing off”。

实例仍然挂起，票据留在宿主的在途表里；`reverse.csa` 跑完就 `HALT`，身体停在 `(3,11)`。

### 3. 等几个 tick

`Junction` 用 `take(now)` 把第 2 轮压到 `now > interrupt_tick + 3`。这 3 个 tick 里身体不被
新命令驱动，正好让“退后”看得见；仲裁器不阻塞 tick 循环。

### 4. 第 2 轮：左转或右转

第 2 轮是 `Finished(Stop)`，动作是 `Load { action: "maze_left.csa" }`（第一次）或
`"maze_right.csa"`（如果左转变体又卡住，仲裁器会切到右转）。两个变体都是同一份 `maze.csa`，
只是 `.var_i turn` 初值是 `1` / `2`：

- 新实例从 `(3,11)` 走到 `(4,11)`，再次看到“无标记 + 直行墙”；
- `turn = 1` 走 `L_turn_left`，转一次并向北进入正解；`turn = 2` 走 `L_turn_right`，向南进入死路；
- 转完后把 `turn` 置回 0，之后恢复普通颜色规则。

`Stop` 结束原来的提问者，`Load` 出来的变体接管身体。left 是默认地图的正解，所以默认 demo 一轮就成功；
right 是兜底路径，文档保留它作为“仲裁器真的在选方向”的证据。

### 5. 输出

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

这 3 行会一直保留到下一次裁决；`--headless` 的 stderr 是完整流。

两个容易看漏的点：

- **为什么探测要先 release 再 `INTR`？** 合并发生在 `resume` 之后、同一 tick 的 `step` 之前；
  挂起实例的最后发布向量仍会参与合并。探测循环在最后一个 tick 已经释放了 `forward`，否则身体会在
  中断那一 tick 继续往墙里顶。
- **为什么还要 `Silence`？** 程序 release 是 guest 的纪律；`Silence` 是宿主对“这个实例先别发声”的
  显式保证。它也是非终局的“停身”：实例仍存活、仍可裁决，只是退出命令合并。

仲裁器只做 linked。想换成动态库或 RPC 仲裁器，用 `cyinevm --interrupt dylib:…|rpc:…`；宿主侧
仍是同一个 [`cyberspine-host`](../../crates/cyberspine-host) 循环，见
[`docs/interrupt.md`](../../docs/interrupt.md) §7 和 §14。

## 跨边界时看到什么

`x`、`y`、`heading`、`moves`、`turns`、`bumps` 都在 rig 里（所以 `maze.rig.json` 是 v2）。
这不是凑数：远端身体只会报出 rig 声明的量，观察者能画的也就是这些。宿主每绘制一帧会向远端多发一次
`Sample`；协议里 `sample` 是纯读，允许在 tick 之外调用，代价是一帧一次往返，不是一 tick 一次。

地形和轨迹不在 rig 里。观察者用 demo 自带的地图作画，并按 tick 记录它看见过的位置，所以远端模式下
轨迹是**观察者**的记忆；如果远端身体换了地图（`MazeBackend::with_grid`），画面上的墙就对不上了。

## 预期行为

```bash
cargo run -p cyberspine-demo-maze
```

在终端里，每 250 ms 走一步，画面原地刷新；第 3 tick 到达 `(4,11)`，4 tick 撞墙探测后在第 7 tick
`INTR`；退后、等 3 tick，第 11 tick 起由 `maze_left` 变体接管，向北转，走完剩下的路线。大约 10 秒后
小人停在 `@`，最后一行是：

```text
goal reached in 40 ticks: 26 moves, 6 turns, 4 bumps
```

退出码 `0`。探测本身带来 4 个 bump，退后和重新进入带来 2 个 move；turns 仍是 6。

如果它停在别处（撞墙、卡住、仲裁器没有回答），说明引导没有走通，程序以 `1` 退出。

重定向到文件时不会原地刷新，而是按 tick 顺序打印每一帧。节奏由动作自己的 `.clock hz = 4` 决定，
而观察者每帧最多推进一个 tick，这样它记录的轨迹才是连续的。

三种打包的最后一行和每一帧都应当相同；`tests/backends.rs` 逐帧比对链接、动态库和 RPC 的输出，
并比对裁决流，还直接打开动态库确认它的 `build_id` 来自 `cyberspine-demo-maze-dylib`。
`tests/arbiter.rs` 钉住默认场景恰好一条中断、第一轮 `reverse.csa`、第二轮 `maze_left.csa`、
最终 odometer `(26,6,4)`。
