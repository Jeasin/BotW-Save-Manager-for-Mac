# botw-save-convert

一个面向 macOS 的精简命令行工具，用于在 **《塞尔达传说：旷野之息》Wii U 与 Switch 存档之间互转**。

这是 [DeltaJordan/BotW-Save-Manager](https://github.com/DeltaJordan/BotW-Save-Manager) 的 Rust 重实现，只保留命令行转换功能，不依赖 .NET、GUI 或任何第三方 Rust crate。

> **早期版本提醒**：目前已经通过算法的合成数据测试和完整 CLI 测试，但还没有用真实存档及游戏主机验证。首次使用时请务必使用存档副本，并保留原始备份。

## 特性

- Wii U ↔ Switch 自动识别与转换
- 单个原生可执行文件，无运行时依赖
- 输入目录只读，绝不原地修改
- 输出先写入临时目录，校验通过后再整体改名
- 拒绝覆盖已存在的输出目录
- 支持 `--dry-run` 预检
- 未知或混合游戏版本默认拒绝转换，可用显式开关放行

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
botw-convert [OPTIONS] <输入目录> <输出目录>
```

输入目录必须直接包含 `option.sav`，可以包含多层 `game_data.sav` 和 `trackblock*.sav` 存档文件。输出目录必须尚不存在，其父目录必须已存在。

先预检：

```bash
botw-convert --dry-run ~/Desktop/zelda-save ~/Desktop/zelda-save-switch
```

确认后转换：

```bash
botw-convert ~/Desktop/zelda-save ~/Desktop/zelda-save-switch
```

Wii U → Switch 和 Switch → Wii U 使用同一个命令，工具会根据 `option.sav` 自动判断方向。

### 选项

```text
-n, --dry-run              只检查，不写入任何文件
    --allow-unsupported    允许未知或混合游戏版本继续转换
-h, --help                 显示帮助
-V, --version              显示版本
```

## 安全设计

- 输入目录不会被修改。
- 输出目录必须不存在，避免误覆盖已有文件。
- 转换在输出目录同级的隐藏临时目录中进行，全部成功并通过结构校验后才重命名为目标目录；失败时会清理临时目录。
- 输出目录不能位于输入目录内部。
- 不跟随符号链接，遇到符号链接会直接拒绝。
- 转换后会重新读取输出，确认平台标志及已知游戏版本没有改变。

## 首次真实存档验证

1. 复制一份完整存档目录，不要直接对唯一存档操作。
2. 先执行 `--dry-run`，确认识别的来源平台、版本和 `.sav` 数量正确。
3. 转换到新目录，检查目标目录文件层级与原目录一致。
4. 将转换结果导入游戏，至少测试启动、读取存档和保存一次。
5. 再转换回原平台，用 `shasum` 比较往返后的文件是否一致。

如果真实设备上读取失败，请保留源存档、转换结果和终端输出，不要继续反复覆盖测试。

## 限制

- 只处理已经导出/解密、包含 `option.sav` 的存档文件夹；不直接读取主机内部存储或加密容器。
- 不做游戏版本升级或降级。源存档各槽位应属于同一游戏版本。
- 支持原工具列出的版本：v1.0、v1.1、v1.2、v1.3、v1.3.3、v1.4、v1.5、v1.6。
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
