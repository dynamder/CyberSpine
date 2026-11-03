# cyber-spine 后端集成规范（链接 / 动态库 / RPC）

状态：v1
依赖：[`isa.md`](isa.md) §9（`Backend` trait）、[`exec.md`](exec.md)（tick 循环）、[`rig.md`](rig.md) §4（身份）、[`binfmt.md`](binfmt.md)（镜像）

> **实现状态（2026-11）**：三种方式全部落地。`crates/cyberspine-backend` 提供
> `wire` / `remote` / `server` / `dylib` / `rpc` / `spec`，`crates/cyberspine-backend-fixture`
> 是三种打包共用的测试 body。本文件是这三种方式的唯一规范。

---

## 0. 一句话

**同一个 `impl Backend`，三种打包。** 区别只在"代码在哪"和"一次调用怎么过去"；每 tick 的语义完全相同。

## 1. 三种方式

| 方式 | 进程 | 语言 | 每 tick 代价 | 什么时候用 |
|---|---|---|---|---|
| **链接** | 同 | Rust | 直接调用，零拷贝 | 默认。后端随宿主一起编译、一起分发；没有 spec |
| **动态库** | 同 | 任意（C ABI） | 一帧进、一帧出，各一次拷贝 | 插件化、版分解耦、工具链不同 |
| **RPC** | 跨 | 任意 | 两个网络往返 | 设备守护进程、引擎、跨机 |

选择准则：**能不跨进程就不跨。** 跨进程只为一件事买单——代码可以独立部署、独立语言、独立崩溃域。
动态库和 RPC 的差别只是传输，之上的字节完全一样，因此两者共用同一套一致性测试。

## 2. 三种方式共享的不变量

1. **每 tick 一个后端瞬间。** `sample` 一次报齐全部可观测量与肌肉读回（`isa.md` §9.2）。
2. **命令向量是纯函数。** `None` = 本 tick 没有实例发布这条肌肉 = 保持上一条命令，不是释放。
3. **身份只传名字，不传哈希。** 跨边界的后端报出它那份 **manifest 文本**；宿主自己解析、校验、
   算指纹（`rig.md` §4）。`BoundFingerprint` 没有 `From<[u8; 32]>`，这条不变量在类型上成立。
4. **失败是终态。** 第一个错误被锁存，之后每次调用立即返回它，不再碰传输层。
5. **值按位传输。** f32 以 IEEE-754 位模式过边界，不经过十进制、不经过 JSON。

## 3. 线上协议 v1

### 3.1 帧

```
偏移  长度  字段
0     4     magic = "CSPB"
4     2     version u16（当前 1）
6     1     kind u8
7     1     flags u8 = 0（未定义位一律拒绝）
8     4     payload_len u32
12    n     payload
```

- **帧头固定小端。** payload 是 MessagePack，其多字节标量按 MessagePack 规定是**大端**——两者不同，
  实现时不要混。
- 数据帧上限 1 MiB；`Welcome.rig` 上限 64 KiB。
- 一帧 = 一个消息。请求一帧、应答一帧，严格同步。
- 未知 kind、非零 flags、长度不符：拒绝。

### 3.2 消息

应答沿用请求的 `kind`；拒绝永远是 `kind = 0x7f`。

| kind | 值 | 请求 payload | 应答 payload |
|---|---|---|---|
| `Hello` | 1 | `Hello { version, client }` | `Welcome { version, rig, build_id }` |
| `Sample` | 2 | `Sample { seq }` | `SampleReply { seq, f, i, muscles }` |
| `Commit` | 3 | `Commit { seq, dt, mask, values }` | `Ack { seq }` |
| `Step` | 4 | `Step { seq, dt }` | `Ack { seq }` |
| `Error` | 0x7f | —（只作为应答） | `Failure { seq, code, message }` |

`seq` 是单调递增的请求号，应答原样带回；不一致就断链。它不是安全机制，是失步检测。

`Failure.code`：`1 = PROTOCOL`（帧/顺序/形状错）、`2 = BODY`（后端拒绝该调用）、`3 = PANIC`。

### 3.3 规范性

- payload 用 **MessagePack**，结构体编码为 **array**（位置编码，字段顺序 = 定义顺序），不是 map。
- 整数取最短表示；字符串 UTF-8；`f32` 是 MessagePack `float32`（`0xca`）4 字节大端。
- **`Commit` 的规范形式**：`mask` 为 0 的位，对应 `values` 必须是 `+0.0` 的位模式；最后一个
  word 里超出肌肉数的位必须为 0。一条命令向量因此只有一个编码，trace 可比较、可哈希。
- **黄金字节向量**在 `crates/cyberspine-backend/tests/wire.rs`：格式真相钉在仓库里，不钉在某个
  编解码 crate 的存亡上。

### 3.4 版本

- 帧头 `version` 不等：直接拒绝，不回 `Error`（对端可能读不懂应答）。
- `Hello.version` 不等：回 `Error { code: PROTOCOL }`，然后断开。
- 动态库另有 **ABI 版本**（§5），与协议版本无关。

## 4. 失败与超时

- 传输层第一次失败（断连、超时、坏帧、后端返回 `Err`）即锁存；**不重试**。
- RPC 单次 exchange 默认 **1 s** 超时；超时按失败处理。
- 宿主侧：`Session::tick` / `Session::advance` 返回 `Err`，调用方必须停止 tick。
- 一次失败的 tick 可能已经推进了实例、但没有交付身体——这是终态，不是可重试信号（`isa.md` §9.2）。

## 5. 动态库 ABI v1

唯一导出符号：

```c
const CspAbiV1 *csp_backend_v1(void);

struct CspAbiV1 {
    uint32_t version;   // 1
    uint32_t size;      // sizeof(CspAbiV1)，按插件编译时的布局
    void *(*create)(const char *config);
    void  (*destroy)(void *body);
    int32_t (*call)(void *body, const uint8_t *req, size_t req_len,
                    uint8_t *resp, size_t resp_cap, size_t *resp_len);
    const char *(*last_error)(void *body);
};
```

- `call` 返回：`0 OK`、`1 RESPONSE_TOO_SMALL`（`*resp_len` = 需要的大小）、`2 PROTOCOL`、
  `3 BODY`、`4 PANIC`。缓冲不够是两段式协议，不是错误。
- `create` 失败**仍返回句柄**，原因放进 `last_error`；宿主因此能说"manifest 与 body 不符"，
  而不是"被拒绝"。只有连句柄都分配不出来时才返回 null。
- **panic 在边界上被捕获**（`export!` 生成的 `catch_unwind`），返回 `4`。不在 `extern "C"` 里
  展开是硬要求。
- **先 `destroy`，后卸载库。** 函数表指向库内，`Drop` 顺序是显式写死的。
- 插件侧只需要宏：

```rust
cyberspine_backend::export! {
    body: MazeBackend,
    rig: include_str!("maze.rig.json"),
    create: |_config| MazeBackend::new(),
}
```

## 6. RPC v1

- **宿主是客户端，后端监听。** 身体通常属于某个拥有设备的进程，让它监听比让宿主监听更自然。
- TCP；`TCP_NODELAY`；默认 1 s 超时（`connect_with_timeout` 可改）。
- 一个连接一个 body，一次服务一个连接，请求顺序处理。
- **无认证、无 TLS**：前提是回环或可信网络。要跨不可信网络，需要的正是沙箱路线（`isa.md` §9.5），
  不属于 v1。
- 每 tick **两个往返**（`Sample`；`Commit` + `Step`）：宿主必须在两者之间恢复实例，无法合并。
  RTT 决定**可达 tick 率**，不改变任何 tick 的内容——这正是 `isa.md` §9.4 早已论证的性质。

### 6.1 代价

每一帧的 payload 都解码成新的 `Vec`（`SampleReply` 三条、`Commit` 两条），跨边界的 tick 因此
每 tick 约五次小分配。这是**适配器**的代价，不是宿主的：`Session::tick` 自身仍然零分配。
若将来需要 1 kHz 以上的远程身体，热路径可以手写零分配解码，线格式不变。

## 7. 一致性

同一个 body、同一份镜像，三种打包必须产生**逐位相同**的 trace：

| 测什么 | 在哪 |
|---|---|
| 帧与消息的黄金字节 | `tests/wire.rs` |
| 握手、粘性失败、错 kind、形状不符 | `tests/remote.rs` |
| 链接 vs 动态库 同 trace | `tests/dylib.rs` |
| 链接 vs RPC 同 trace | `tests/rpc.rs` |
| spec 语法与缺 feature 报错 | `tests/spec.rs` |

## 8. 非目标（v1）

重连 / 热插拔 / 服务发现；TLS / 认证；一个 session 多个 body；`Commit`+`Step` 合并 ack；
异步宿主（tick 循环保持同步，运行时装在传输层里）；设备侧 RPC。

## 9. 实现位置

| 模块 | 内容 |
|---|---|
| `cyberspine-backend::wire` | 帧头、消息、MessagePack 编解码、`Commit` 规范性 |
| `cyberspine-backend::remote` | `Transport`、`RemoteBody`、握手、粘性失败 |
| `cyberspine-backend::server` | body 侧：一帧请求 → 一帧应答；manifest 与 body 的一次性比对 |
| `cyberspine-backend::dylib` | `libloading` 加载、`Drop` 顺序、`export!` |
| `cyberspine-backend::rpc` | tokio 客户端与服务端；运行时关在 `TcpTransport` 里 |
| `cyberspine-backend::spec` | `dylib:` / `rpc:` 的解析与打开；linked 没有 spec |

## 附录 A：非 Rust 实现速查

每个消息是一个 MessagePack array，元素按定义顺序：

```
Hello       = [version:u16, client:str]
Welcome     = [version:u16, rig:str, build_id:str]
Sample      = [seq:u32]
SampleReply = [seq:u32, f:[f32], i:[i32], muscles:[f32]]
Commit      = [seq:u32, dt:f32, mask:[u64], values:[f32]]
Step        = [seq:u32, dt:f32]
Ack         = [seq:u32]
Error       = [seq:u32, code:u16, message:str]
```

`mask` 的第 `m` 位对应第 `m` 个肌肉（LSB first）。
