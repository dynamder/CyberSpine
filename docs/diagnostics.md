# 诊断

状态：v1（汇编器、加载器、渲染器已接入；`cyinevm` 的运行期映射见 §8）
依赖：[`asm.md`](asm.md) §6、[`binfmt.md`](binfmt.md) §9、[`runtime.md`](runtime.md)
规定：诊断的词汇、码、位置归属、输出形状与退出码

---

## 0. 三条不变量

1. **产生 ≠ 渲染。** 除 `cyberspine-report`（诊断）与建在其上的 `cyberspine-run-report`（运行事件与终态）外，任何 crate 都不把诊断变成字节。
   库 crate 里的 `println!`/`eprintln!` 正在清零，见 §8 的例外清单。
2. **失败不是返回值，是 sink 事件。** 阶段函数返回"产物有没有"，为什么没有进
   `Diagnostics`。`Diagnostics::failed()` 是唯一判定点。
3. **上下文由知道它的人附加。** 后端只知道 socket，主机才知道 tick；主机包后端。
   层级不颠倒。

## 1. 词汇（`cyberspine-diag`）

```rust
pub enum Level { Note, Warning, Error }     // 有序：max 即最严重

pub struct Code(&'static str);              // "asm.unknown-mnemonic"

pub enum Site {
    Source      { line: u32, column: u32 },
    Byte        { offset: u64 },
    Instruction { pc: u32 },
    Session     { instance: u32, tick: u64 },
    Whole,                                  // 只有归属物，没有位置
}

pub struct Anchor { artifact: Box<str>, site: Site }

pub struct Diagnostic {
    level, code, message,
    primary: Option<Anchor>,
    labels: Vec<Label>,                     // 可以在别的 artifact 里
    notes: Vec<String>,
    help: Option<String>,
}

pub struct Cause { code, message, help }    // 跨界插件唯一能产的东西
```

- **`Anchor` 把归属物和位置绑在一起。** 不存在"假定只有一个文件"的状态。
  fault 的指令在镜像里、源行在另一个文件里——这不是例外，是常态。
- **`artifact` 是解析后的路径**，可被直接打开。调试段存的是相对镜像的路径，
  解析发生在产生诊断的那一层，不留给下游再拼一次。
- **`Cause` 没有位置字段。** 插件不知道 tick，类型上就不允许它假装知道。

## 2. 码

码是 `namespace.name`，是机器契约；消息是散文，可以改。

| 命名空间 | 所有者 | 例 |
|---|---|---|
| `asm` | `cyberspine-asm` / `cyinea` | `asm.unknown-mnemonic`、`asm.pending-leak` |
| `binfmt` | `cyberspine-binfmt` | `binfmt.C6`、`binfmt.M4`、`binfmt.I1` |
| `exec` | `cyberspine-exec` | `exec.fuel-exhausted` |
| `rt` | 宿主 / `cyinevm` | `rt.trace-write` |
| `backend` | 身体边界 | `backend.transport` |
| `arbiter` | 仲裁边界 | `arbiter.protocol` |

- **码是粗粒度的规则类，变体是细粒度的。** `asm.bad-operand` 覆盖缺操作数、缺逗号、
  类型不符三种变体；一个变体一个码会让每次加规则都要改文档。
- **没有全局码表。** 每个 crate 用 `codes!` 从自己的错误枚举生成 `Coded` 与 `CODES`：

  ```rust
  codes! {
      LoadError: Error {
          Self::TooShort { .. }        => "binfmt.C1",
          Self::ContentHashMismatch{..} => "binfmt.C6",
      }
  }
  ```

  `Coded::code()` 没有默认实现，`match` 由宏生成：新增变体不写码编译不过。
  唯一性是**每 crate** 的（`table_is_sane`），因为依赖图已经把三个领域隔开了——
  `cyinea` 链接不到 `binfmt` 的码，反之亦然。
- `binfmt` 保留规范里的 C/M/I 编号，不重排。

## 3. 位置归属与调试信息

调试信息不是字符串格式化，是**坐标系之间的翻译**。

- `DebugInfo::site(pc)` 是唯一翻译入口，返回结构（文件、行、列），不返回字符串。
- **翻译在产生者做一次**，结果作为 `Label` 附在诊断上；渲染器永不翻译。
- 路径解析是翻译的一部分：`image_dir.join(debug.source())`。
- 加载期**不做**翻译：加载时调试段自己还是嫌疑对象（C9），拿它解释另一条错误是循环论证。
  所以 `binfmt.*` 的诊断只有 `Byte` / `Instruction` / `Whole`，没有 `Source`。

降级是契约，不是错误：

| 情况 | 结果 |
|---|---|
| `--strip` | 只有 `Instruction{pc}`，没有 Source label |
| 源码不在机器上 | Source label 在，画不出代码框，只显示 `路径:行:列` |
| pc 不在 line map（隐式 HALT） | 没有 Source label |
| 镜像与源码一起移动 | 路径相对镜像解析，仍然正确 |

任何一条都不许让运行失败；JSON 用 `null`，不用缺字段。

## 4. sink

```rust
let mut sink = Diagnostics::new(deny_warnings);
sink.push(Diagnostic::of(&kind).at(anchor));
if sink.failed() { /* 唯一的停止点 */ }
```

- 上限 64 条（`DEFAULT_LIMIT`），溢出计入 `dropped()`。两个 CLI 把 `dropped()` 交给渲染器：
  人读末尾是 `note: N more diagnostic(s) were not shown`，JSONL 末尾是一条 `kind:"truncated"`。
- `--deny-warnings` 是 sink 的一个构造参数，不是第二条代码路径。
- **sink 用于会恢复并继续的多错阶段**（解析一行错后继续下一行）。运行期失败是终止条件，
  不是集合：`cyinevm` 用 `Result` 表示"这次运行停了"，在 `main` 一次性转换成诊断。
  关键不在于用什么类型，而在于**只有一处把错误变成字节**。

## 5. 输出（`cyberspine-report` / `cyberspine-run-report`）

诊断渲染在 `cyberspine-report`；中断流与运行终态在 `cyberspine-run-report`。拆开是因为后者
依赖 host/arbiter，而 `cyinea` 只编译、不运行镜像，不应链接插件边界。

**流固定，格式可变：诊断永远走 stderr，产物永远走 stdout。**

| 模式 | stderr | stdout |
|---|---|---|
| 人读（默认） | 诊断（`annotate-snippets`） | `--trace` / `--report` 的产物 |
| `--json` | 诊断（JSON Lines） | 同上，不变 |

`--json` 只改格式，不改流。这样 `--trace`（TSV）与 `--report json` 不会和诊断抢同一个
句柄；`cyinea` 的 stdout 在两种模式下都是空的。`cyinevm --json` 隐含 `--quiet`：启动摘要
是写给人看的，放在 JSONL 流里就是唯一一行解析不了的东西。

一条诊断一个对象：

```json
{"schema":1,"kind":"diagnostic","level":"error","code":"asm.unknown-mnemonic",
 "message":"unknown mnemonic `FADDD`","primary":{"artifact":"bad.csa","kind":"source",
 "line":7,"column":5},"labels":[],"notes":[]}
```

- `schema` 只在删除字段或改变含义时 +1；允许加字段。
- `code` 稳定，`message` 不稳定。
- `Site::Whole` 序列化成 `{"kind":"whole"}`；`primary` 缺失时字段省略。

列表被 sink 的上限截断时，`--json` 在诊断之后追加一条：

```json
{"schema":1,"kind":"truncated","dropped":37}
```

人读模式对应一行 `note: 37 more diagnostic(s) were not shown`。`kind` ∈ `diagnostic` /
`truncated`（运行流另有 `event` / `run`，见下）。

诊断或事件流写不进 stderr（管道关闭、重定向目标不可写）是环境失败，退出 5；`cyinevm`
仍先写出 `--report` 的终态，再返回这个码。

中断与裁决也是一条一条的记录，随运行实时写出（`crates/cyberspine-run-report/src/event.rs`）：

```json
{"schema":1,"kind":"event","what":"interrupt","event":0,"instance":0,
 "action":"action.csp","pc":0,"tick":0,"reason":"no progress",
 "site":{"artifact":"probe.csa","kind":"source","line":8,"column":5}}
```

`what` ∈ `interrupt` / `round` / `finished` / `unknown` / `abandoned` / `action` / `carry` /
`verdict` / `failed`。事件不是诊断：中断是正常通信，裁决是宿主对它的回答；它们共用同一个
渲染器，只是为了让 `--json` 用一个词汇描述整场运行。

运行终态是另一条记录，由 `--report json` 写出（`crates/cyberspine-run-report/src/render.rs`）：

```json
{"schema":1,"kind":"run","outcome":"halted","ticks":31,"dt":0.016666668,
 "overruns":0,"instances":[{"id":0,"layer":0,"action":"target/hello.csp"}]}
```

`outcome` ∈ `budget`（`--ticks` 用完）、`halted`（`--until-halt` 满足）、`stopped`（Ctrl-C、
trace 管道关闭）、`faulted`、`rejected`（输入被拒，退出 3）、`environment`。**fault 与
rejected 也写报告**：失败是最需要终态的结局。`outcome` 描述的是**运行本身**怎么结束：诊断
或裁决日志写不进去只会把退出码变成 5，不会把 `faulted` 改写成 `environment`。

## 6. 退出码

| 码 | 含义 |
|---|---|
| `0` | 成功 |
| `2` | 命令行用法错误（clap） |
| `3` | 输入被拒绝：源码、manifest、`-o` 指向源文件、生成的镜像超过 4 MiB |
| `4` | 运行异常结束：fault |
| `5` | 环境失败：读写不了文件、诊断写不进 stderr、后端/仲裁器失败 |

`cyinea` 只产生 `0/2/3/5`。CLI **不得 panic**：参数组合矩阵测试
（`crates/cyberspine-runtime/tests/cli_matrix.rs`）钉住"永不 101"。

## 7. 防腐

| 失败模式 | 机制 |
|---|---|
| 加变体忘写码 | `Coded::code` 无默认 + 宏生成穷尽 match → 编译失败 |
| 码名不成形 / 跨命名空间 | `table_is_sane`，每 crate 一个测试 |
| 错误在产生点变成句子 | `Diagnostic` 是唯一载体 |
| 库 crate 直接打印 | CI grep（待加） |
| 输出格式悄悄变 | `schema` 整数 + 渲染测试 |
| 外部插件码冲突 | `backend.` / `arbiter.` 前缀 |

## 8. 通知（`NoticeSource`）

不是所有要说的话都是诊断。回放与录制不一致时，运行**成功**了，但结果不可复现：

```
warning[rt.replay-unused]: 3 recorded round(s) were never applied
  --> runs/maze.jsonl
  help: the run diverged from the recording: …
```

这条链路刻意和 `Arbiter` 分开：

- `cyberspine-arbiter` 有 `NoticeSource` trait（`take_notices`）和 `Notice { cause, artifact }`。
  `Arbiter` 保持两个方法——裁决是每个仲裁器都要做的，通知不是。
- `cyberspine-host` 的 `Replay::report()` 给出一个共享计数器的 `ReplayReport`；`Replay`
  被装进 `Box<dyn Arbiter>` 之后，这个句柄仍然活着。
- `open_arbiter` 返回 `Installed { arbiter, notices: Option<Box<dyn NoticeSource>> }`，
  runtime 在运行结束后取一次，`into_diagnostic` 把它变成 `Level::Warning`。
- 只在 `Budget` / `Halted` 结局取：Ctrl-C 截断或运行失败时，"有轮次没用上"是预期的噪声。
- 取走语义：`take_notices` 第二次调用为空，不会重复报告。

这条通道不改变退出码。要 CI 硬失败，再加 `--deny-warnings`，不要提前发明退出码。

## 9. 尚未接入

- `BackendError` / `ArbiterError` 已经携带 `Cause`：body 和 arbiter 可以给自己的失败起
  `backend.<name>.*` / `arbiter.*` 的码，默认是 `backend.failed` / `arbiter.failed`。
  仲裁器的失败跨线时带码；`BackendError` 仍保留 `std::error::Error` 源链，
  所以它是一个 `{ cause, source }` 而不是裸 `Cause`——丢掉源链换一个码不划算。
- 库内剩下的 4 处打印都发生在**没有调用者可以接收错误**的边界上：C ABI 的 `create`
  （返回 null 时只能写 stderr）、RPC 服务端的连接日志。要清零得先给它们各自一条上报通道。
- `exec.fuel-exhausted` 是唯一的 `exec.*` 码，因为 `Fault` 目前只有一个变体。
- CI grep 要等那 4 处清零后才能加。
