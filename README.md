# forge-app

Forge 的原生 macOS 应用；当前仓库提供可在 Linux 测试的 Rust Core P0 骨架。

- `forge-protocol`：领域对象、Command、应答与带 seq 的 Event；生产依赖只有 serde。
- `forge-store`：SQLite WAL、schema v1 向前迁移与 SHA-256 blob。
- `forge-provider`：Provider 请求、流式事件和错误分类接口。
- `forge-tools`：Tool 接口、Unix 进程组取消、Clock 与可选 failpoints。
- `forge-core`：Runtime 命令应答和事件广播通道。
- `forge-testkit`：脚本化 Provider、真实进程树假命令和事件断言。
- `forge-state`：空占位，由 UI 开发（Forge Web Developer）负责。
- `forge-ui`：空占位，由 UI 开发（Forge Web Developer）负责。

Linux 使用 `rust-toolchain.toml` 固定的 Rust 1.100.0：

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo build --workspace
cargo test --workspace --all-features
cargo test --workspace
tree=$(cargo tree -p forge-core -p forge-state -e normal)
printf '%s\n' "$tree"
if printf '%s\n' "$tree" | grep -qi gpui; then exit 1; fi
```
