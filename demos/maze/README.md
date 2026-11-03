# 迷宫 demo

这个 demo 展示 CyberSpine 的基本工作方式。

一个小人在 21×15 的迷宫里走。它没有地图，只靠脚下地面的颜色和眼前的墙决定下一步：

- 红色 → 左转
- 蓝色 → 右转
- 没有颜色 → 直行
- `@` → 停下

动作程序是 [`maze.csa`](maze.csa)，身体是 [`src/backend.rs`](src/backend.rs) 里的 `MazeBackend`，
两者共享的名字表是 [`maze.rig.json`](maze.rig.json)。

## 这个 demo 展示的工作方式

1. **程序只写名字。** 动作读 `marker_red`、写 `turn_left`；它不知道坐标、墙或迷宫形状，这些名字由 rig 提供。
2. **身体是普通 Rust。** 迷宫、位置、朝向、碰撞都在 `MazeBackend` 里。它每个 tick 被问一次状态、收一次命令、推进一步，双方都看不到对方的内部状态。
3. **交换只发生在 tick 边界。** 程序看到的是一份冻结的 sensor 快照，身体拿到的是一条合并后的命令向量；没有共享内存，也没有回调。
4. **名字在汇编期绑定。** 汇编器按 rig 把名字解析成下标，并把 rig 指纹写进镜像；宿主加载时比对后端指纹，不一致就拒绝运行。
5. **换身体不换动作。** 同一份 `MazeBackend` 有三种打包：链接进本进程、编译成动态库、跑在另一个进程里用 TCP 服务。动作、镜像和画面一模一样，见下一节。
6. **观察者也只能看 rig。** 画面里的位姿、脚下颜色、墙和里程计全部来自 rig；地形和走过的轨迹属于观察者。

## 三种打包

```bash
# 1. 链接：身体和宿主一起编译
cargo run -p cyberspine-demo-maze

# 2. 动态库：同一个 body 的 cdylib，加载到本进程
cargo build -p cyberspine-demo-maze
cargo run -p cyberspine-demo-maze -- -b dylib:self

# 3. RPC：身体在自己的进程里监听，宿主连上去
cargo run -p cyberspine-demo-maze --bin maze-serve        # 终端 A
cargo run -p cyberspine-demo-maze -- -b rpc:127.0.0.1:7777 # 终端 B
```

`dylib:self` 是 demo 的简写：它加载和当前二进制同目录、由同一个包编出的动态库
（`libcyberspine_demo_maze.so` / `.dylib` / `cyberspine_demo_maze.dll`）。真正的写法仍然是
`dylib:<path>`，和 `cyinevm -b` 一致。`cargo run` 只为二进制编 rlib，所以先跑一次
`cargo build` 才会有那个动态库；缺库时错误信息会给出期望的路径。

`maze-serve` 打完一行 `maze body serving on ...` 就等宿主连接。一个进程服务一次运行：
协议没有 reset，身体会保留上一次被驱动到的位置。

三种模式都只是换 `Backend`，宿主和动作不知道区别。加 `--headless` 可以不画画面、直接跑完并打印汇总，
CI 和脚本用这个：

```bash
cargo run -p cyberspine-demo-maze -- --headless -b dylib:self
```

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

在终端里，每 250 ms 走一步，画面原地刷新；大约 8 秒后小人停在 `@`，最后一行是：

```text
goal reached in 31 ticks: 24 moves, 6 turns, 0 bumps
```

退出码 `0`。如果它停在别处（撞墙、卡住），说明引导没有走通，程序以 `1` 退出。

重定向到文件时不会原地刷新，而是按 tick 顺序打印每一帧。节奏由动作自己的 `.clock hz = 4` 决定，
而观察者每帧最多推进一个 tick，这样它记录的轨迹才是连续的。

三种打包的最后一行和每一帧都应当相同；`tests/backends.rs` 逐帧比对链接、动态库和 RPC 的输出。
