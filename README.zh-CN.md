完全vibecoding的产物，请见谅😭

# Water Stream Calculator 1.17.1

[English](README.md) | 简体中文

Water Stream Calculator 1.17.1 是一个本地运行的 Minecraft 1.17.1 掉落物水道计算器，用于建模掉落物在水道中的运动。项目提供浏览器结构编辑器、Rust 模拟后端、可达候选搜索、结果查看，以及一份游戏实测样例数据。

## 功能

- 在本地数据 Web 中编辑和查看水道结构。
- 使用 Rust 后端模拟掉落物运动。
- 搜索可达结构候选，并将通过验证的结果写入数据 Web。
- 使用本地 split-run 格式保存运行结果。
- 直接打开自带样例数据 `游戏实测2`。

## 模型与搜索概览

### 掉落物运动模型

Rust 后端根据 Minecraft 1.17.1 源码建立掉落物运动模型，重点对应 `ItemEntity` tick 路径。模型覆盖水流推进、重力、碰撞修正、落地摩擦、冰面与粘液块地板影响，以及 tick 相位细节。

### 候选搜索

搜索器扩展结构前缀，模拟候选表现，并使用速度与驻留节奏等指标对有希望的状态评分。每一层扩展后，搜索器会先剪枝 frontier，再继续下一层搜索。

### 并行评估

在可达候选搜索中，前缀评估会按搜索线程设置分发到多个 worker 线程。候选排序是启发式的；最终是否成立仍由 Rust 模拟结果和命中率指标判断。

### 数据查看

本地数据 Web 会整理运行结果、稳态指标、CSV 导出和交互式速度图表，方便直观看到运动变化，并与游戏实测数据对照。

## Windows 快速开始

启动本地服务：

```powershell
.\start-windows.ps1
```

然后打开：

```text
http://127.0.0.1:8766
```

如果 PowerShell 阻止脚本执行，使用：

```powershell
powershell -ExecutionPolicy Bypass -File .\start-windows.ps1
```

停止服务：

```powershell
.\stop-windows.ps1
```

## 从源码构建

`bin/windows/` 中已包含 Windows 二进制文件。如需手动重新构建：

```powershell
cargo build --release --manifest-path .\rust-backend\Cargo.toml
```

`start-windows.ps1` 默认使用自带二进制文件。如果 Rust 源码更新且本机可用 `cargo`，脚本会重新构建并启动新的二进制文件。

## 项目目录

- `viewer/`
  用于结构编辑、模拟、搜索和结果查看的静态 Web 前端。
- `rust-backend/`
  本地服务、模拟、结果存储和可达候选搜索的 Rust 后端源码。
- `model/config/waterway-structure-parts.json`
  水道结构部件配置快照和格式参考。
- `assets/minecraft/textures/block/`
  前端展示使用的最小方块纹理资源。
- `data/viewer_data/runs/`
  数据 Web 的结果存储目录，包含样例数据 `游戏实测2`。
- `docs/`
  配置格式、模型说明和 Rust 架构文档。

## 运行数据

- 数据 Web 可见的结果保存在 `data/viewer_data/runs/`。
- 搜索诊断产物写入 `data/reachability-candidate-generator/`。
- 生成数据都位于当前项目目录内。

## 配置说明

`model/config/waterway-structure-parts.json` 记录项目使用的结构部件格式。当前 Rust 搜索目录仍编译在 `rust-backend/src/lib.rs` 中；该 JSON 文件作为外部配置格式参考保留。

## 文档

- [结构部件配置格式](docs/waterway-structure-parts-config.md)
- [Minecraft 1.17.1 掉落物水道模型](docs/item-waterway-model-1.17.1.md)
- [Rust 架构说明](docs/rust-architecture.md)
- [结构部件 JSON](model/config/waterway-structure-parts.json)
