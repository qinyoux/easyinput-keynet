# vendor/tao 补丁说明（companion）

本目录是 tao **0.37.1** 的冻结副本，带两处本地补丁（源码内
`[companion patch 2026-10-14]` 标记），由 `Cargo.toml` 的
`[patch.crates-io] tao = { path = "vendor/tao" }` 生效。

> 版本沿革：2026-10-14 首版复刻 easyop 的 vendor/tao 0.35.3；因本工程解析到
> tauri 2.12.3 → tao 0.37.1（补丁版本与图不匹配则 cargo 告警
> "patch was not used in the crate graph" 且**静默不生效**），遂从本地
> registry 缓存重新 vendor 0.37.1 并把两处改动照做一遍。

## 修的是什么

tao Windows 事件循环 runner 的共享计数是 `Rc`（**非原子**），而 `AppHandle` 是
`Send + Sync + Clone`，官方推荐用法就是"进线程前先 clone 一个"。多个线程并发
clone/drop `AppHandle` 会并发读改写同一个非原子计数，计数被写坏到 0 后，下一次
`Rc::clone` 命中 `assert_unchecked(strong != 0)` 的 UB 检查 ⇒ `panic_nounwind`
⇒ **进程直接 abort**（退出码 0xc0000409，WER 记为 BEX64，无任何业务报错）。

## 两处改动（Rc → Arc，只改计数原子性，不改行为）

| 文件 | 改动 |
|---|---|
| `src/platform_impl/windows/event_loop/runner.rs` | `use std::rc::Rc` → `use std::sync::Arc`；`type EventLoopRunnerShared<T> = Rc<EventLoopRunner<T>>` → `Arc<...>` |
| `src/platform_impl/windows/event_loop.rs` | `Rc::new(EventLoopRunner::new(...))` → `Arc::new(...)`（Arc 在 prelude，无需 import） |

自动 trait 不变：`EventLoopRunner<T>` 内部是 Cell/RefCell，本来就 `!Send + !Sync`，
所以 `Arc<EventLoopRunner<T>>` 与 `Rc<...>` 同为 `!Send + !Sync`。

## 何时可以删掉补丁

tao 上游把 `EventLoopRunnerShared` 改成 Arc/原子计数之后
（2026-09-30 查其 dev 分支仍是 `Rc`；相关问题 tauri-apps/tauri#10001 自 2024 年开着）。
删法：删 `Cargo.toml` 的 `[patch.crates-io]` 节 + `rm -rf vendor/tao`，
并重跑"并发 clone 压测"确认。

⚠️ 冻结代价：vendor 里的 tao 不再跟随上游。若将来 tauri 要求的 tao 主版本
再次变化，cargo 会告警补丁未生效（不会静默）——届时重新 vendor 对应版本并
照做这两处改动。
