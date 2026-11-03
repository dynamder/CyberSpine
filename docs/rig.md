# cyber-spine rig 清单与指纹

状态：v1
依赖：[`isa.md`](isa.md) §13（rig、装配期绑定、指纹）、[`binfmt.md`](binfmt.md) §4（头段字段）、[`asm.md`](asm.md) §3.2（`.rig`）

> **实现状态（2026-11）**：全部落地。`cyberspine-rig`（类型 + 指纹）、`cyberspine-manifest`（JSON 解析）、汇编器按 manifest 生成表并把指纹写进头段、`cyinea --rig`、宿主的 `Backend::fingerprint()` + `Backend::shape()` 与加载期比对（未绑定镜像由宿主的 `Host::permissive()` 决定）。本文件描述的就是现状。

---

## 0. 一句话

**rig 是后端实现的那具身体的名字集合；manifest 是它的 JSON 描述；指纹是它的身份。**

汇编器用 manifest 生成镜像，后端用自己的声明生成同一份 `Rig`，两边独立算出同一个指纹。加载时比对，不等就拒绝运行。

## 1. 为什么需要指纹

manifest 是手写或工具生成的，后端代码是另一处真相。两者可能不一致：manifest 过期、抄错、拿错文件。

不引入指纹时，这种不一致会变成**运行期的一次错误动作**——语义 UB，比内存 UB 后果更严重，因为 CyberSpine 很可能在驱动真实设备。

引入指纹后，不一致在**加载期**暴露。manifest 因此不承担"真相来源"的角色，只是装配时的一份方便输入。

## 2. manifest 格式

```json
{
  "rig": { "name": "live2d-standard", "version": 1 },
  "muscles": ["mouth_open", "brow_l"],
  "sensors_f": ["blink_speed"],
  "sensors_i": ["gaze_lock"]
}
```

文件名按惯例是 `<name>.rig.json`：`.rig` 是汇编指令的名字，`.json` 后缀只是给编辑器看的（JSON 高亮因此生效）；汇编器把路径原样使用，不检查后缀。

| 字段 | 类型 | 说明 |
|---|---|---|
| `rig.name` | string | 身份名，非空 |
| `rig.version` | u32 | 语义版本。**肌肉含义改变时必须 +1**，即使名字没变 |
| `muscles` | [string] | 肌肉名，数组顺序 = 物理下标 |
| `sensors_f` | [string] | 后端 f32 可观测量，顺序 = 下标 |
| `sensors_i` | [string] | 后端 i32 可观测量，顺序 = 下标 |

规则：

- **数组顺序是语义**。换了顺序就是另一份 rig，指纹不同。
- 名字必须匹配 `[A-Za-z_][A-Za-z0-9_]*`。
- 不得使用 `dt` / `tick`（语言内置），也不得使用 `f0`..`f63` / `i0`..`i63` 的形状（寄存器拼写）。
- 肌肉名与两个 sensor 表**共享一个命名空间**：肌肉的读回 sensor 用肌肉名（`binfmt.md` §6.2），同名即冲突。
- 未知字段一律报错（`deny_unknown_fields`），这样拼错的键不会变成一个静默的空表。

## 3. 指纹

```
fingerprint = blake3(canonical(rig))[0..32]
```

规范化编码（全部小端、字段长度前缀，保证任意两份不同的 rig 不会拼出同一串字节）：

```
"cyberspine-rig\0"
le32(len) || name
le32(version)
le32(count) || (le32(len) || muscle_name)*
le32(count) || (le32(len) || sensor_f_name)*
le32(count) || (le32(len) || sensor_i_name)*
```

- `dt` / `tick` **不在**编码里：它们是语言内置，双方都按同一规则派生，不属于某份 rig。
- 编码来自**解析后的结构**：JSON 里的空白、键序、注释（JSON 没有注释）都不影响。

## 4. 谁算、何时比

```
汇编器：manifest 文件 → Rig → fingerprint → 写进镜像头段（binfmt.md §4）
后端：  代码里的声明 → Rig → fingerprint → 加载时报出
        （跨边界的后端报出 manifest 文本，宿主自己解析并算指纹，见 backend.md §2）
加载：  两者相等才运行，否则拒绝（isa.md §13.3）
```

- 后端实现 `Backend::fingerprint()`，结果应在构造时算好并缓存。
- **跨进程或动态库的后端不传指纹，传 manifest。** 宿主解析、校验、自己算指纹：一个从设备读回来的
  32 字节永远不能变成 `BoundFingerprint`，因为该类型没有这样的构造路径。日志里也因此是可读的名字与
  版本，而不是一串十六进制。
- **`BoundFingerprint` 不会是全零。** 镜像侧的 `Fingerprint` 可以是全零（未绑定），后端的不能：
  该类型字段私有、只有一个构造器 `of(&Rig)`，并且刻意没有 `From<Fingerprint>`/`Default`/`UNBOUND`
  常量——所以"从设备读回 32 字节然后贴上这个类型"这件事写不出来。全零摘要的概率是 2⁻²⁵⁶，
  构造处有一条 `debug_assert` 记着这件事。
- 未绑定镜像（头段指纹全 0，即装配时没有 rig）**默认拒绝**；放行是**宿主类型的属性**，
  不是后端字段：`Host<B, Strict>` 拒，`Host<B, Permissive>` 收（`Host::new(b).permissive()`，
  单向，没有反向方法）。只有测试、固件这类不经过宿主的路径需要它。

## 5. 信任模型

后端是可信原生代码（`isa.md` §9.5）。指纹**防手滑，不防恶意**：能谎报指纹的后端，代码已经跑在设备上了。

它的价值在于把"manifest 与后端代码是否一致"从人工保证降级成机器检查。

## 6. 与镜像的关系

- 镜像的 `muscles` / `sensors_f` / `sensors_i` 表**由 manifest 生成**（`asm.md` §3.3），因此 `muscle_count` 等于 rig 的肌肉数，实例按 rig 分配。
- `imm` 直接是物理下标，加载期没有重写阶段。
- 镜像因此**只为这一份 rig 生成**：不存在"跨 rig 复用"，也不存在 VOID 降级。

唯一安全的复用形态是「同一 manifest 的不同实现」——它们的指纹天然相同。
