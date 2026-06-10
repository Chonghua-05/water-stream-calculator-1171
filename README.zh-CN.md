# Water Stream Calculator 1.17.1

Minecraft 1.17.1 掉落物匀速水道计算器，提供水道结构编辑、模型模拟、可达候选搜索和本地结果查看能力。

[English README](README.md)

## 包含内容

- `viewer/`
  数据 Web 前端，用于结构编辑、模拟、结果查看和搜索。
- `rust-backend/`
  Rust 后端源码，负责 `serve-web`、模拟、结果存储和可达候选搜索。
- `model/config/waterway-structure-parts.json`
  水道结构部件配置快照和格式参考。
- `assets/minecraft/textures/block/`
  前端展示使用的最小方块纹理资源。
- `data/viewer_data/runs/`
  自带一份样例数据 `游戏实测2`。
- `docs/`
  配置格式文档、模型说明文档和 Rust 架构说明。

## Windows 快速开始

1. 如需从源码重新构建：

```powershell
cargo build --release --manifest-path .\rust-backend\Cargo.toml
```

2. 启动本地服务：

```powershell
.\start-windows.ps1
```

3. 浏览器打开：

```text
http://127.0.0.1:8766
```

启动后，数据 Web 默认可见的样例结果为：

- `游戏实测2`

## 运行目录

- 前端静态文件目录：`viewer/`
- 查看和保存运行结果：`data/viewer_data/`
- 搜索诊断产物：`data/reachability-candidate-generator/`
- 方块纹理资源：`assets/minecraft/textures/block/`

## 说明

- `bin/windows/` 中自带 Windows 版求解器二进制文件；如果 `rust-backend/` 源码更新且本机可用 `cargo`，`start-windows.ps1` 会优先重新构建后启动。
- `model/config/waterway-structure-parts.json` 作为公开配置格式参考一并提供；当前实际搜索目录仍编译在 Rust 后端中，细节见架构文档。

## 关键文件

- [`docs/waterway-structure-parts-config.md`](docs/waterway-structure-parts-config.md)
- [`docs/item-waterway-model-1.17.1.md`](docs/item-waterway-model-1.17.1.md)
- [`docs/rust-architecture.md`](docs/rust-architecture.md)
- [`model/config/waterway-structure-parts.json`](model/config/waterway-structure-parts.json)
