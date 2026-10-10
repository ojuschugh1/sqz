<!-- i18n-source: README.md sha256:e20e0153585ab14d62a29797338f75e9415f19639ba4f71317cabc907c46f4f6 -->
<!-- i18n:languages -->
[English](../README.md) | [简体中文](README.zh-CN.md)
<!-- /i18n:languages -->

<!-- i18n:note -->
> 本文是英文文档 [README.md](../README.md) 的译文。
<!-- /i18n:note -->

<p align="center">
  <pre>
  ███████╗ ██████╗ ███████╗
  ██╔════╝██╔═══██╗╚══███╔╝
  ███████╗██║   ██║  ███╔╝
  ╚════██║██║▄▄ ██║ ███╔╝
  ███████║╚██████╔╝███████╗
  ╚══════╝ ╚══▀▀═╝ ╚══════╝
  </pre>
</p>

<p align="center">
  <strong>面向 AI 编程智能体的注入前上下文压缩与会话去重</strong>
</p>

<p align="center">
  <sub>
    <strong>真实会话统计：</strong>
    压缩 3,003 次 ·
    <strong>节省的 token（tokens saved）：178,442</strong> ·
    平均减少 24.7% · 对模型已有的输出最高减少
    <strong>92%</strong>
  </sub>
</p>

<p align="center">
  <a href="https://thenextgentechinsider.com/pulse/sqz-tool-cuts-llm-token-use-by-92-for-file-heavy-ai-tasks"><img src="https://img.shields.io/badge/%231_Featured-NextGen_Tech_Insider-ff6600?style=for-the-badge&logo=newspaper&logoColor=white" alt="精选推荐"></a>
  <a href="https://hysenlabs.com/en/projects/ojuschugh1-sqz"><img src="https://img.shields.io/badge/Featured-Hysen_Labs-1f6feb?style=for-the-badge&logo=readme&logoColor=white" alt="Hysen Labs 精选推荐"></a>
</p>

<p align="center">
  <a href="https://crates.io/crates/sqz-cli"><img src="https://img.shields.io/crates/v/sqz-cli?logo=rust&logoColor=white&label=crates.io&color=e6522c" alt="Crates.io"></a>
  <a href="https://www.npmjs.com/package/sqz-cli"><img src="https://img.shields.io/npm/v/sqz-cli?logo=npm&logoColor=white&label=npm&color=cb3837" alt="npm"></a>
  <a href="https://pypi.org/project/sqz/"><img src="https://img.shields.io/pypi/v/sqz?logo=python&logoColor=white&label=PyPI&color=3775a9" alt="PyPI"></a>
  <a href="https://marketplace.visualstudio.com/items?itemName=ojuschugh1.sqz"><img src="https://img.shields.io/badge/VS%20Code-Marketplace-007acc?logo=visual-studio-code&logoColor=white" alt="VS Code"></a>
  <a href="https://addons.mozilla.org/en-US/firefox/addon/sqz-context-compression/"><img src="https://img.shields.io/badge/Firefox-Add--on-ff7139?logo=firefox-browser&logoColor=white" alt="Firefox"></a>
  <a href="https://plugins.jetbrains.com/plugin/31240-sqz--context-intelligence/"><img src="https://img.shields.io/badge/JetBrains-Plugin-000000?logo=jetbrains&logoColor=white" alt="JetBrains"></a>
  <a href="https://discord.gg/j8EEyH5dSB"><img src="https://img.shields.io/discord/1493251029075235076?logo=discord&logoColor=white&label=Discord&color=5865F2" alt="Discord"></a>
  <a href="https://github.com/ojuschugh1/homebrew-sqz"><img src="https://img.shields.io/badge/Homebrew-tap-FBB040?logo=homebrew&logoColor=white" alt="Homebrew"></a>
</p>

<p align="center">
  <a href="https://ojuschugh1.github.io/sqz/">文档</a> ·
  <a href="#install">安装</a> ·
  <a href="#how-it-works">工作原理</a> ·
  <a href="#supported-tools">支持的工具</a> ·
  <a href="#compress-any-mcp-server">MCP 代理</a> ·
  <a href="../docs/quality-benchmark.md">基准测试</a> ·
  <a href="../CHANGELOG.md">更新日志</a> ·
  <a href="https://discord.gg/j8EEyH5dSB">Discord</a>
</p>

---

AI 编程智能体的大部分输入 token 都花在了工具输出上：构建日志、测试运行、`git status`，以及每次编辑后再次读取的同一个文件。这些内容全部原样进入上下文窗口，并且在之后的每一轮中都会被重新发送。

sqz 在工具输出进入上下文之前对其进行压缩。按命令格式化器会保留智能体据以行动的内容（失败的测试、断言、`file:line`），丢掉其余内容。模型上下文中已有的任何内容都会以一个简短的引用返回，而不是返回内容本身。源代码原样通过，堆栈跟踪、迁移和凭据行也是如此。

```sh
cargo install sqz-cli sqz-mcp     # or: brew install ojuschugh1/sqz/sqz · npm i -g sqz-cli · pipx install sqz
sqz init                          # hooks + MCP server + agent guidance for every client it finds
```

<p align="center">
  <img src="../assets/demo.gif" alt="sqz：一个 200 行的文件被完整提供，再次读取第 41-80 行时变成一个行范围引用，cargo test 的输出折叠到只剩失败信息，sqz stats 同时显示这两项" width="800">
</p>

<sub>发布版二进制文件的真实输出（<code>assets/demo.tape</code>）。可用 <code>vhs assets/demo.tape</code> 重新生成。</sub>

```
Without sqz:                              With sqz:

cat auth.py (200 lines):   1,300 tokens    1,300 tokens  (source is served in full)
cargo test (1 failure):      366 tokens      185 tokens  (failure, assertion, file:line)
sed -n '41,80p' auth.py:     260 tokens       35 tokens  (§ref:…:L41-80§ to the read above)
git status, unchanged:       166 tokens       37 tokens  (§ref:…§)
─────────────────────────────────────      ─────────────
Total:                     2,092 tokens    1,557 tokens  (26% saved, nothing dropped)
```

单个 Rust 二进制文件，具有确定性，不调用 LLM，可离线运行。每个压缩结果都可以用 `sqz expand` 逐字节一致地恢复，但提到凭据、堆栈跟踪或迁移的输出除外，sqz 从不缓存这类输出。

> [!NOTE]
> **名称说明**：本仓库 [ojuschugh1/sqz](https://github.com/ojuschugh1/sqz) 是一个独立项目，与任何名称相似或功能相似的其他工具没有关联，与其他名称取自“squeeze”缩写的压缩项目也没有关联。如果你是从 crates.io、npm、PyPI 或 Homebrew 安装的 `sqz` / `sqz-cli` / `sqz-mcp`，那么它就来自本仓库。

## Token 节省

> 在 3,003 次真实压缩中平均减少 **24.7%** ·
> 对模型已有的输出返回 **13 个 token** 的引用 ·
> 在[质量基准测试](../docs/quality-benchmark.md)中保留了 **100%** 的智能体关键事实 ·
> 对源代码以及堆栈跟踪和凭据行的减少为 **0%**

一位开发者一周的数据，取自真实的 `sqz gain` 输出：

```
$ sqz gain
sqz token savings (last 7 days)
──────────────────────────────────────────────────
  04-13 │                              │   2,329 saved
  04-14 │                              │       0 saved
  04-15 │███                           │  12,954 saved
  04-16 │██                            │   9,223 saved
  04-17 │████                          │  14,752 saved
  04-18 │██████████████████████████████│ 105,569 saved
  04-19 │████████                      │  30,882 saved
  04-20 │█                             │   4,334 saved
──────────────────────────────────────────────────
  Total: 3,003 compressions, 178,442 tokens saved (24.7% avg reduction)
```

### 按命令压缩

单条命令的压缩效果（通过 `cargo test -p sqz-engine benchmarks` 测得）：

| 内容 | 压缩前 | 压缩后 | 节省 |
|---|---:|---:|---:|
| 重复的日志行 | 148 | 39 | **74%** |
| 大型 JSON 数组 | 259 | 111 | **57%** |
| JSON API 响应 | 64 | 53 | **17%** |
| Git diff | 61 | 54 | **11%** |
| 普通文本/文档 | 124 | 122 | **2%** |
| 堆栈跟踪（安全模式） | 82 | 82 | **0%** |

### 会话级去重

模型上下文中已有的任何内容都会以引用的形式返回。以下数据使用发布版二进制文件和一个临时数据库测得，测试夹具取自 `sqz/tests/quality_bench.rs` 和 `demo/`，单位是智能体看到的全部内容（引用本身加上 sqz 在 stderr 上的一行说明）的 cl100k token 数：

| 重复内容 | 不用 sqz | 使用 sqz | 节省 |
|---|---:|---:|---:|
| 再次运行未发生变化的 `git status` | 166 | 37 | **78%** |
| 之前读过的 200 行文件中的第 41-80 行 | 260 | 35 | **87%** |
| 相同的 `cargo test` 输出，没有任何变化 | 366 | 39 | **89%** |

关于实际会重复的内容，这里多说一句。一位用户统计了自己的 92 个 Claude Code 会话（[脚本](https://gist.github.com/harrison001/76250b61ad665bf9a377e7096dffdbe0)），发现在 542 次读取中，完全相同的整文件重复读取为 1 次，行范围的重复读取则占 8.4%。所以 sqz 并不把“同一个文件读了五遍”当作主打场景：实践中真正出现的重复是未变化的命令输出、已读文件的行范围，以及小幅编辑后重新读取的文件，每种情况都有自己的引用类型（[详情](../docs/stop-rereading-files.md)）。命令输出嘈杂、命令重复较多的会话收益最大。

## 安装 <a id="install"></a>

**预编译二进制文件**（无需编译器，适用于所有平台）：

```sh
# macOS / Linux
curl -fsSL https://raw.githubusercontent.com/ojuschugh1/sqz/main/install.sh | sh

# Windows (PowerShell)
irm https://raw.githubusercontent.com/ojuschugh1/sqz/main/install.ps1 | iex

# Any platform via npm
npm install -g sqz-cli

# macOS / Linux via Homebrew
brew tap ojuschugh1/sqz
brew install sqz
```

**通过 Cargo 从源码构建：**

```sh
cargo install sqz-cli sqz-mcp
```

`sqz-cli` 提供 `sqz` 二进制文件，`sqz-mcp` 提供 MCP 服务器。`sqz-engine` 是一个库依赖，它会自动编译，不需要单独安装。

**从源码构建**（`cargo install sqz-cli`）同样可行，但需要 C 工具链：

- Linux：`build-essential`（apt）或同等的软件包
- macOS：Xcode Command Line Tools（`xcode-select --install`）
- **Windows：Visual Studio Build Tools，并安装“使用 C++ 的桌面开发”（Desktop development with C++）工作负载**。没有这些工具，`cargo install` 会失败并报错 `linker link.exe not found`。如果你还没有安装它们，请改用上面的 PowerShell 或 npm 安装方式。

然后进行初始化：

```sh
sqz init --global     # hooks apply to every project on this machine
# or
sqz init              # hooks apply to just this project (.claude/settings.local.json)
```

`--global` 会写入 `~/.claude/settings.json`（即 [Anthropic 作用域表](https://docs.claude.com/en/docs/claude-code/settings)中的用户作用域），因此 sqz 钩子会在这台机器上的每个 Claude Code 会话中触发。这是首次安装时的常见做法。`~/.claude/settings.json` 中你已有的 `permissions`、`env`、`statusLine` 以及与 sqz 无关的钩子都会保留，sqz 会合并自己的条目，而不是覆盖原有内容。这些条目是它的钩子和两条允许规则：`Bash(set -o pipefail)` 和 `Bash(sqz compress --cmd *)`，它们只覆盖 sqz 改写时添加的部分。sqz 不会批准命令，最终仍由你自己的权限规则决定。当 Claude Code 在执行改写后的命令前询问你，或拒绝执行它时，提示信息中引用的是改写后的命令，例如对于 `git tag v0`，显示的是 `set -o pipefail; git tag v0 2>&1`。

不带参数的 `sqz init`（项目作用域）适用于只想让 sqz 在某一个仓库中生效的情况。如果 `~/.claude/settings.json` 已经包含 sqz 钩子，它就不会把这些钩子写入项目文件，因为它们已经在每个项目中运行。

安装之后，`sqz doctor` 会检查整条链路（二进制文件、数据库、shell 钩子、检测到了哪些客户端以及哪些客户端实际经过 sqz、最近是否有内容被压缩），并针对发现的每个问题给出修复方法。`sqz doctor --fix` 会应用这些修复。升级 sqz 之后请运行一次：它会把旧版本写入的钩子和指引更新到最新，删除当前 sqz 不再使用的钩子文件，以及 `~/.claude/settings.json` 中已有钩子在项目里的副本，把 sqz 二进制文件已不存在的钩子指向当前运行的二进制文件，并提示仍在运行其他 sqz 二进制文件的钩子。`sqz uninstall` 只删除 sqz 自己的条目，只有在文件中没有其他内容时才删除该文件，最后列出它保留的文件。如果没有终端来回答它的提示，就需要加上 `--yes`，`sqz reset` 也是如此。

### 在安装任何钩子之前，看看 sqz 本可以节省多少

如果你已经在使用 Claude Code 或 Kiro，你的会话记录就保存在磁盘上。用 sqz 引擎回放这些记录，就能基于你自己的会话得到反事实估算，全部在本地计算，不上传任何内容。下面是作者上周使用 Kiro IDE 工作的数据，只保留了摘要部分：

```sh
$ sqz discover --replay
sqz discover — counterfactual replay (last 7 days)
────────────────────────────────────────────────────────

  Kiro IDE: 35 sessions, 155 sub-agent runs, advisory

                            outputs    tokens in    avoidable
    shell                     6,297    1,917,662      454,028  23.7%
    file reads                4,880    7,854,071      188,239   2.4%
    other MCP servers         1,547    6,717,440    4,234,128  63.0%
    already through sqz         260    1,393,517            -
    out of reach              4,172    1,282,285            -

  Avoidable with no help from the agent:   0 tokens
  Avoidable if the agent uses every route: 4,876,395 tokens, 25.4% of all tool output
  Of that, 3,816,588 is the MCP proxy's lossy first pass.
```

每条输出都按照 sqz 在该客户端上实际能够接触到它的方式进行回放。Kiro 不允许 sqz 改写工具调用，所以在 Kiro 上第一个总计为零；在 Claude Code 上，shell 的统计行会分为钩子能自行改写的命令和钩子跳过的命令。第二个总计假设智能体把 shell 输出通过管道交给 `sqz compress`，用 sqz MCP 工具读取文件（无损，因此只有重复读取才计入），并让 MCP 服务器运行在 `sqz-mcp proxy` 之后；报告还会说明其中有多少来自代理在第一遍处理时截断长字符串和数组。网页抓取、编辑和子智能体的结果在所有客户端上都无法触及（out of reach），只计数，不回放。

它会扫描 `~/.claude/projects` 和 `~/.kiro/sessions`（Kiro CLI 和 IDE），你也可以用 `--transcripts PATH` 指定任意位置。每份会话记录都在一个全新的临时缓存上回放，因此去重引用不会假装跨越会话；报告会注明这是估算（estimate），因为它确实只是估算：这些输出当时并没有被压缩。

**只用一个智能体**？传入 `--only`（或 `--skip`）来限制写入哪些配置：

```sh
sqz init --only opencode              # just OpenCode, nothing else
sqz init --only opencode,codex        # OpenCode and Codex
sqz init --skip cursor,windsurf       # everything except Cursor and Windsurf
```

可用的名称：`claude`、`cursor`、`windsurf`、`cline`、`gemini`、`kiro`、`opencode`、`codex`。别名（`claude-code`、`gemini-cli`、`roo`、`kiro-cli`）同样可用。`--only` 和 `--skip` 不能同时使用。

### 手动安装（保留配置中的注释）

`sqz init` 会让你的配置文件经过 JSON 解析器读入再写回，以合并 sqz 条目，这会丢掉 `opencode.jsonc`（以及其他工具接受的类似带注释的 JSON 文件）中的所有注释。如果你精心为配置写了注释并希望保留它们，请改为手动安装。

**OpenCode**，分两步：

1. 把插件文件放到对应位置。`sqz` 会把生成的 TS 代码打印到 stdout，这样你就不必手写路径转义逻辑：

   ```sh
   mkdir -p ~/.config/opencode/plugins
   sqz print-opencode-plugin > ~/.config/opencode/plugins/sqz.ts
   ```

2. 自己把 MCP 条目添加到现有的 `opencode.jsonc` 中。把下面这段内容追加到顶层的 `mcp` 对象里（如果 `mcp` 对象不存在，就先创建它）：

   ```jsonc
   "sqz": {
     "type": "local",
     "command": ["sqz-mcp", "--transport", "stdio"],
     "enabled": true
   }
   ```

文件其余部分的注释保持不变。OpenCode 会自动发现插件文件，不需要在 `plugin` 数组中添加条目（添加后会导致重复加载，见 issue #10）。

**其他工具**：Claude Code 和 Gemini CLI 使用不支持注释的普通 JSON 配置，Cursor、Windsurf 和 Cline 使用 markdown 规则文件，Codex 则会在 `~/.codex/config.toml` 中得到一个保留你注释的 TOML 条目，因此对这些工具来说，自动方式不会造成破坏。对这些工具请使用 `sqz init --only <tool>`。已有的 `.gemini/settings.json` 会合并进 sqz 的钩子；如果它含有注释或无法解析，`sqz init` 不会改动它，而是打印出需要手动添加的条目。

之后运行 `sqz doctor`，查看为每个客户端配置了什么，以及它处于哪个集成层级。

## 工作原理 <a id="how-it-works"></a>

<p align="center">
  <img src="../assets/sqz-architecture.png" alt="sqz 系统架构" width="700" />
</p>

在 Claude Code、Gemini CLI、Copilot CLI、OpenCode 这些透明式（transparent）客户端上，sqz 会安装一个钩子来改写 shell 命令，让命令输出在到达 AI 工具之前先经过 sqz。在建议式（advisory）客户端上，则会告知智能体把输出通过管道交给 `sqz compress`，或使用 sqz MCP 工具。下表列出了每个客户端所属的层级。

```
Claude → git status → [sqz hook rewrites] → compressed output
```

会被压缩的内容：
- **Shell 输出**：按命令格式化器（git、cargo、npm/pnpm/yarn/bun、pytest、ruff、mypy、go、docker ps/images、kubectl describe/apply、terraform、gradle、maven、xcodebuild、adb logcat、dotnet、grep/rg、tree、curl 等）
- **JSON**：去掉 null 和空值，截断超过 500 个字符的字符串，紧凑编码，TOON 格式
- **日志**：折叠重复的行
- **测试输出**：只显示失败（针对 Rust、Go、Python、JS、JVM 的状态机解析器）

不会被压缩的内容：
- 源代码：首次读取时完整提供
- 堆栈跟踪、SQL 迁移、PEM 块和凭据行：交给安全模式，无论大小都原样通过，并且从不缓存；通过 shell 钩子时，按命令格式化器仍可能对它们周围测试运行的其余部分进行摘要
- 你的提示词和 AI 的回复：由 AI 工具控制，而不是 sqz

## 支持的工具 <a id="supported-tools"></a>

| 工具 | 集成方式 | 设置 |
|---|---|---|
| Claude Code | PreToolUse + PostToolUse 钩子 + MCP 服务器，透明式（transparent） | `sqz init` |
| Cursor | `.cursor/rules/sqz.mdc` 指引，建议式（advisory）；MCP 服务器需手动添加 | `sqz init` |
| Windsurf | `.windsurf/rules/sqz.md` 指引，建议式（advisory）；MCP 服务器需手动添加 | `sqz init` |
| Cline | `.clinerules` 指引，建议式（advisory）；MCP 服务器需手动添加 | `sqz init` |
| Gemini CLI | BeforeTool 钩子，透明式（transparent） | `sqz init` |
| Kiro | Steering + MCP 服务器，建议式（advisory） | `sqz init` |
| OpenCode | TypeScript 插件 + MCP 服务器，透明式（transparent） | `sqz init` |
| Codex CLI | AGENTS.md 指引 + MCP 服务器，建议式（advisory） | `sqz init` |
| Zed | AGENTS.md 指引 + MCP 服务器，建议式（advisory） | `sqz init` |
| Copilot CLI | preToolUse 钩子，透明式（transparent），在 `~/.copilot` 存在时安装 | `sqz init` |
| Copilot coding agent (CI) | 仓库级钩子，在云端沙箱中自举 | `sqz init --ci` + 提交 |
| 任意 CI 流水线 | [setup-sqz action](https://github.com/ojuschugh1/setup-sqz)：在 LLM 步骤之前压缩日志 | `uses: ojuschugh1/setup-sqz@v1` |
| 任意 MCP 服务器 | 用 `sqz-mcp proxy` 包装它，压缩它的工具结果 | 见下文 |
| VS Code | [扩展](https://marketplace.visualstudio.com/items?itemName=ojuschugh1.sqz) | 从 Marketplace 安装 |
| JetBrains | [插件](https://plugins.jetbrains.com/plugin/31240-sqz--context-intelligence/) | 从 Marketplace 安装 |
| Chrome | 浏览器扩展 | ChatGPT、Claude.ai、Gemini、Grok、Perplexity |
| [Firefox](https://addons.mozilla.org/en-US/firefox/addon/sqz-context-compression/) | 浏览器扩展 | 同样的网站 |

透明式（transparent）客户端有一个钩子，会自行把 shell 命令交给 sqz 处理。在建议式（advisory）客户端上，只有当智能体把输出通过管道交给 `sqz compress` 或调用 sqz MCP 工具时，sqz 才能节省 token；客户端内置的 shell 和文件工具会绕过它。`sqz doctor` 会显示每个客户端所属的层级。

## 压缩任意 MCP 服务器 <a id="compress-any-mcp-server"></a>

`sqz-mcp proxy` 位于你的智能体和任意 stdio MCP 服务器之间，压缩返回的内容：工具结果会经过完整的 sqz 流水线（重复内容使用去重引用，堆栈跟踪和机密信息走安全模式，错误结果保持不变），冗长的工具描述会被缩短，这样 `tools/list` 就不会再吃掉你的上下文窗口。代理会注入一个 `sqz_expand` 工具，让智能体能够逐字节一致地恢复任何已缓存的原始内容；提到凭据、堆栈跟踪或迁移的结果从不缓存。

在 MCP 配置中给服务器的命令加上前缀，即可包装该服务器：

```json
{
  "mcpServers": {
    "github": {
      "command": "sqz-mcp",
      "args": ["proxy", "--", "npx", "-y", "@modelcontextprotocol/server-github"]
    }
  }
}
```

选项：`--lazy-tools` 会把每个工具描述缩短为一句话，并注入一个 `sqz_tool_help` 工具，按需提供完整的原始文档（有些 MCP 服务器光是 `tools/list` 就要花掉 10-40k 个 token）；`--no-desc` 会原样保留描述；`--no-cache` 不存储任何内容，因此不会发送去重引用，也不会添加 `sqz_expand` 工具。它适用于所有 MCP 客户端（Claude Code、Cursor、Windsurf、Zed、Codex、Kiro 等），因为客户端看到的只是一个普通的 MCP 服务器。完整指南（包括该服务器自带的 `sqz_read_file` / `sqz_grep` / `sqz_list_dir` 工具和各客户端的配置）见 [MCP 上下文压缩](../docs/mcp-context-compression.md)。

## CLI

```sh
sqz init --global             # Install hooks for every project on this machine
sqz init                      # Install hooks for just this project
sqz init --only kiro          # Only configure Kiro (skip the rest)
sqz init --only opencode      # Only configure OpenCode (skip the rest)
sqz init --skip cursor        # Configure every agent except Cursor
sqz init --ci                 # Also write .github/hooks/sqz.json for Copilot agent CI runs
sqz compress <text>           # Compress (or pipe from stdin)
sqz compress --no-cache       # Compress without dedup (always full output)
sqz expand <ref>              # Recover original content from a §ref:HASH§ token
sqz compact                   # Evict stale context to free tokens
sqz reset                     # Clear dedup cache or compression stats
sqz gain                      # Show daily token savings (bar chart)
sqz gain --project .          # Per-project daily gains
sqz gain --days 30            # Last 30 days
sqz stats                     # Cumulative compression report (includes regret signals)
sqz stats --cost              # Estimated $ saved under a prompt-cached billing model
sqz stats --breakdown         # Per-command token usage breakdown
sqz stats --project .         # Stats for current project only
sqz stats --project list      # List all tracked projects
sqz stats --share             # Five-line share card sized for a paste into an issue or post
sqz doctor                    # Verify sqz is installed, wired into your clients, and active
sqz doctor --fix              # Apply the fixes doctor prints (run once after upgrading)
sqz uninstall                 # Remove sqz's hooks, guidance and MCP entries, keep the rest
sqz discover                  # Find missed savings (incl. your weakest-compressing commands)
sqz discover --replay         # Counterfactual replay of your agent transcripts (Claude Code, Kiro)
sqz recall "auth timeout"     # Full-text search everything sqz has compressed
sqz resume                    # Re-inject session context after compaction
sqz vizit                     # Live terminal dashboard (like htop for AI agents)
sqz hook claude               # Process a PreToolUse hook (Claude Code)
sqz hook claude-post          # Process a PostToolUse hook (Claude Code)
sqz hook kiro                 # Legacy; Kiro now uses steering + MCP (sqz init)
sqz print-opencode-plugin     # Print OpenCode plugin TS for manual install
sqz proxy --port 8080         # Local endpoint: compresses an API request body and sends it back, no forwarding
```

### 绕过去重的办法

当 sqz 第二次看到相同的内容时，会返回一个紧凑的 `§ref:HASH§` 引用标记，而不是完整文本。大多数模型都能正确处理，但有些模型（例如 GLM 5.1）无法解析这种引用格式，会陷入循环。有四种办法可以绕开这个问题：

```sh
# 1. Recover original content from a ref
sqz expand a1b2c3d4              # prefix match
sqz expand '§ref:a1b2c3d4§'     # paste the whole token

# 2. Compress without dedup (per-invocation)
echo "..." | sqz compress --no-cache

# 3. Disable dedup globally (env var)
export SQZ_NO_DEDUP=1

# 3b. Or just shorten the ref freshness window (seconds, default 1800).
# Useful on clients without a compaction hook; 0 disables refs.
export SQZ_REF_TTL_SECS=300

# 4. MCP passthrough tool (returns input byte-exact, zero transforms)
# Available via tools/list when sqz-mcp is running
```

### 召回：搜索 sqz 见过的所有内容

经过 sqz 的所有内容都会在本地建立索引（SQLite FTS5，保存在你的机器上，不会有任何内容离开它）。当智能体执行上下文精简（compaction），丢掉了一小时前的那条错误信息时，可以直接搜索它，而不必重新运行命令：

```sh
$ sqz recall "connection refused"
  1. [git · 2h ago]  … curl: (7) Failed to connect: >>connection refused<< on port 5432 …
     full content: sqz expand 3f9a1c05e2d84b17
```

智能体可以通过 `sqz_recall` MCP 工具使用同样的功能。

## 跟踪你自己的节省情况

随时在 shell 中运行 `sqz gain`，查看你自己的每日明细（输出的样子见上文“Token 节省”一节），运行 `sqz stats` 则可以查看完整的累计报告：

```sh
$ sqz stats
  📊 sqz compression stats
  ──────────────────────────────────────────────────

  178,442  tokens saved
  ↓  24.7% average reduction

  Compressions           3,003
  Tokens in              721,840
  Tokens out             543,398
  Tokens saved           178,442
  Avg reduction          24.7%

  🗄️  Cache
  ──────────────────────────────────────────────────
  Entries                43
  Size                   39.1 KB
```

加上 `--breakdown`，可以准确看到哪些命令消耗的 token 最多：

```sh
$ sqz stats --breakdown

  🔍 Top Token Consumers
  ──────────────────────────────────────────────────────────────────────
  command               calls  tokens in        out    saved
  ──────────────────────────────────────────────────────────────────────
  dedup                   249      45541       3237      93%
  stdin                    51      30851      24289      21%
  auto                    132      18288       7740      58%
  echo                     17       1050        558      47%
  ls -la                    8        948        948       0%
  cargo build               7        170        145      15%
  git status                4         56          8      86%
  ──────────────────────────────────────────────────────────────────────
```

`sqz stats` 和 `sqz gain` 统计的是 sqz 实际打印内容的 cl100k token 数，包括缩写图例和展开提示。同一类字符连续超过 2 KB 的片段（例如很长的分隔线）会按 2 KB 分片计数，误差可能约为 1% 再加 4 个 token。

### 如实核算：成本与遗憾

减少 token 并不会自动减少账单成本：在服务商的提示缓存下，大部分上下文在重新传输时享有约 90% 的折扣，而如果压缩丢掉了智能体需要的内容，反而要多花几轮（[arXiv:2607.12161](https://arxiv.org/abs/2607.12161)）。sqz 对这两方面都做了测量，而不是含糊其辞：

- `sqz stats --cost` 会在带提示缓存的计费模型下估算节省的美元金额（缓存写入一次 ×1.25，之后每一轮缓存读取 ×0.10），所有假设都会打印出来，并且可以覆盖（`--price-in`、`--reread-turns`、`--cache-write-mult`、`--cache-read-mult`）。这是保守的估算：如果没有缓存，同样的节省在每一轮都会按全额输入价格计费。
- `sqz stats` 会报告遗憾信号（regret signals）：快速重跑（quick re-runs）和引用展开（ref expands）。快速重跑指智能体在 2 分钟内重新产生了逐字节相同的输出，这次重复没有换来任何东西（Claude Code 的 PostToolUse 钩子从不用引用作答，所以它的重复计为普通压缩）；引用展开指 `§ref§` 引用标记被恢复为原始字节。两者都是“压缩丢掉了模型需要的内容”的代理指标。如果某条命令反复出现，说明它的格式化器需要改进，请提交 issue。

sqz 的设计已经避开了那项研究所测量的失败模式：压缩是确定性的，并且与查询无关（从不使服务商的提示缓存失效）；只处理新的工具输出（从不改写历史）；持续跟踪输出、运行服务器或把任务放到后台的命令保持不变（在 Claude Code 的非 bypass 模式下，PreToolUse 钩子会让管道、循环和 heredoc 保持原样，由 PostToolUse 钩子在成功运行后压缩它们的输出）；分组的命令列表和管道（Gemini CLI、Copilot CLI、bypass 模式下的 Claude Code）会跳过按命令格式化器；16 个 token 的净收益门槛会跳过连自身标记的开销都抵不回来的压缩。如果改写后可能运行与原命令不同的一组命令，该命令也会保持原样。在 `ls ${DEST:?}; echo next` 中，单独把 `ls` 接入管道，会让 `echo next` 在展开失败使 shell 停止之后仍然运行；而在命令列表作为一个分组整体经过 sqz 时，`cd build && make; make test` 中失败的 `cd` 会跳过整个分组，而不是只跳过 `make`。
<img width="3456" height="1918" alt="图片" src="https://github.com/user-attachments/assets/d308bc60-81f3-4935-9728-b30534e5b5e5" />


**按项目过滤：**

```sh
sqz stats --project .           # stats for current project only
sqz stats --project list        # list all tracked projects
sqz gain --project .            # daily gains for current project
sqz gain --days 30              # last 30 days instead of 7
sqz gain --days 30 --project .  # combine both
```

统计数据以 SQLite 格式存储在本地的 `~/.sqz/sessions.db` 中，不会有任何数据离开你的机器。

## 压缩原理

1. **按命令格式化器**，适用于以下命令：

   | 生态 | 命令 |
   |---|---|
   | Git | status、log、diff、show、stash、remote、fetch、push、pull、add、commit |
   | Rust | cargo build/test/clippy/check/nextest |
   | JavaScript | npm/pnpm/yarn/bun install/audit/outdated（失败时也包括 test 和 run）、tsc、eslint、biome |
   | Python | pytest、ruff、mypy、pip |
   | Go | go test（包括 `-json` 流）、go build、go vet、golangci-lint |
   | 云 | terraform plan/apply/destroy/refresh/init |
   | 容器 | docker/podman ps/images、kubectl describe/apply |
   | JVM | gradle build/test、maven |
   | 移动端 | xcodebuild（build + test）、adb logcat |
   | .NET | dotnet build/publish/test |
   | 系统 | grep/rg、tree、curl/wget |

   其他命令，包括 kubectl get 和 logs、gh、aws、gcloud、ls 和 find，都会经过通用的压缩流水线。

2. **表格压缩器**：对于没有专用格式化器的工具（`ps aux`、`netstat`、数据库 CLI），其列对齐的输出中成串的填充空格会被折叠为两个空格的分隔符。检测器很严格：缩进的行、代码、YAML 和 JSON 永远不会被匹配。
3. **结构摘要**：引擎提供了一个 API，可以把代码文件精简为导入、函数签名和调用图。钩子和 MCP 路径不使用它，它们会完整提供源代码。
4. **去重缓存**：SHA-256 内容哈希，跨会话持久保存。模型已有的逐字节相同的输出 = 13 个 token 的引用。模型已完整拥有的文件中的某个行范围（`sed -n '40,80p'`、带 `offset`/`limit` 的 `sqz_read_file`）= `§ref:HASH:L40-80§`。小幅编辑后的重新读取 = 发生变化的行，外加两侧各一行未变化的行，通过 sqz MCP 工具提供（shell 钩子会再次发送编辑后的文件）；分散在整个文件中的编辑会完整返回。只有当原始内容仍在上下文中时才会提供引用：窗口为 30 分钟，可通过 `SQZ_REF_TTL_SECS` 调整，并会在上下文精简（compaction）时重置。
5. **JSON 流水线**：去掉 null → 删除元数据字段（`__v`、`etag`、请求 ID 和关联 ID）→ 扁平化 → 折叠数组 → TOON 编码（无损的紧凑格式）。值为空字符串、空数组或空对象的字段在任意深度都会被删除，超过 500 个字符的字符串会被截断为前 500 个字符，后面接上 `...`。
6. **安全模式**：堆栈跟踪（跟踪头及其后的各帧）、行首的 SQL DDL、PEM 块和凭据行（键名看起来像机密的 `key=value` 或 `key: value`），无论是否带 ANSI 颜色，都会跳过流水线，无论大小都原样通过，只去掉 ANSI 代码，并且从不缓存。通过 shell 钩子时，按命令格式化器仍会先运行。只有当它的摘要把包含上述内容的每个段落的每一行都原样保留时，才会采用该摘要，因此它可能会对跟踪周围的其余测试输出进行摘要；否则整个输出原样通过。只是在这些结构之外提到密码、密钥、跟踪或迁移的输出会照常压缩，但同样不会被缓存，因此有损阶段从中丢掉的任何内容都无法展开。

如果某个阶段发生 panic，`sqz compress` 会原样打印输入，并在 stderr 上输出一行提示，同时保留命令的退出状态；Claude Code 的 PostToolUse 钩子则会让结果保持原样。

实测的质量数据，包括每个阶段会丢掉什么、从不触碰什么，见[质量基准测试](../docs/quality-benchmark.md)。完整的技术细节见 [docs/](../docs/README.md)。

## 配置

```toml
# ~/.sqz/presets/default.toml
[preset]
name = "default"
version = "1.0"

[compression.condense]
enabled = true
max_repeated_lines = 3

[compression.strip_nulls]
enabled = true

[budget]
warning_threshold = 0.70
default_window_size = 200000
```

### 按项目使用独立数据库

sqz 持久保存的所有内容（统计数据、去重缓存、会话）都存放在一个 SQLite 文件中，默认是 `~/.sqz/sessions.db`。设置 `SQZ_DB_PATH` 可以改为每个项目各用一个：

```sh
# e.g. in the project's .envrc (direnv)
export SQZ_DB_PATH="$PWD/.sqz/sessions.db"
```

所有入口都遵循这个设置：shell 钩子、`sqz stats`/`gain`/`expand`，以及 MCP 服务器（在 MCP 配置的 `env` 下设置）。建议使用绝对路径：相对路径会相对于进程运行时所在的目录解析。如果父目录不存在，会自动创建（模式 0700，与 `~/.sqz` 相同）。

## 隐私

- 零遥测：不传输任何数据，没有崩溃报告
- 完全离线：可在物理隔离的环境中运行
- 所有处理都在本地完成

## 开发

```sh
git clone https://github.com/ojuschugh1/sqz.git
cd sqz
cargo test --workspace
cargo build --release
```

## 许可证

[Elastic License 2.0](../LICENSE)（ELv2）：可以自由使用、fork 和修改。有两项限制：不得提供与之竞争的托管服务，不得移除许可证声明。

## 链接

- [文档索引](../docs/README.md)
- [如何让 AI 编程智能体不再反复读取同一个文件](../docs/stop-rereading-files.md)
- [MCP 上下文压缩](../docs/mcp-context-compression.md)
- [质量基准测试：压缩率与信息保留](../docs/quality-benchmark.md)
- [上下文腐化与上下文压缩](../docs/context-rot.md)
- [如何选择上下文压缩方案](../docs/comparison.md)
- [白皮书：注入前上下文压缩](../docs/whitepaper.md)
- [Token 节省基准测试](../docs/benchmark.md)
- [Discord](https://discord.gg/j8EEyH5dSB)
- [更新日志](../CHANGELOG.md)
- MCP Registry 名称：`mcp-name: io.github.ojuschugh1/sqz`

## Star 历史

如果 sqz 今天帮你节省了 token，点一个 star 可以帮助更多人发现它。如果你想展示自己的数据，`sqz stats --share` 会打印出你自己的数字。

<a href="https://star-history.dera.page/#ojuschugh1/sqz&Date">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://star-history.dera.page/svg?repos=ojuschugh1/sqz&type=Date&theme=dark" />
   <source media="(prefers-color-scheme: light)" srcset="https://star-history.dera.page/svg?repos=ojuschugh1/sqz&type=Date" />
   <img alt="Star 历史图表" src="https://star-history.dera.page/svg?repos=ojuschugh1/sqz&type=Date" width="600" />
 </picture>
</a>

## 贡献者

感谢所有为 sqz 贡献代码、修复和想法的人：

<a href="https://github.com/ojuschugh1/sqz/graphs/contributors">
  <img src="https://contrib.rocks/image?repo=ojuschugh1/sqz" alt="sqz 贡献者" />
</a>

也感谢每一位提交了详细 bug 报告、促成我们这些修复的人。精确的复现步骤让这个项目在每次发布时都变得更好。想加入他们吗？PR 会很快得到审查：从[未关闭的 issue](https://github.com/ojuschugh1/sqz/issues) 开始吧。

<sub>贡献者头像墙由 [contrib.rocks](https://contrib.rocks) 生成。</sub>
