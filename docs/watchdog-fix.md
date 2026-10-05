# 独立 Watchdog 修复说明

本补丁基于 Clash-Verge-Buty 1.6.6，不更改版本号，也未向 GitHub 提交或推送。

## 行为

- `clash-verge-buty.exe` 仅承载 GUI，蓝色完整猫图标，任务管理器显示名 `Clash-Verge-Buty`。
- `clash-verge-buty-watchdog.exe` 为独立监督程序，黄色完整猫图标，文件名全小写与 GUI 一致；任务管理器显示名 `Clash-Verge-Buty-Watchdog`。黄色取自仓库自带系统代理托盘图标 `tray-icon-sys.png` 的原始色板（主体 `#EE8A68`），仅换色不改变完整猫形。
- 正常退出：GUI 清理系统代理与核心后退出 0，Watchdog 随之结束。
- 托盘重启：GUI 清理后退出 42，现有 Watchdog 立即重新启动 GUI，不产生第二个监督程序。
- 异常退出：最多重试三次，每次等待十秒；日志位于 `.config/io.github.clash-verge-buty.data/logs/watchdog/`。
- “系统代理守卫”继续在 GUI 中运行，职责与 Watchdog 无关。
- 启动时依据前次核心 PID、可执行文件路径和 `-d` 配置目录核验并清理残留 Mihomo。
- 仓库自带的 `tray-icon-sys.png` 保持原样，未被本修复改动。

## 构建

在已配置的工作区本地 Rust 和 Node 环境中：

```text
pnpm build
pnpm build:watchdog --target x86_64-pc-windows-msvc
```

指定目标时，GUI 可使用 `pnpm tauri build --bundles none --target x86_64-pc-windows-msvc`，随后执行同目标的 `pnpm build:watchdog`。正式/alpha CI 已加入 helper 构建、复制和便携包完整性检查。开发模式 `beforeDevCommand` 会构建并放置 debug helper。

图标由 `scripts/generate-watchdog-icon.ps1` 生成：以完整蓝猫资源为形状，按仓库自带 `tray-icon-sys.png` 的主体色做逐通道线性映射，提供 16、32、48、64、128、256 像素 ICO 与 PNG。

GUI 的任务管理器显示名来自 `Cargo.toml` 的 `description`（与 `package.json.description`、`tauri.conf.json` 的 short/longDescription 同步为 `Clash-Verge-Buty`）；Watchdog 的显示名在其 `build.rs` 中固定为 `Clash-Verge-Buty-Watchdog`。

本地宿主：Windows 10 x64、Visual Studio 2022 Community 17.14.18、Rust 1.85.1、Node 20.20.0、pnpm 8.15.9。Rust/Node/包管理器与缓存均放在工作区内，未更改全局 PATH。

## 验证

- 前端 `tsc` 与 Vite 发布构建通过。
- 主程序 `cargo fmt`、`cargo check --locked`、Windows x64 release 构建通过；稳定版 rustfmt 对项目原有 nightly-only 配置给出警告，不影响格式化和编译。
- Watchdog release 构建通过。`cargo test` 当前没有单元测试，行为验收使用 `scripts/test-watchdog.ps1` 黑盒回归：正常退出、失败一次后恢复、退出 42 重启、四次启动后耗尽三次重试、互斥单实例、外部应用继续存活，均通过。
- 实际 GUI 使用工作区隔离配置测试，关闭系统代理、自启、TUN，使用 43331/17897/19097 端口。启动与重复启动后恰好一个 GUI 和一个 Watchdog。
- 强制结束实际测试 GUI 后，监督程序自动恢复 GUI，旧核心退出、新核心重新监听 17897，验证通过。
- 便携包五种场景通过：完整、缺 helper、helper 为目录、只有 helper 没有 GUI、非标准 GUI 名称 fallback。
- 内嵌大/小图标提取成功，实际资源为完整黄色猫（原版珊瑚橙色板）；文件描述分别为 `Clash-Verge-Buty` 与 `Clash-Verge-Buty-Watchdog`。
- 测试后已清理本任务启动的进程。原安装版代理 PID 19936 仍监听 7897/9097，原自启计划任务不变。

使用项目本地 Rust 环境执行 `pwsh -File scripts/test-watchdog.ps1` 可复跑六项行为回归。测试产物位于工作区 `.tmp` 的独立目录。

## 使用修复包

关闭现有应用后，将修复便携包解压到应用目录，必须将 GUI 和 Watchdog 两个 EXE 一起使用。保留现有 `.config` 目录，不覆盖订阅和个人配置。本任务交付修复包，未替换 `C:/UserProgram/Clash-Verge-Buty` 中正在运行的安装版。

便携包为本地未签名构建。GitHub CI 和远端签名配置尚未实际运行。
