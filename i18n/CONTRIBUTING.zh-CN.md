<!-- i18n-source: CONTRIBUTING.md sha256:da3c749928a7f68e597cc1f0a0eba90065241548812430032de3fcc69b9d7188 -->
<!-- i18n:languages -->
[English](../CONTRIBUTING.md) | [简体中文](CONTRIBUTING.zh-CN.md) | [日本語](CONTRIBUTING.ja.md) | [한국어](CONTRIBUTING.ko.md) | [Español](CONTRIBUTING.es.md) | [Português (Brasil)](CONTRIBUTING.pt-BR.md)
<!-- /i18n:languages -->

<!-- i18n:note -->
> 本文是英文文档 [CONTRIBUTING.md](../CONTRIBUTING.md) 的译文。
<!-- /i18n:note -->

# 为 sqz 做贡献

感谢你有兴趣参与贡献。下面介绍如何开始。

## 环境准备

```sh
git clone https://github.com/ojuschugh1/sqz.git
cd sqz
cargo test --workspace
```

如果所有测试都通过，就可以开始了。

## 构建

工作区包含四个 crate：

| Crate | 说明 |
|---|---|
| `sqz_engine` | 核心压缩库，所有逻辑都在这里 |
| `sqz` | CLI 二进制文件（`sqz compress`、`sqz init` 等） |
| `sqz-mcp` | MCP 服务器（stdio/SSE 传输） |
| `sqz-wasm` | 用于浏览器扩展的 WASM 构建 |

构建全部内容：

```sh
cargo build --workspace
```

构建单个 crate：

```sh
cargo build -p sqz-engine
```

生成文档：

```sh
cargo doc --workspace --no-deps --open
```

## 运行测试

运行所有 crate 的全部测试：

```sh
cargo test --workspace
```

运行单个 crate 的测试：

```sh
cargo test -p sqz-engine
```

按名称运行指定的测试：

```sh
cargo test -p sqz-engine test_compress_json_applies_toon
```

只运行基于属性的测试（proptest）：

```sh
cargo test -p sqz-engine prop_
```

## 代码风格

- 提交前运行 `cargo fmt`：使用标准 Rust 格式，没有自定义规则
- 运行 `cargo clippy --workspace`：不允许有任何警告
- 遵循现有代码中的模式。如果文件使用 `thiserror`，就用 `thiserror`。如果使用 `serde`，就用 `serde`。不要为我们已经处理好的事情引入新的库
- 所有公开项都要写文档注释。保持简短实用：说明函数做什么，而不是它有多厉害
- 在有帮助的地方使用带代码示例的 `///` 文档注释。示例必须能够编译（`cargo test --doc`）

## 添加新的压缩阶段

1. 在 `sqz_engine/src/stages.rs` 中创建你的阶段结构体（如果比较复杂，也可以放在新文件中）
2. 实现 `CompressionStage` trait：
   ```rust
   impl CompressionStage for MyStage {
       fn name(&self) -> &str { "my_stage" }
       fn priority(&self) -> u32 { 50 } // lower = runs earlier
       fn process(&self, content: &mut Content, config: &StageConfig) -> Result<()> {
           // mutate content.raw in place
           Ok(())
       }
   }
   ```
3. 在 `sqz_engine/src/pipeline.rs` 中把它加入流水线，`new()` 和 `reload_preset()` 中都要添加
4. 在 `sqz_engine/src/preset.rs` 中添加配置字段（结构体 + TOML 反序列化）
5. 在 `pipeline.rs` 的 `stage_config_from_preset()` 中接入配置
6. 编写基于属性的测试（proptest），验证该阶段的不变量
7. 如果该阶段应默认启用，请在 `Preset::default()` 中更新默认预设

## 添加新的 CLI 命令

1. 在 `sqz/src/main.rs` 的 `Command` 枚举中添加一个变体
2. 在 `main()` 中添加 match 分支，把它分派到你的 `cmd_*` 函数
3. 编写 `cmd_*` 函数，遵循现有命令的模式
4. 使用 `require_engine()` 获取一个 `SqzEngine` 实例
5. 面向用户的输出打印到 stdout，诊断信息打印到 stderr（以 `[sqz]` 为前缀）

## 架构规则

- 所有压缩逻辑都在 `sqz_engine` 中。CLI、MCP 服务器和 WASM crate 都只是薄适配层，不应包含压缩逻辑
- 核心引擎不发起任何网络请求
- 没有遥测，没有数据分析，没有崩溃报告
- 所有正确性属性都要有基于属性的测试（proptest）
- 提交前必须通过 `cargo check --workspace`

## PR 流程

1. Fork 本仓库，并从 `main` 创建一个分支
2. 进行修改
3. 运行完整检查：
   ```sh
   cargo fmt --check
   cargo clippy --workspace
   cargo test --workspace
   cargo doc --workspace --no-deps
   ```
4. 写一条清晰的提交信息，说明改了什么以及为什么改
5. 向 `main` 提交 pull request
6. 处理审查意见

## 报告问题

- 使用 GitHub Issues
- 请包含：操作系统、sqz 版本（`sqz --version`）、复现步骤、预期行为与实际行为
- 对于测试失败：请附上完整的 `cargo test` 输出
- 可以用英文或你自己的语言撰写 issue 和 pull request。用英文能最快得到回复

## 翻译

README 和本指南的翻译位于 `i18n/`。规则、如何更新翻译以及如何添加语言，见 [i18n/README.md](README.md)。

## CLA

提交 pull request 即表示你同意[贡献者许可协议（CLA）](../CLA.md)。CLA 只有英文版本，适用的是英文文本。
