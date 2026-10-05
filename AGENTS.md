# AGENTS.md

## 语言

所有非代码说明、任务总结、PR 描述正文、Root Cause、Fix、Testing、验证结果说明默认使用简体中文。

代码、文件名、函数名、变量名、命令、日志关键字、错误原文、commit message、PR title 可以保留英文。

## 工作区边界（强制）

- 本项目为 Windows-only。工作区根目录即仓库所在目录；依赖、工具链、缓存、构建产物、测试产物、临时文件一律只放在工作区内。
- 禁止使用全局依赖：不得调用系统全局安装的 Rust、Node.js、pnpm、Python 等可项目化工具链；不得向 `~/.cargo`、`~/.rustup`、全局 `node_modules`、`pip install --user`、`npm install -g`、`cargo install` 等全局位置写入任何内容；不得修改用户全局 `PATH`、shell 配置或用户级工具配置文件。
- 禁止将临时产物或其他任何产物放置在工作区外（如 `C:\tmp`、系统 `%TEMP%`、用户主目录）。
- 删除一律使用可恢复删除（回收站 API），遵循全局用户指令的删除规范。

## 工具链与依赖（全部工作区本地）

- Rust：工作区 `.rustup\` 与 `.cargo\`，通过命令级环境变量 `RUSTUP_HOME`、`CARGO_HOME` 指定，并把 `.cargo\bin` 前置到命令级 `PATH`；离线构建用 `CARGO_NET_OFFLINE=true`。
- Node.js：工作区 `.toolchain\node-v20.20.0-win-x64\`，命令级 `PATH` 前置。
- pnpm：工作区 `.toolchain\pnpm\`（本地 npm 安装）；npm 缓存用环境变量 `NPM_CONFIG_CACHE` 指向 `.toolchain\npm-cache\`，pnpm store 用 `--store-dir` 指向 `.toolchain\pnpm-store\`。
- 进程级 `TEMP`/`TMP` 必须指向 `<工作区>\.tmp\<任务标识>\host-temp\` 之类的目录，防止工具链向系统 Temp 写文件。
- MSVC 链接依赖系统安装的 Visual Studio Build Tools（宿主平台，允许使用系统版本，不要求复制进工作区）。
- 安装依赖前先检查工作区内是否已有可用副本，禁止重复下载；外部网络访问遵循全局网络路由技能的判定。

## 构建产物（全部工作区内）

- Rust 构建输出：`src-tauri\target\`、`src-tauri\watchdog\target\`（`.gitignore` 已排除）。
- 前端构建输出：`dist\`（Vite 产物）。
- 打包交付物：统一放 `dist\watchdog-fix\`。注意 `vite build` 会清空整个 `dist\`，重新构建前端前先把交付物移出 `dist`，或在移动交付物前重建目标目录。
- 构建所需的 sidecar 与资源：`src-tauri\sidecar\`、`src-tauri\resources\`（从本地安装目录复制，`.gitignore` 已排除，不提交）。
- 构建产物默认按临时产物处理；用户要求保留的交付物除外。

## 临时产物

- 临时脚本、测试目录、下载缓存一律放 `<工作区>\.tmp\<任务或会话标识>\`，任务结束后按全局规则清理（回收站 API）。
- 不得把临时文件写入 `.tmp\` 之外的仓库目录（`.gitignore` 已覆盖的构建输出位置除外）。

## 检查

提交前按修改范围运行：

- 前端改动：`pnpm exec tsc --noEmit`
- Rust 改动：`cargo fmt` 和 `cargo check`
- 完整检查：`pnpm check`

如果检查因环境限制失败，必须说明原因；不要把 `cargo fmt` 当成编译检查，也不要声称未通过的检查已通过。
