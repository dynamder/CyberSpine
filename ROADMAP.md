# 路线图

> 草案，2026-11-15。`docs/` 是已实现的规范；本文件只写未做的部分、顺序与未定事项。

## 1. 当前位置

| 层 | 已交付 | 规范 |
|---|---|---|
| 语言与格式 | 指令表、`.csp` 镜像、装配期名字绑定、rig 指纹 | [`isa.md`](docs/isa.md) [`binfmt.md`](docs/binfmt.md) [`asm.md`](docs/asm.md) |
| 执行 | 定长 tick、sensor 快照、命令合并、fuel、`WAIT`/`YIELD`/`FLUSH` | [`exec.md`](docs/exec.md) |
| 宿主 | 多实例、tick 循环、trace、fault、动作目录 | [`runtime.md`](docs/runtime.md) |
| 中断 | `INTR`、原因模板、票据、多轮裁决、`Load`/`Silence`/`Restore` | [`interrupt.md`](docs/interrupt.md) |
| 边界 | body 与仲裁器各三种打包；`--decisions` / `--replay` | [`backend.md`](docs/backend.md) |
| 工程 | 三平台 CI、MSRV 1.89、`thumbv7em` no_std、QEMU 固件、fuzz | `.github/workflows/ci.yml` |

核心已足够支撑快路径；缺“写 → 跑 → 看 → 改”的回路与第一具真实身体。

## 2. 前提

来自产品文档 `ai-game.md`（不在本仓库）：

1. 实时性优先于推理能力：反应时间超过上限，决策再聪明也没有效果。
2. 快慢双路径：快路径质量越高，慢路径能被容忍的延迟越大。
3. 代码是 LLM 的母语，也是输入设备：csa 必须能现场写、现场编译、现场替换。
4. 感知不在实时链路里解决：全程录制，事后把时序数据交给慢路径分析。

## 3. 里程碑

顺序：先闭环，再扩面。H0、H1 与具体游戏无关；H2 才需要选定身体。

### H0 迭代回路 ⬜ 0.5–1 周

缺源行与机器可读结果：csa 只能盲写。

| 编号 | 交付 | 验收 |
|---|---|---|
| R0.1 ✅ | `cyinea` 默认产出 debug 构建：镜像尾部内嵌调试段（`flags` bit 0，标签表 + `pc → 行:列`）；`--strip` 产出 release 构建 | debug / release 执行一致，哈希不同；`--strip` 的字节等于 release |
| R0.2 ✅ | `cyinevm` 在 fault 与 `INTR` 事件中渲染 `file:line:col`；`--trace` 是冻结的观察格式，不动 | 测试钉住 `(probe.csa:8:5)` 形式的输出 |
| R0.3 ✅ | `cyinevm --report json`：终态（`budget`/`halted`/`stopped`/`faulted`/`environment`）、tick 数、实例列表；诊断统一 `--json`（stderr JSONL），退出码分 `3/4/5` | 端到端用例钉住两条记录与三个退出码 |
| R0.4 | 助记符前缀一致性进 `cyinea`（[`isa.md`](docs/isa.md) §11） | 违反规则的镜像被拒绝 |

依赖：无。格式定在 [`binfmt.md`](docs/binfmt.md) §6.7。

### H1 可观察性 ⬜ 2–3 周

感知只有一条路：录制 + 事后分析。`--trace` 是观察格式，`--decisions` 只记裁决，两者都不带世界状态。

| 编号 | 交付 | 验收 |
|---|---|---|
| R1.1 | `--record <dir>`：单一时间轴 `timeline.jsonl`（tick、sensor 快照、合并命令、裁决），可选画面帧 | 录制不向 body 多发 `sample`，不改变任何 tick 的内容 |
| R1.2 | 用录制中的输入重放，得到同一条轨迹 | 与 `--replay` 分开：一个回放裁决，一个回放 run |
| R1.3 | `InterruptEvent` 携带世界状态（rig 名字 + 值），`reason` 文本保留给人 | 仲裁器读到数值，而不只是格式化字符串 |
| R1.4 | 导演通道：本地 JSON 控制面 `state` / `switch <action>` / `pause` / `param` | 无中断也能换动作；慢路径能主动探查 |

依赖：R0.3。
R1.3 改动 arbiter 线格式：升 `ABI_VERSION`，或引入 v2 帧。

### H2 第一具真实身体 ⬜ 3–4 周

唯一需要选定游戏的里程碑。

| 编号 | 交付 | 验收 |
|---|---|---|
| R2.1 | 平台选型（D1、D2） | 决策记录写进 §5 |
| R2.2 | body crate：平台输入 / 状态做成 `Backend` + rig manifest | 指纹校验通过，`cyinevm` 能直接驱动 |
| R2.3 | 标准动作骨架：轮询 sensor、no-progress 探测、`INTR` 模板（从 maze demo 抽出） | 新动作从模板起步 |
| R2.4 | 延迟基线：loopback → 真实，报 p50/p99 | 数字进 `--report json` |

依赖：H1。
验收：4 个动作（走 / 打 / 跳 / 撤）+ 一次真实的“掉血 → fallback → 慢路径 → 切动作”。

### H3 慢路径接缝 ⬜ 1–2 周

| 编号 | 交付 | 验收 |
|---|---|---|
| R3.1 | 规则型 arbiter 参考实现（无网络、可测试） | 覆盖 H2 的四类中断 |
| R3.2 | 模型型 arbiter：超时降级到规则 | 模型不可用时 tick 不延长 |
| R3.3 | 决策模型接口（Jev 一类），不绑定具体实现 | 换模型只换一个 crate |

依赖：R1.3。

### H4 复盘闭环 ⬜ 1 周

| 编号 | 交付 | 验收 |
|---|---|---|
| R4.1 | run 归档：录制 + 裁决 + 目标 + 结局 | 一次 run 是一个目录，可搬移 |
| R4.2 | 分析包：时间轴摘要，直接喂 LLM | 同一 run 两次生成结果一致 |
| R4.3 | 回写规范：人 / LLM 同一条流程 | 文档 + 一个示例 run |

依赖：H1、H2。

### H5 未排期

- 屏幕-only 感知（无 mod / 无内部状态的游戏）。
- 多游戏 body 抽象与标准 rig 库（键盘 / 鼠标 / 手柄）。
- ISA 扩展：`.data` 数据段、`INX` 索引读、`CALL`/`RET`。
- 冻结 `isa_version`、发布 crates。
- 性能项：单块 arena、取消边界检查（[`exec.md`](docs/exec.md) §11）。

## 4. 明确不做

[`isa.md`](docs/isa.md) §12 之外，本路线追加：

- 调度器 / 动作选择。
- 全局变量、`spawn`/`await`、`FEQ`。
- 实时视觉感知（多模态推理进 16 ms 闭环目前不成立）。
- 游戏内存修改 / 注入作弊：若要做，单独文档，不进主路线。
- 实时把世界状态推给慢路径：录制替代，见 §2 第 4 条。

## 5. 未定事项

| 编号 | 问题 | 影响 | 建议 |
|---|---|---|---|
| D1 | 目标游戏 | H2 全部 | Minecraft / Fabric：能拿到可信状态 |
| D2 | 是否允许读游戏内部状态 | 感知质量 = 快路径上限 | 第一版允许；屏幕-only 放 H5 |
| D3 | 画面帧是否进 v1 录制 | 存储与帧率 | 先 PPM，每秒几帧 |
| D4 | 慢路径模型部署形态（本地 / 云） | H3 的延迟与降级策略 | 本地规则 + 远端模型 |
| D5 | 输入注入层级（游戏内 API / `SendInput` / 虚拟 HID） | 兼容性与工程风险 | 游戏内 API 优先；OS 层放 H5 |

## 6. 风险

| 风险 | 触发条件 | 缓解 |
|---|---|---|
| 感知质量不足 | sensor 少或抖 | 选能读状态的游戏；sensor 先窄而准 |
| 延迟预算被击穿 | 裁决 / 模型同步阻塞 tick | arbiter 是非阻塞模型；R3.2 强制降级 |
| 输入被游戏拦截 | OS 层注入 | 游戏内 API 优先 |
| 回归无法自动化 | 真游戏依赖人工 | 录制回放进 CI |
| token 成本 | 每次中断都问模型 | 规则 / 决策模型挡在前面 |
| 工程失控 | 三个子系统并行 | H0 / H1 与游戏解耦 |

## 7. 变更记录

- 2026-11-15：草案：H0–H4 的范围与验收；D1–D5 未定。
- 2026-11-16：R0.1 / R0.2 落地；调试信息改为内嵌调试段（`binfmt.md` §6.7），不再用 `.cspd` 旁路。
- 2026-11-21：诊断重构（[`diagnostics.md`](docs/diagnostics.md)）：单一 `Diagnostic` 载体、每 crate 稳定码、`cyberspine-report` 唯一渲染器；`cyinea`/`cyinevm` 的 `--json` 与 `--report json`；退出码改为 `3/4/5`；fault/INTR 的源行路径按镜像目录解析（此前打印的是相对路径）。R0.3 完成。
- 2026-11-21：body 与 arbiter 的失败携带 `Cause`（码 + 消息 + help）；仲裁器失败跨线带码（预发布阶段不 bump 线上版本）。中断/裁决事件流搬进 `cyberspine-report`，`--json` 下与诊断、终态同一套记录。`Code` 支持运行时拥有的字符串，插件可以给自己的失败起名。
