# cyber-spine 汇编源格式（`.csa`）

状态：v1（随 isa v2 一起冻结）
依赖：[`isa.md`](isa.md)（指令编码与语义）、[`binfmt.md`](binfmt.md)（可执行镜像 `.csp`）
范围：汇编源码的词法、语法、伪指令、标签解析、汇编期校验与调试信息

---

## 0. 定位

- 输入：UTF-8 文本 `.csa`；输出：`Image`（`src/binfmt/image.rs`）→ `.csp` 字节，外加 `DebugInfo { labels, line_map }`。
- 汇编器**不接触 rig**。名字只解析成**头段表下标**（逻辑下标）；逻辑下标 → rig 物理下标是加载期 bind 的事（缺失绑 VOID，`isa.md` §13.3）。
- 指令语法由 `OpCode::signature()` 派生；助记符表由 `OpCode::ALL` + `mnemonic()` 反查生成。**加一条指令只改 `src/vm/op.rs`，不手抄任何清单。**
- 汇编期完成：标签→偏移、名字→表下标、`group` 展开、隐式 `HALT` 补齐、所有静态可判定的校验。

## 1. 词法

| 元素 | 形式 |
|---|---|
| 注释 | `#` 到行尾 |
| 标识符 | `[A-Za-z_][A-Za-z0-9_]*`，**大小写敏感** |
| 寄存器 | `f0`..`f63` / `i0`..`i63`（`f0`/`i0` 硬连零，写丢弃） |
| 十进制整数 | `42` `-42`（带符号，范围 i32） |
| 非十进整数 | `0x1F` `0b1010` `0o17`：**按 u32 位模式**，重解释为 i32（`0xFFFFFFFF` = `-1`） |
| 浮点字面量 | `1.5` `-0.25` `1e-3` `inf` `-inf` `nan` |
| 标签 | `ident` 后跟 `:` |
| 伪指令 | `.` 开头 |

- 数字允许 `_` 分隔（`1_000_000`）。
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
- 全部可省，取缺省值。

### 3.2 名字表

```
.muscle  brow_l                     # 肌肉：只是一个名字
.muscle  brow_r
.group   brows = brow_l, brow_r     # 汇编期展开；只在 FEXCITE 位置可用
.sensor_f blink_speed               # 后端可观测量，f32
.sensor_i gaze_lock                 # 后端可观测量，i32
.var_f   phase = 0.0                # 实例变量，f32
.var_i   state = 0                  # 实例变量，i32
```

- `.muscle X` **自动**生成一条同名读回 sensor（`binfmt.md` §6.2 的读回不变量靠构造成立）；显式 `.sensor_f` 与肌肉同名 = 错误。
- `dt` / `tick` **自动声明**、始终存在，不需要写。
- `.group` v1 **不嵌套**；组名不得与肌肉名重名（会撞上读回 sensor 的唯一性）。

### 3.3 表序

镜像表序固定（`binfmt.md` §3/§6）：

```
muscles   = [.muscle 声明序]
sensors_f = [每个肌肉的读回项，按 .muscle 序] ++ [.sensor_f 声明序] ++ [dt]
sensors_i = [.sensor_i 声明序] ++ [tick]
vars_f / vars_i = 声明序
```

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

- 见 `isa.md` §3.2 / §4.11。
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

- 偏移由 `src/vm/jump.rs::jump_offset` 唯一计算，汇编器不得重推 `pc + 1 + off`。
- 末尾非 `HALT` 时补一条隐式 `HALT`；末标签指向它（§5.3「跳到末尾」）。
- 校验：未定义标签、重复标签、`off` 装不进 i32；末标签在手写末尾 `HALT` 之后 = 越界错误。

## 6. 汇编期校验

除 `binfmt.md` §9 的加载期校验（汇编器尽量提前做），增加：

1. 未定义/重复标签。
2. 跳转目标越界。
3. 寄存器文件不匹配、arity 错误、立即数超范围。
4. `FEXCITE` 引用的名字不在 muscles 表；`FSENSE`/`ISENSE`/`FVGET`/`FVSET`/`IVGET`/`IVSET` 的名字不在对应表。
5. `.sensor_f` 与肌肉同名。
6. **pending 泄漏**：见 §4.3，硬错误。
7. `WAIT` 时长非负有限且 ≤ `max_wait`。
8. 名字、标签合法；`fN`/`iN`/`dt`/`tick` 保留名不得用作标签/名字。

pending 检查是一个布尔数据流：`FEXCITE` 置真，`FLUSH` 清假，在挂起点/HALT 处若为真报错；分支处取并集。

## 7. 完整例子

```
# greet.csa — 打招呼：抬眉 → 微笑 → 保持 → 复位
.layer          10
.clock          hz = 60
.fuel           4096
.max_wait       600.0

.muscle         brow_l
.muscle         brow_r
.muscle         mouth_corner_l
.muscle         mouth_corner_r
.group          brows   = brow_l, brow_r
.group          corners = mouth_corner_l, mouth_corner_r

.sensor_f       smile
.sensor_f       brow_raise
.sensor_i       gaze_lock

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

助记符、操作数、arity、立即数类型全部由 `src/vm/op.rs` 的 `isa_table!` 派生。本文件描述的是**词法**（怎么写寄存器/数字/名字/标签）；指令语义见 `isa.md` §4。