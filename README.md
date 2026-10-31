# botw-save-convert

一个面向 macOS 的精简命令行工具，用于在 **《塞尔达传说：旷野之息》Wii U 与 Switch 存档之间互转**。

这是 [DeltaJordan/BotW-Save-Manager](https://github.com/DeltaJordan/BotW-Save-Manager) 的 Rust 重实现，只保留命令行转换功能，不依赖 .NET、GUI 或任何第三方 Rust crate。

> **验证状态**：算法已用真实存档验证。Cemu 的 Wii U 存档转换后，0–3 号槽位与同一进度的 Ryujinx Switch 存档逐字节一致。尚未在真机上完成导入测试，首次使用请仍然保留原始备份。

## 特性

- Wii U ↔ Switch 自动识别与转换，转换前打印识别结果
- 不给输出目录时，自动在输入目录旁生成 `<原名>-switch` / `<原名>-wiiu`
- 输出目录的缺失父目录会自动创建
- 单个原生可执行文件，无运行时依赖
- 输入目录只读，绝不原地修改
- 输出先写入临时目录，校验通过后再整体改名
- 拒绝覆盖已存在的输出目录
- 已是目标平台字节序的文件原样复制，不会二次翻转
- 支持 `--dry-run` 预检
- 未知游戏版本默认拒绝转换，可用显式开关放行
- 识别 Switch 2 Edition（v1.7）并给出明确说明，而不是报“未知文件”

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
botw-convert --dry-run ~/Desktop/zelda-save
```

确认后转换：

```bash
botw-convert ~/Desktop/zelda-save
# 也可以显式指定输出
botw-convert ~/Desktop/zelda-save ~/Desktop/out/zelda-switch
```

输出目录必须尚不存在；缺失的父目录会自动创建。转换时会打印识别结果：

```text
Detected : Wii U save (v1.5)
Target   : Switch (NS)
Files    : 86 .sav
Output   : /path/to/zelda-save-switch
```

Wii U → Switch 和 Switch → Wii U 使用同一个命令，工具会根据 `option.sav` 自动判断方向。

### 选项

```text
-n, --dry-run              只检查，不写入任何文件
    --allow-unsupported    允许未知游戏版本继续转换
-h, --help                 显示帮助
-V, --version              显示版本
```

## 安全设计

- 输入目录不会被修改。
- 输出目录必须不存在，避免误覆盖已有文件；缺失的父目录会自动创建。
- 转换在输出目录同级的隐藏临时目录中进行，全部成功并通过结构校验后才重命名为目标目录；失败时会清理临时目录。
- 输出目录不能位于输入目录内部。
- 不跟随符号链接，遇到符号链接会直接拒绝。
- 转换后会重新读取输出，确认平台标志及已知游戏版本没有改变。
- 每个 `.sav` 会单独读取头部：已经是目标平台字节序的文件原样复制，避免重复翻转把存档转坏。

## 首次真实存档验证

1. 复制一份完整存档目录，不要直接对唯一存档操作。
2. 先执行 `--dry-run`，确认识别的来源平台、版本和 `.sav` 数量正确。
3. 转换到新目录，检查目标目录文件层级与原目录一致。
4. 将转换结果导入游戏，至少测试启动、读取存档和保存一次。
5. 再转换回原平台，用 `shasum` 比较往返后的文件是否一致。

如果真实设备上读取失败，请保留源存档、转换结果和终端输出，不要继续反复覆盖测试。

## 限制

- 只处理已经导出/解密、包含 `option.sav` 的存档文件夹；不直接读取主机内部存储或加密容器。
- 不做游戏版本升级或降级。
- 支持原工具列出的版本：v1.0、v1.1、v1.2、v1.3、v1.3.3、v1.4、v1.5、v1.6，另外可识别 **v1.7 Switch 2 Edition（0x4730）**。
- **Switch 2 Edition 存档不能转成 Wii U**：Wii U 版游戏没有 v1.7 格式，工具会明确拒绝，而不是生成一个游戏读不了的文件。需要 Switch → Wii U 时请使用 v1.5/v1.6 的 Switch 存档。
- 存档目录中可以混有“已经是另一平台格式”的槽位（例如之前转换过或手工合并过）。这些文件不会被再次翻转，工具会以 warning 列出。
- 首次在真实设备上使用前，请保留一份未改动的原始存档。

## 开发与测试

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

测试包含转换常量检查、平台/版本识别、哈希与物品分支、trackblock 特殊处理、完整目录转换、输出保护以及 CLI 退出码。

## 许可证与署名

本项目按 **GNU Affero General Public License v3.0 only** 发布，见 [LICENSE](LICENSE) 和 [NOTICE](NOTICE)。

转换算法和常量移植自 AGPL-3.0 的 BotW Save Manager；它不是官方工具，与 Nintendo 无关。
