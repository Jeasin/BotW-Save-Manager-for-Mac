# botw-save-convert

一个面向 macOS 的精简命令行工具，用于在 **《塞尔达传说：旷野之息》Wii U 与 Switch 存档之间互转**。

这是 [DeltaJordan/BotW-Save-Manager](https://github.com/DeltaJordan/BotW-Save-Manager) 的 Rust 重实现，只保留命令行转换功能，不依赖 .NET、GUI 或任何第三方 Rust crate。

> **验证状态**：两个方向都已用真实存档验证。
> Wii U → Switch：Cemu 存档转换后，0–3 号槽位与同一进度的 Ryujinx 存档逐字节一致。
> Switch → Wii U：一份 1.9.0 存档（8 个槽位，v1.5 / v1.6 / v1.8-v1.9 混排）转换后，8 个槽位与 Cemu 原生 Wii U 存档的**记录序列（128399 条）逐条对齐**，`option.sav` 为 336 字节 / 40 条记录，全部槽位头部为 `00 00 47 1b`。
> 尚未在真机上完成导入测试，首次使用请仍然保留原始备份。

## 特性

- Wii U ↔ Switch 自动识别与转换，转换前打印逐槽位报告
- 不给输出目录时，自动在输入目录旁生成 `<原名>-switch` / `<原名>-wiiu`
- 输出目录的缺失父目录会自动创建
- 单个原生可执行文件，无运行时依赖
- 输入目录只读，绝不原地修改
- 输出先写入临时目录，校验通过后再整体改名
- 拒绝覆盖已存在的输出目录
- 已是目标平台字节序的文件原样复制，不会二次翻转
- 支持 `--dry-run` 预检
- 未知游戏版本默认拒绝转换，可用显式开关放行
- 区分 v1.0 – v1.9，说明每个版本的平台限制
- **Switch → Wii U 时会自动降级**：把 v1.6 / v1.8-v1.9 存档里 Wii U 不认识的字段丢掉、头部改回 v1.5，使它变成 Wii U 能读的记录集
- 逐槽位列出读档界面的信息：写入时刻、游玩时长、地区，以及这一档是普通/大师模式、手动还是自动（读自 `caption.sav`）

## 构建

要求 Rust 1.85 或更高版本：

```bash
cargo build --release
```

生成的可执行文件位于：

```text
target/release/botw-convert
```

安装到用户目录：

```bash
install -m 755 target/release/botw-convert ~/.local/bin/botw-convert
```

## 使用

```bash
botw-convert [OPTIONS] <输入目录> [输出目录]
```

输入目录必须直接包含 `option.sav`，可以包含多层 `game_data.sav` 和 `trackblock*.sav` 存档文件。

不写输出目录时，工具会在输入目录的**同级**生成带平台后缀的新目录：

```text
.../80000001/BotW/          →  .../80000001/BotW-switch
.../0/save-backup/          →  .../0/save-backup-wiiu
```

输入名本身已带 `-switch` / `-wiiu` 时会替换后缀，不会叠加。

先预检：

```bash
botw-convert --dry-run ~/Downloads/zelda-save
```

确认后转换：

```bash
botw-convert ~/Downloads/zelda-save
# 也可以显式指定输出
botw-convert ~/Downloads/zelda-save ~/Downloads/out/zelda-switch
```

输出目录必须尚不存在；缺失的父目录会自动创建。一次 Switch → Wii U 的实际报告：

```text
Source   : /Users/me/Downloads/202610311719-backup-astris-002
Save type: Switch (NS) save
option   : v1.8/v1.9 (920 bytes)
Versions : v1.5 x4, v1.6 x2, v1.8/v1.9 x2
Target   : Wii U
Output   : /Users/me/Downloads/202610311719-backup-astris-002-wiiu
Files    : 69 .sav (8 slots, 52 trackers, 8 captions, 1 option)

Slot  Stored  Version      Bytes    Saved                Play  Kind           Place             Action
0     Switch  v1.5         1027208  2026-06-07 13:58   141:05  normal auto    Central Hyrule    convert to Wii U
1     Switch  v1.5         1027208  2026-06-07 14:04   117:28  normal auto    Eldin Canyon      convert to Wii U
2     Switch  v1.5         1027208  2026-06-07 14:06   156:54  normal auto    Eldin Canyon      convert to Wii U
3     Switch  v1.5         1027208  2026-05-16 21:04    88:38  normal manual  West Hateru       convert to Wii U
4     Wii U   v1.6         1027216  2025-02-04 19:05   103:07  normal auto    Central Hyrule    keep + downgrade
5     Wii U   v1.6         1027216  2025-02-04 19:07    82:23  normal auto    Hebura Mountains  keep + downgrade
6     Switch  v1.8/v1.9    1027248  2026-10-22 14:23   157:23  master auto    East Hateru       convert + downgrade
7     Switch  v1.8/v1.9    1027248  2026-10-21 16:55   141:13  master manual  East Hateru       convert + downgrade
warning: 4 slot(s) use a save format newer than the Wii U release (4 v1.6, 5 v1.6,
         6 v1.8/v1.9, 7 v1.8/v1.9); their extra fields are dropped and the header
         is rewritten to v1.5
warning: option.sav is v1.8/v1.9; its newer fields are dropped to restore the
         336-byte v1.5 layout the Wii U release writes

Done. Converted 65 .sav to Wii U format.
      Kept 4 .sav that were already Wii U.
      Downgraded 9 file(s) to the v1.5 layout (85 newer field(s) dropped).
      210 files copied in total.
      Output: /Users/me/Downloads/202610311719-backup-astris-002-wiiu
```

各列含义：

| 输出 | 含义 |
|---|---|
| `Files` | 目录里 `.sav` 总数，以及按用途的分类（槽位、tracker、caption、option、重复副本） |
| `Slot` | 槽位目录名，即 `game_data.sav` 所在的文件夹 |
| `Stored` | 该文件**自身头部**显示的平台，可能和整个存档不一致 |
| `Version` | 从文件头读出的游戏版本 |
| `Saved` | 该档写入的时刻，读自该槽的 `caption.sav` |
| `Play` | 该档累计游玩时长（`小时:分钟`），同样来自 `caption.sav` |
| `Kind` | `normal` / `master` 加 `manual` / `auto`，见下节 |
| `Place` | 存档时的地区名（`MapArea_*` 去掉前缀、驼峰断开） |
| `Action` | `convert` 翻转字节序、`keep` 原样复制、`drop` 不写入输出、`BLOCKED` 无法转换；`+ downgrade` 表示同时降级 |

`--dry-run` 不会因为输出目录已存在而失败，只给出 warning，方便反复预检。

Wii U → Switch 和 Switch → Wii U 使用同一个命令，工具会根据 `option.sav` 自动判断方向。

### 槽位、自动保存与手动保存

一份旷野之息存档固定有 8 个槽位目录 `0`–`7`，它们不是 8 个平级的自由存档位：

- **普通模式 6 个**：1 个手动存档 + 5 个轮换的自动存档
- **大师模式 2 个**：1 个手动 + 1 个自动

目录名是固定的，游戏只按索引写。**「这一档是手动还是自动」不写在 `game_data.sav` 里**，而是写在每个槽位旁边的 `caption.sav` —— 也就是读档界面上显示的那些信息：

| 字段哈希 | 含义 |
|---|---|
| `0x0ba7cf40` | 写入时刻（Unix 时间戳） |
| `0xa0678286` | 该档游玩时长（分钟） |
| `0xe51256e2` | `1` = 大师模式 |
| `0x4acb5004` | `0` = 手动存档，`1` = 自动存档 |
| `0x46d266b6` | 地点名（每 4 字节一条记录，哈希重复） |
| `0xf26f1a02` | `MapArea_*` 地区 id |

这几个哈希是在真实存档上对出来的：时间与时长和读档界面一致，两个标志位把 8 个槽切成「普通 6 / 大师 2」，且每个模式正好一个手动档，与官方结构吻合。`caption.sav` 缺失或损坏时，这几列显示 `-`，不影响转换。

`caption.sav` 的字符串块在 Wii U 与 Switch 上**逐字节相同**（不参与字节序翻转），只有数值字段翻转；降级时它的记录数完全不变（v1.5 与 v1.8/v1.9 都是 189 条），只换版本号，这部分是无损的。

`Saved` 打印的是**游戏自己时钟的读数**（按 UTC 渲染）。模拟器把这个时钟设成本机时间，所以在模拟器存档上看到的就是你保存时的钟点；真机存档可能相差一个时区。

### 选项

```text
-n, --dry-run              只检查，不写入任何文件
    --allow-unsupported    允许未知游戏版本继续转换
    --drop-incompatible    Switch → Wii U 时，跳过没有字段表、无法降级的槽位
    --option-sav <FILE>    Switch → Wii U 时，用指定的 Wii U option.sav 覆盖输出
-v, --verbose              列出每个 .sav 的处理方式
-h, --help                 显示帮助
-V, --version              显示版本
```

退出码：`0` 成功，`1` 失败，`2` 参数错误，`3` 存档不受支持（版本号无法识别，且没有加 `--allow-unsupported`）。

## 版本对照

| 版本 | 头部 | Wii U | Switch | 说明 |
|---|---|---|---|---|
| v1.5 | `0x471b` | ✅ 最终版本 | ✅ | 双向转换最稳妥 |
| v1.6 | `0x471e` | ⚠️ 无此更新 | ✅ | 仅 Switch 有（Labo VR），转 Wii U 时丢 1 个字段 |
| v1.8 / v1.9 | `0x4730` | ⚠️ 无此更新 | ✅ | 转 Wii U 时丢 5 个字段（`game_data.sav`） |

Wii U 版的最后一次更新是 2018-01 的 1.5.0，任天堂官方也写明 Wii U 版没有 1.6.0 及之后的更新。1.6.0（Labo VR）与 1.8/1.9（随 Switch 2 一起推送的泰语支持与修 bug，Switch 同样收到）都只面向 Switch 系。

`0x4730` 这个头部同时覆盖 1.8.x 与 1.9.x，社区编辑器（marcrobledo 的 BOTW 编辑器）也是把它们合并成一条 `v1.8/v1.9`，存档本身无法区分。

### 新版本存档如何转成 Wii U（降级）

`option.sav`、`game_data.sav`、`caption.sav` 用的是**同一个容器**：

```text
0x00  u32  版本号
0x04  u32  不变
0x08  u32  不变
0x0c  ...  { u32 字段哈希; u32 值 } 重复
末尾  u32  0xffffffff 终止符
```

它是**按哈希索引的键值表**，不是固定偏移，所以“新版本”只是在同一张表里多了若干条记录。降级的做法因此很直接：

1. 丢掉 Wii U 版不认识的字段（`game_data.sav` 5 个、`option.sav` 10 个哈希）；
2. 把头部版本号改成 `0x471b`；
3. 其余记录原封不动保留（进度、物品、地图数据都在这些记录里）。

以实测的 1.9.0 存档为例：

| 文件 | 原始 | 降级后 | 丢掉 |
|---|---|---|---|
| `game_data.sav` | 1027248 B / 128404 条 | 1027208 B / 128399 条 | 5 条（40 B） |
| `option.sav` | 920 B / 113 条 | 336 B / 40 条 | 73 条（10 个哈希，含数组/字符串块） |
| `caption.sav` | 1528 B | 1528 B | 无，只改版本号 |

降级后的 8 个槽位与 Cemu 原生 Wii U 存档的记录哈希序列**逐条一致**（128399 条全等），说明容器结构完全对齐。

被丢掉的字段是游戏设置类的新条目（例如 1.8/1.9 新增的选项），不是进度数据；代价是这些新设置项在 Wii U 上不存在，回到 Switch 时也不会被还原。

`--option-sav` 仍然保留，作为“我手上就有一份 Wii U `option.sav`，直接用它”的覆盖手段；不指定时工具会自己降级源文件。

### 为什么有些槽位是“混排”的

判据只有一条，但足够硬：**平台字节序是游戏自己写死的**。Switch 版永远写小端（头部 `xx 47 00 00`），Wii U 版永远写大端（头部 `00 00 47 xx`）。一个 Switch 存档里出现大端文件（或反过来），只可能是转换器或手工拷贝放进去的，游戏本身不可能写出对方平台的字节序。

实测的两份存档就是这种情况：

| 槽位 | Ryujinx（Switch 存档） | Cemu（Wii U 存档） | 结论 |
|---|---|---|---|
| 0–3 | `1b 47 00 00` 小端 v1.5 | `00 00 47 1b` 大端 v1.5 | 正常 |
| 4–5 | `00 00 47 1e` **大端** v1.6 | `1e 47 00 00` **小端** v1.6 | 两边都放了对方的格式 |
| 6–7 | `30 47 00 00` 小端 v1.8/v1.9 | `00 00 47 1b` 大端 v1.5 | 正常 |

再校验一层：把 Ryujinx 的 4/5 用同一算法翻回小端，和 Cemu 的 4/5 对比，**sha256 完全相同**（`68d13cbc…`、`56eb3401…`）——同一份数据的两种字节序。同样，Ryujinx 的 0–3 与“Cemu 转 Switch”的输出逐字节相同，说明整份 Ryujinx 存档就是从 Cemu 存档转过去的，而 4/5 是被某次转换留在了对方字节序里。

因此处理方式是：**已经是目标平台字节序的文件只复制、不翻转**，需要时再降级；否则二次翻转会直接把存档写坏。

## 安全设计

- 输入目录不会被修改。
- 输出目录必须不存在，避免误覆盖已有文件；缺失的父目录会自动创建。
- 转换在输出目录同级的隐藏临时目录中进行，全部成功并通过结构校验后才重命名为目标目录；失败时会清理临时目录。
- macOS 有时会拒绝“重命名文件夹”却允许写入（`~/Downloads`、`~/Desktop` 有隐私弹窗，沙箱环境直接返回 EPERM）。这时工具改为把已完成的临时目录**复制**到目标位置，再删掉临时目录，结果一致。
- 输出目录不能位于输入目录内部。
- 不跟随符号链接，遇到符号链接会直接拒绝。
- 转换后会重新读取输出，逐槽位确认平台字节序和游戏版本号没有改变。
- 每个 `.sav` 会单独读取头部：已经是目标平台字节序的文件原样复制，避免重复翻转把存档转坏。
- 云同步/文件管理器产生的重复副本（如 `game_data-NSConflict-5.sav`、`game_data-冲突-Jeasin_Win11.sav`）会被识别并提示，仍按普通 `.sav` 处理。

## 首次真实存档验证

1. 复制一份完整存档目录，不要直接对唯一存档操作。
2. 先执行 `--dry-run`，确认逐槽位表格里的 `Stored`、`Version`、`Action` 符合预期。
3. 转换到新目录，检查目标目录文件层级与原目录一致。
4. 将转换结果导入游戏，至少测试启动、读取存档和保存一次。
5. 再转换回原平台，用 `shasum` 比较往返后的文件是否一致。

如果真实设备上读取失败，请保留源存档、转换结果和终端输出，不要继续反复覆盖测试。

## 限制

- 只处理已经导出/解密、包含 `option.sav` 的存档文件夹；不直接读取主机内部存储或加密容器。
- 降级只丢弃“Wii U 版本不认识的设置类字段”，不改写游戏进度；但被丢弃的新设置项无法在回到 Switch 后自动还原。
- 版本表覆盖 v1.0、v1.1、v1.2、v1.3、v1.3.3、v1.4、v1.5、v1.6、v1.8/v1.9（`0x4730`）。更晚的版本号（任意大于 `0x471b` 的头）默认被拒绝：加 `--allow-unsupported` 会按已知字段表降级，加 `--drop-incompatible` 则把这些槽位排除在输出之外。
- 被改造过的存档（社区编辑器里标成 `v1.6*`、魔数为 `0x0f423d`/`0x0f423e`/`0x0f423f` 那类）头部超出 `0x47xx` 形状，会被当成无法识别，需要用 `--allow-unsupported` 放行。
- 存档目录中可以混有“已经是另一平台格式”的槽位（例如之前转换过或手工合并过）。这些文件不会被再次翻转，工具会以 warning 列出。
- 首次在真实设备上使用前，请保留一份未改动的原始存档。

## 开发与测试

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

测试包含转换常量检查、平台/版本识别、哈希与物品分支、trackblock 特殊处理、记录表解析与降级、逐槽位动作判定、完整目录转换、`--option-sav` 覆盖、输出保护以及 CLI 退出码。

## 许可证与署名

本项目按 **GNU Affero General Public License v3.0 only** 发布，见 [LICENSE](LICENSE) 和 [NOTICE](NOTICE)。

转换算法和常量移植自 AGPL-3.0 的 BotW Save Manager；它不是官方工具，与 Nintendo 无关。
