# cyber-spine 汇编源格式（`.csa`）

状态：v1（随 isa 冻结）
依赖：[`isa.md`](isa.md)（指令编码与语义）、[`binfmt.md`](binfmt.md)（可执行镜像 `.csp`）
范围：汇编源码的词法、语法、伪指令、标签解析、汇编期校验与调试信息

---

## 0. 定位

- 输入：UTF-8 文本 `.csa`；输出：`Image`（`crates/cyberspine-core/src/binfmt/image.rs`）→ `.csp` 字节，外加 `DebugInfo { labels, line_map }`。
- 汇编器**只读 manifest，不接触 rig 的实现**。manifest 声明肌肉、可观测量和身份（[`rig.md`](rig.md)）；名字在装配期解析成 **rig 的物理下标**，直接写进 `imm`（`isa.md` §13.2）。镜像因此与 rig 一一对应，加载期用指纹校验。
- 指令语法由 `OpCode::signature()` 派生；助记符表由 `OpCode::ALL` + `mnemonic()` 反查生成。**加一条指令只改 `crates/cyberspine-core/src/isa.rs`，不手抄任何清单。**
- 装配期完成：标签→偏移、名字→物理下标、`group` 展开、隐式 `HALT` 补齐、所有静态可判定的校验。

## 1. 词法

| 元素 | 形式 |
|---|---|
| 注释 | `#` 到行尾 |
| 标识符 | `[A-Za-z_][A-Za-z0-9_]*`，**大小写敏感** |
| 寄存器 | `f0`..`f63` / `i0`..`i63`（`f0`/`i0` 硬连零，写丢弃） |
| 十进制整数 | `42` `-42`（带符号，范围 i32） |
| 非十进整数 | `0x1F` `0b1010` `0o17`：**按 u32 位模式**，重解释为 i32（`0xFFFFFFFF` = `-1`） |
| 浮点字面量 | `1.5` `-0.25` `1e-3` `inf` `-inf` `nan` |
| 字符串 | `"..."`；只有 `\\` 与 `\"` 是转义。目前只用于 `.rig` |
| 标签 | `ident` 后跟 `:` |
| 伪指令 | `.` 开头 |

- 整数允许 `_` 分隔（`1_000_000`、`0xFF_FF`）；浮点是否允许 `_` **未定义**（`tests/text.rs` 中对应用例保持 `#[ignore]`）。
- 助记符/伪指令**大小写不敏感**（`fadd` == `FADD`）；标签/名字**大小写敏感**。
- `fN`/`iN` 是保留形状：不能做标签或名字。
- `dt`、`tick` 是内置 sensor 名，保留，不得作为肌肉/sensor/变量名。
- `;` 保留（非注释），留作将来可能的语句分隔符。

## 2. 行结构

```
line   ::= label* (instr | directive | comment)? comment?
label  ::= ident ":"
instr  ::= mnemonic operand ("," operand)*
```

- 一条指令一行；`label: instr` 允许同行；一行可堆多个标签。
- 声明型伪指令必须全部出现在**第一条指令之前**；`.text` 只是可选标记行。

## 3. 头段伪指令

### 3.1 标量

```
.layer 10                          # 层号 u16；宿主用它做命令合并排序键
.clock hz = 60                     # tick 率，> 0；dt = 1/hz
.fuel 4096                         # 每次恢复的指令预算
.max_wait 600.0                    # 单次 WAIT 上界（虚拟秒）
```

- `.clock` 只有 `hz` 一个字段（周期计费表已退役，见 `isa.md` §6）。
- **当前实现（v1）要求四个标量全部显式给出**；缺失字段在声明段结束处报
  `MissingField`（四个缺失会一次报全）。未来若改为可省，必须在这里定义各自缺省值。

### 3.2 rig 与名字

```asm
.rig "rigs/live2d-standard.rig"      # JSON manifest，路径相对本文件

.group   brows = brow_l, brow_r      # 汇编期展开；只在 FEXCITE 位置可用
.var_f   phase = 0.0                 # 实例变量，f32
.var_i   state = 0                   # 实例变量，i32
```

- **肌肉和可观测量的声明不在源码里**。它们由 manifest 提供（[`rig.md`](rig.md)）；源码只引用名字。
- `.rig` 给出 manifest 路径（相对**本文件所在目录**）；`cyinea --rig <path>` 可覆盖它，同样相对本文件。两者都没有时汇编失败。
- 旧伪指令 `.muscle` / `.sensor_f` / `.sensor_i` 已删除，源码里出现即 `unknown directive` —— 名字只来自 manifest。
- `dt` / `tick` 由语言内置，不在 manifest 里，也不能被声明。
- `.group` v1 **不嵌套**；成员必须是 manifest 里存在的肌肉名。
- 引用了 manifest 中不存在的名字 → **汇编错误**（不是运行期的 VOID）。

### 3.3 表序

镜像的表由 manifest 生成，`dt` / `tick` 追加在末尾（`binfmt.md` §3/§6）：

```
muscles   = [manifest.muscles 序]
sensors_f = [每个肌肉的读回项，按 manifest.muscles 序] ++ [manifest.sensors_f 序] ++ [dt]
sensors_i = [manifest.sensors_i 序] ++ [tick]
vars_f / vars_i = 源码声明序
```

顺序即物理下标，因此 manifest 的数组顺序是语义的一部分。

## 4. 指令与操作数

### 4.1 两条规则

- **R1**：源操作数顺序 = `isa.md` §4 表顺序。推论：分支目标在最后（`FLT f1,f2,L`）；存储/激发目标在最前（`FEXCITE m,f1`、`FVSET v,f1`）。
- **R2**：立即数字面量**直接表示 `imm` 字段本身**。

| `imm` 解释 | 写法 |
|---|---|
| `F32`（`FMOVI`/`WAIT`） | 浮点字面量 → round-to-nearest 转 f32 位模式；`0x…` → 直接就是位模式 |
| `I32`（`IMOVI`） | 十进制带符号；`0x`/`0b`/`0o` → u32 位模式，补码重解释 |
| `Index`（`FSENSE`/`FEXCITE`/`FVGET`/…） | **名字**，从对应表解析 |
| `JumpOffset` | **标签**，不接受数字 |

- 十进制浮点用 `f32::from_str` 直读（单次正确舍入），**不得**先转 f64 再转 f32（双重舍入）。
- 寄存器操作数按 `Operand::Reg(_, RegType)` 校验文件；arity 由 `signature()` 校验——`FADD f1,i2,f3` 汇编期报错。

### 4.2 `FEXCITE` 与 `group`

```
FEXCITE mouth_open, f1          # 单肌肉
FEXCITE corners,     f1          # group：1 源项 → N 指令，各拿同一个 f1
```

`group` 只在 `FEXCITE` 的位置被接受。

### 4.3 `FLUSH`

```
FLUSH                            # 无操作数；published ← pending，清 pending
```

- 见 `isa.md` §8.2 / §4.11。
- 汇编期约束（§6）：从最后一次 `FLUSH` 起，若有 `FEXCITE` 未 `FLUSH` 就遇到挂起点（`YIELD`/`WAIT`）或 `HALT`，是**硬错误**。

## 5. 标签与两遍

```
第一遍（收集 + 计数）
  pc = 0
  for item in source:
      if item is label: labels[name] = pc      # 标签指向其后的第一条指令
      else:             pc += emit_count(item)  # group 会 1→N
  code_len = pc

第二遍（发射）
  pc = 0
  for item in source:
      if item is label: continue
      if item is jump with target t:
          off = jump_offset(pc, labels[t])
          emit(op, off as i32)
      else:
          emit(...)
      pc += emit_count(item)
```

- 偏移由 `crates/cyberspine-core/src/vm/jump.rs::jump_offset` 唯一计算，汇编器不得重推 `pc + 1 + off`。
- 末尾非 `HALT` 时补一条隐式 `HALT`；末标签指向它（§5.3「跳到末尾」）。
- 校验：未定义标签、重复标签、`off` 装不进 i32；末标签在手写末尾 `HALT` 之后 = 越界错误。

## 6. 汇编期校验

除 `binfmt.md` §9 的加载期校验（汇编器尽量提前做），增加：

1. 未定义/重复标签。
2. 跳转目标越界。
3. 寄存器文件不匹配、arity 错误、立即数超范围。
4. `FEXCITE` 引用的名字不在 manifest 的 muscles；`FSENSE`/`ISENSE`/`FVGET`/`FVSET`/`IVGET`/`IVSET` 的名字不在对应表。
5. `.group` 的成员不在 manifest 的 muscles。
6. **pending 泄漏**：见 §4.3，硬错误。
7. `WAIT` 时长非负有限且 ≤ `max_wait`。
8. 名字、标签合法；`fN`/`iN`/`dt`/`tick` 保留名不得用作标签、变量名或组名。manifest 里的名字受同一组规则约束：`generate_image` 也校验它拿到的 `Rig`，因为代码里构造的 `Rig` 不经过 manifest 解析，而镜像仍必须是加载期合法的。

pending 检查是一个布尔数据流：`FEXCITE` 置真，`FLUSH` 清假，在挂起点/HALT 处若为真报错；分支处取并集。

### 6.1 诊断

- 解析阶段**不 panic**：任何非法输入都以 `AsmParseErrors` 返回。
- 每个错误带 1-based `Location { line, column }` 与稳定的 `ParseErrorKind`，
  便于工具按类别处理，而不是解析错误字符串。
- `header()` / `text()` 在行边界恢复：一行出错后继续检查后续行，因此一次汇编可以
  报告多条独立错误；`parse()` 还会把头段与指令段的错误合并后按行列排序。
- 固定头段：重复字段在**重复行**报告；缺失字段在**声明段结束处**（`.text` 行、
  第一条指令行或 EOF）报告，且四个缺失字段全部报告。
- `text()` 的行号从它自己的输入算起；`parse()` 会把指令段错误平移回文件坐标。
- 渲染使用 [`annotate-snippets`](https://crates.io/crates/annotate-snippets)：
  ```
  error: unknown mnemonic `FADDD`
    ╭▸ bad.csa:7:5
    │
  7 │     FADDD f1, f2, f3
    ╰╴    ━
  ```
- CLI 可执行文件统一命名为 `cyine` + 工具字母：汇编器是 **`cyinea`**；VM 侧按同一
  规则使用 `cyinev`。asm 与 vm 是独立进程，不合并成单个二进制。
- CLI 输出 `path:line:col: error: …` + 源行 + 标记，并以非零退出码结束。

```bash
cyinea -a greet.csa              # 输出 greet.csp
cyinea -a greet.csa -o out.csp   # 指定输出路径
```

### 6.2 输出（落盘）

写盘不属于格式（`binfmt.md`）也不属于 VM：VM 只读，写是生产者的动作。实现见
`crates/cyberspine-asm/src/store.rs`，契约如下。

- **原子替换**：字节先写进目标**同目录**的临时文件，再 `rename` 覆盖目标。
  崩溃或磁盘满时旧文件保持原样；不会留下半截 `.csp`——半截文件在加载期只会报
  C6 哈希不符，极难定位。同目录同时保证 `rename` 不跨文件系统（否则 `EXDEV`）。
- **跟随符号链接**：写到链接目标，链接本身保留（与 `fs::write`、Cargo 一致）。
- **权限**：Unix 上目标已存在时继承其权限位；新文件按 umask（通常 `0644`）。
  Windows 上继承只读属性。否则临时文件写入器会把产物悄悄变成 `0600`。
- **不 `fsync`**：原子 ≠ 掉电持久。`rename` 保证读者看到旧的或新的，不保证
  两者之一在断电后仍在。
- **只读目标仍会被替换**：Unix 只看目录权限，Windows 走 `SetFileInformationByHandle`
  回退；这与 `fs::write` 的行为不同，是有意为之。
- **Windows 上被占用的目标**：短暂退避重试（杀毒/索引器），仍失败则报错并提示
  文件可能被其他进程打开或映射。
- **写前自检**：编码结果必须先通过加载器（`binfmt.md` §9）才会落盘；不通过按
  汇编器内部错误报告，不产生文件。
- `-o` 指向输入源文件时拒绝执行（否则 `.csa` 会被二进制覆盖）。
- 退出码：`0` 成功；`1` 汇编或写盘错误；`2` 命令行用法错误（clap 的约定）。

## 7. 完整例子

```
# greet.csa — 打招呼：抬眉 → 微笑 → 保持 → 复位
.rig            "rigs/live2d-standard.rig"

.layer          10
.clock          hz = 60
.fuel           4096
.max_wait       600.0

.group          brows   = brow_l, brow_r
.group          corners = mouth_corner_l, mouth_corner_r

.text
entry:
    ISENSE  i1, gaze_lock
    IJNZ    i1, L_done

    FMOVI   f1, 1.0
    FMOVI   f2, 0.0
    FMOVI   f3, 0.85
    FMOVI   f4, 0.90

    FEXCITE brows, f1
    FLUSH                           # 抬眉这一批提交
wait_brow:
    FSENSE  f5, brow_raise
    FGE     f5, f4, L_brow_ok
    YIELD
    JMP     wait_brow
L_brow_ok:

    FEXCITE corners, f1
    FLUSH                           # 微笑这一批提交
wait_smile:
    FSENSE  f5, smile
    FGE     f5, f3, L_smile_ok
    YIELD
    JMP     wait_smile
L_smile_ok:

    WAIT    2.0

    FEXCITE brows, f2
    FEXCITE corners, f2
    FLUSH                           # 复位这一批提交
    WAIT    0.5
L_done:
    HALT
```

（`dt` / `tick` 未显式声明，自动存在。）

## 8. 调试信息

- 标签表与 `指令下标 → 源行号` 的 line_map 放独立 `.cspd`，不进入 `.csp`（`isa.md` §5.5）。
- 反汇编用标签表显示 `JMP L_brow_ok` 而非 `JMP +37`。
- `.cspd` 字节格式**尚未规范**，属于独立任务。

## 9. 与 opcode 表的绑定

助记符、操作数、arity、立即数类型全部由 `crates/cyberspine-core/src/isa.rs` 的 `isa_table!` 派生。本文件描述的是**词法**（怎么写寄存器/数字/名字/标签）；指令语义见 `isa.md` §4。
