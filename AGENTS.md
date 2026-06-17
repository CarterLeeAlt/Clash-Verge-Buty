# AGENTS.md

## Language

所有非代码说明、任务总结、PR 描述正文、Root Cause、Fix、Testing、验证结果说明默认使用中文。

代码、文件名、函数名、变量名、命令、日志关键字、错误原文、commit message、PR title 可以保留英文。

## Checks

提交前按修改范围要求运行：

- 前端改动：`pnpm exec tsc --noEmit`
- Rust 改动：`cargo fmt` 和 `cargo check`
- 完整检查：`pnpm check`

如果检查因环境限制失败，必须说明原因；不要把 `cargo fmt` 当成编译检查，也不要声称未完成的检查已通过。
