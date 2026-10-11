<!-- i18n-source: README.md sha256:9de81c95b725cb7bc266ef2b2a2cbcd1982f6433ee6ee9c84cfaf284e8b2efc8 -->
<!-- i18n:languages -->
[English](../README.md) | [简体中文](README.zh-CN.md) | [日本語](README.ja.md) | [한국어](README.ko.md) | [Español](README.es.md) | [Português (Brasil)](README.pt-BR.md)
<!-- /i18n:languages -->

<!-- i18n:note -->
> この文書は英語版の[README.md](../README.md)を翻訳したものです。
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
  <strong>AIコーディングエージェント向けの、注入前のコンテキスト圧縮とセッションの重複排除</strong>
</p>

<p align="center">
  <sub>
    <strong>実際のセッションの統計：</strong>
    圧縮3,003回 ·
    <strong>178,442トークンを節約</strong> ·
    平均削減率24.7% ·
    モデルがすでに持っている出力では最大<strong>92%</strong>
  </sub>
</p>

<p align="center">
  <a href="https://thenextgentechinsider.com/pulse/sqz-tool-cuts-llm-token-use-by-92-for-file-heavy-ai-tasks"><img src="https://img.shields.io/badge/%231_Featured-NextGen_Tech_Insider-ff6600?style=for-the-badge&logo=newspaper&logoColor=white" alt="掲載"></a>
  <a href="https://hysenlabs.com/en/projects/ojuschugh1-sqz"><img src="https://img.shields.io/badge/Featured-Hysen_Labs-1f6feb?style=for-the-badge&logo=readme&logoColor=white" alt="Hysen Labsに掲載"></a>
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
  <a href="https://ojuschugh1.github.io/sqz/">ドキュメント</a> ·
  <a href="#install">インストール</a> ·
  <a href="#how-it-works">仕組み</a> ·
  <a href="#supported-tools">対応ツール</a> ·
  <a href="#compress-any-mcp-server">MCPプロキシ</a> ·
  <a href="../docs/quality-benchmark.md">ベンチマーク</a> ·
  <a href="../CHANGELOG.md">変更履歴</a> ·
  <a href="https://discord.gg/j8EEyH5dSB">Discord</a>
</p>

---

AIコーディングエージェントは、入力トークンの大半をツールの出力に費やしています。ビルドログ、テストの実行結果、`git status`、編集のたびに読み直される同じファイルなどです。これらはすべて未加工のままコンテキストウィンドウに入り、その後のターンでも毎回送り直されます。

sqzは、ツールの出力がコンテキストに入る前に圧縮します。コマンド別フォーマッターは、エージェントが次の行動に使う情報（失敗したテスト、アサーション、`file:line`）を残し、使わない情報を取り除きます。モデルがすでにコンテキストに持っている内容は、本文の代わりに短い参照として返されます。ソースコードは変更されずにそのまま通過し、スタックトレース、マイグレーション、認証情報の行も同様です。

```sh
cargo install sqz-cli sqz-mcp     # or: brew install ojuschugh1/sqz/sqz · npm i -g sqz-cli · pipx install sqz
sqz init                          # hooks + MCP server + agent guidance for every client it finds
```

<p align="center">
  <img src="../assets/demo.gif" alt="sqz：200行のファイルは全文が渡され、41-80行目の再読み込みは行範囲の参照になり、cargo testの出力は失敗箇所だけにまとめられ、sqz statsにはその両方が表示されます" width="800">
</p>

<sub>リリース版バイナリによる実際の出力です（<code>assets/demo.tape</code>）。<code>vhs assets/demo.tape</code>で再生成できます。</sub>

```
Without sqz:                              With sqz:

cat auth.py (200 lines):   1,300 tokens    1,300 tokens  (source is served in full)
cargo test (1 failure):      366 tokens      185 tokens  (failure, assertion, file:line)
sed -n '41,80p' auth.py:     260 tokens       35 tokens  (§ref:…:L41-80§ to the read above)
git status, unchanged:       166 tokens       37 tokens  (§ref:…§)
─────────────────────────────────────      ─────────────
Total:                     2,092 tokens    1,557 tokens  (26% saved, nothing dropped)
```

単一のRustバイナリで、動作は決定的、LLMは呼び出さず、オフラインで動作します。圧縮された結果はすべて、`sqz expand`を使えばバイト単位で同一の内容に復元できます。例外は認証情報、スタックトレース、マイグレーションに触れる出力で、sqzはこれらを決してキャッシュしません。

> [!NOTE]
> **名前の区別について**：このリポジトリ（[ojuschugh1/sqz](https://github.com/ojuschugh1/sqz)）は独立したプロジェクトであり、名前や動作が似ている他のツールや、「squeeze」を略した名前の他の圧縮プロジェクトとは、いずれも関係がありません。crates.io、npm、PyPI、Homebrewから`sqz` / `sqz-cli` / `sqz-mcp`をインストールした場合、それはこのリポジトリのものです。

## トークンの節約

> 実際の圧縮3,003回での平均削減率は**24.7%** ·
> モデルがすでに持っている出力に対する参照は**13トークン** ·
> [品質ベンチマーク](../docs/quality-benchmark.md)での、エージェントにとって重要な事実の保持率は**100%** ·
> ソースコード、さらにスタックトレースと認証情報の行での削減率は**0%**

ある開発者の一週間を、実際の`sqz gain`の出力で測定したものです：

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

### コマンド別の圧縮

単一コマンドの圧縮（`cargo test -p sqz-engine benchmarks`で測定）：

| 内容 | 圧縮前 | 圧縮後 | 節約 |
|---|---:|---:|---:|
| 繰り返されるログ行 | 148 | 39 | **74%** |
| 大きなJSON配列 | 259 | 111 | **57%** |
| JSON APIのレスポンス | 64 | 53 | **17%** |
| Gitのdiff | 61 | 54 | **11%** |
| 文章・ドキュメント | 124 | 122 | **2%** |
| スタックトレース（セーフモード） | 82 | 82 | **0%** |

### セッション単位（重複排除あり）

モデルがすでにコンテキストに持っている内容は、参照として返されます。リリース版バイナリと使い捨てのデータベースを使い、`sqz/tests/quality_bench.rs`と`demo/`のフィクスチャで測定しました。数値は、エージェントが目にするすべての内容（参照と、stderrに出るsqzの一行の注記）のcl100kトークン数です：

| 繰り返しの内容 | sqzなし | sqzあり | 節約 |
|---|---:|---:|---:|
| 出力が変わらない`git status`の再実行 | 166 | 37 | **78%** |
| 以前に読んだ200行のファイルの41-80行目 | 260 | 35 | **87%** |
| 何も変わっていない、同じ`cargo test`の出力 | 366 | 39 | **89%** |

実際に何が繰り返されるのかについて、少し補足します。自分のClaude Codeセッション92件を数えたあるユーザー（[スクリプト](https://gist.github.com/harrison001/76250b61ad665bf9a377e7096dffdbe0)）によると、ファイル全体がまったく同じ内容で再読み込みされたのは542回の読み込みのうち1回で、行範囲の再読み込みは8.4%でした。そのためsqzは、「同じファイルを五回読む」ケースを売りにしていません。実際に起きる繰り返しは、変更のないコマンドの出力、すでに読んだファイルの行範囲、小さな編集の後に読み直されるファイルであり、それぞれに専用の参照の種類があります（[詳細](../docs/stop-rereading-files.md)）。コマンドの出力が多く、同じコマンドを繰り返すセッションで最も大きな効果が出ます。

## インストール<a id="install"></a>

**ビルド済みバイナリ**（コンパイラは不要で、すべてのプラットフォームで動作します）：

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

**Cargoでソースからビルドする場合：**

```sh
cargo install sqz-cli sqz-mcp
```

`sqz-cli`は`sqz`バイナリを、`sqz-mcp`はMCPサーバーを提供します。`sqz-engine`はライブラリとしての依存関係で、自動的にコンパイルされるため、別途インストールする必要はありません。

**ソースからのビルド**（`cargo install sqz-cli`）も使えますが、Cツールチェーンが必要です：

- Linux：`build-essential`（apt）またはそれに相当するもの
- macOS：Xcode Command Line Tools（`xcode-select --install`）
- **Windows：「C++ によるデスクトップ開発」（Desktop development with C++）ワークロードを含むVisual Studio Build Tools**。これがないと、`cargo install`は`linker link.exe not found`で失敗します。まだ用意していない場合は、代わりに上のPowerShellまたはnpmでのインストールを使ってください。

次に初期化します：

```sh
sqz init --global     # hooks apply to every project on this machine
# or
sqz init              # hooks apply to just this project (.claude/settings.local.json)
```

`--global`は`~/.claude/settings.json`（[Anthropicのスコープ表](https://docs.claude.com/en/docs/claude-code/settings)でいうユーザースコープ）に書き込むため、sqzのフックはこのマシン上のすべてのClaude Codeセッションで動作します。初回のインストールでは、通常こちらを使います。`~/.claude/settings.json`にある既存の`permissions`、`env`、`statusLine`、およびsqzと無関係なフックは保持されます。sqzは上書きせず、自分のエントリーをマージします。追加されるエントリーは、sqzのフックと二つの許可ルール`Bash(set -o pipefail)`と`Bash(sqz compress --cmd *)`で、これらのルールが対象とするのは書き換えによって追加される部分だけです。sqzがコマンドを承認することはなく、判断するのは引き続き利用者自身の権限ルールです。書き換えられたコマンドの実行前にClaude Codeが確認を求めたり、実行を拒否したりする場合、そのメッセージには書き換え後のコマンドが引用されます。たとえば`git tag v0`であれば`set -o pipefail; git tag v0 2>&1`です。

オプションなしの`sqz init`（プロジェクトスコープ）は、sqzを一つのリポジトリ内だけで有効にしたい場合に便利です。`~/.claude/settings.json`にすでにsqzのフックがある場合は、それらがすでにすべてのプロジェクトで動作するため、プロジェクトのファイルには含めません。

インストール後は、`sqz doctor`が一連の構成全体を確認します。対象はバイナリ、データベース、シェルフック、検出されたクライアントのうち実際にsqzを経由しているのはどれか、最近何かが圧縮されたかどうかです。見つかった不備ごとに修正方法を表示します。`sqz doctor --fix`はそれらの修正を適用します。sqzをアップグレードした後は、これを一度実行してください。このコマンドは、古いバージョンが書き込んだフックとガイダンスを最新の状態にし、現在のsqzが使わなくなったフックファイルと、`~/.claude/settings.json`にすでにあるフックのプロジェクト側のコピーを削除し、sqzバイナリがなくなったフックの参照先を実行中のバイナリに切り替え、まだ別のsqzバイナリを実行しているフックがあればそれを知らせます。`sqz uninstall`はsqz自身のエントリーだけを削除します。ファイルを削除するのはその中に他に何も残っていない場合だけで、最後に残したファイルを一覧表示します。確認のプロンプトに答えるターミナルがない場合は、`sqz reset`と同じく`--yes`が必要です。

### フックをインストールする前に、sqzで節約できたはずの量を確認する

すでにClaude CodeやKiroを使っているなら、トランスクリプトはディスク上にあります。それをsqzエンジンでリプレイすると、自分のセッションについての反実仮想の推定値が得られます。計算はローカルで行われ、何もアップロードされません。以下は、作者のKiro IDEでの直近一週間の作業を、要約部分だけに絞って示したものです：

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

各出力は、そのクライアントでsqzが実際にその出力に到達できたはずの経路でリプレイされます。Kiroではsqzがツール呼び出しを書き換えることができないため、一つ目の合計はゼロになります。Claude Codeでは、シェルの行が、フックが自動で書き換えるコマンドと、フックがスキップするコマンドに分かれます。二つ目の合計は、エージェントがシェルの出力を`sqz compress`にパイプし、sqzのMCPツールでファイルを読み（可逆なので、数えられるのは繰り返しだけです）、MCPサーバーを`sqz-mcp proxy`の背後で動かすことを前提としています。そのうちどれだけが、プロキシが初回に長い文字列と配列を切り詰めた分なのかも、レポートに示されます。Webの取得、編集、サブエージェントの結果はどのクライアントでも対象外（out of reach）で、リプレイせずに件数だけを数えます。

`~/.claude/projects`と`~/.kiro/sessions`（Kiro CLIとIDE）をスキャンします。`--transcripts PATH`で任意の場所を指定することもできます。各トランスクリプトは新しい使い捨てのキャッシュに対してリプレイされるため、重複排除の参照が、セッションをまたいで効いているかのように見せかけることはありません。また、レポートに「推定値（estimate）」と書かれているのは、実際にそうだからです。これらの出力は、当時は圧縮されていませんでした。

**使うエージェントが一つだけなら**、`--only`（または`--skip`）で書き込む設定を限定できます：

```sh
sqz init --only opencode              # just OpenCode, nothing else
sqz init --only opencode,codex        # OpenCode and Codex
sqz init --skip cursor,windsurf       # everything except Cursor and Windsurf
```

指定できる名前：`claude`、`cursor`、`windsurf`、`cline`、`gemini`、`kiro`、`opencode`、`codex`。別名（`claude-code`、`gemini-cli`、`roo`、`kiro-cli`）も使えます。`--only`と`--skip`は併用できません。

### 手動インストール（設定ファイルのコメントを残す場合）

`sqz init`はsqzのエントリーをマージするために、設定ファイルをJSONパーサーで読み込んでから書き戻します。そのため、`opencode.jsonc`（および他のツールが受け付ける同様のコメント付きJSONファイル）にあるコメントはすべて失われます。設定に丁寧にコメントを付けていて、それを残したい場合は、代わりに手動でインストールしてください。

**OpenCode**は二つの手順で設定します：

1. プラグインファイルを所定の場所に置きます。`sqz`が生成したTSをstdoutに出力するので、パスのエスケープ処理を手で書く必要はありません：

   ```sh
   mkdir -p ~/.config/opencode/plugins
   sqz print-opencode-plugin > ~/.config/opencode/plugins/sqz.ts
   ```

2. 既存の`opencode.jsonc`に、MCPのエントリーを自分で追加します。トップレベルの`mcp`オブジェクトの中にこのブロックを追加してください（`mcp`オブジェクトがなければ作成します）：

   ```jsonc
   "sqz": {
     "type": "local",
     "command": ["sqz-mcp", "--transport", "stdio"],
     "enabled": true
   }
   ```

ファイルの他の部分にあるコメントはそのまま残ります。OpenCodeはプラグインファイルを自動的に検出するため、`plugin`配列へのエントリーは不要です（追加すると二重に読み込まれます。Issue #10を参照してください）。

同じプラグインファイルが、OpenCode 1でも2でも読み込まれます。OpenCode 2はsqz 2.0.0以前が書き込んだプラグインを読み込まないため、sqzをアップグレードした後は`sqz doctor --fix`を実行して（または上の`print-opencode-plugin`の行をもう一度実行して）、プラグインを書き直してください。

**その他のツール**：Claude CodeとGemini CLIはコメントに対応していない通常のJSON設定を使い、Cursor、Windsurf、Clineにはmarkdownのルールファイルが作られ、Codexには`~/.codex/config.toml`に、コメントを保持するTOMLのエントリーが追加されます。そのため、これらのツールでは自動での設定でも内容が損なわれません。これらには`sqz init --only <tool>`を使ってください。既存の`.gemini/settings.json`にはsqzのフックがマージされます。そのファイルにコメントがあるか、解析できない場合、`sqz init`はファイルに手を加えず、手動で追加するエントリーを表示します。

その後`sqz doctor`を実行すると、各クライアントに何が設定されたか、どの統合レベルにあるかを確認できます。

## 仕組み<a id="how-it-works"></a>

<p align="center">
  <img src="../assets/sqz-architecture.png" alt="sqzのシステムアーキテクチャ" width="700" />
</p>

透過型（transparent）のクライアント（Claude Code、Gemini CLI、Copilot CLI、OpenCode）では、sqzはシェルコマンドを書き換えるフックをインストールし、出力がAIツールに届く前にsqzを通るようにします。助言型（advisory）のクライアントでは、出力を`sqz compress`にパイプするか、sqzのMCPツールを使うようエージェントに指示します。各クライアントの統合レベルは下の表のとおりです。

```
Claude → git status → [sqz hook rewrites] → compressed output
```

圧縮されるもの：
- **シェルの出力**：コマンド別フォーマッター（git、cargo、npm/pnpm/yarn/bun、pytest、ruff、mypy、go、docker ps/images、kubectl describe/apply、terraform、gradle、maven、xcodebuild、adb logcat、dotnet、grep/rg、tree、curlなど）
- **JSON**：nullと空の値の除去、500文字を超える文字列の切り詰め、コンパクトなエンコード、TOON形式
- **ログ**：繰り返される行をまとめます
- **テストの出力**：失敗だけを表示します（Rust、Go、Python、JS、JVM向けのステートマシン型パーサー）

圧縮されないもの：
- ソースコード：初回の読み込みでは全文を渡します
- スタックトレース、SQLマイグレーション、PEMブロック、認証情報の行：セーフモードに回され、サイズにかかわらず変更されずにそのまま渡され、キャッシュもされません。ただしシェルフックでは、それらの周りにあるテスト実行の残りの部分を、コマンド別フォーマッターが要約することもあります
- ユーザーのプロンプトとAIの応答：sqzではなくAIツールが制御します

## 対応ツール<a id="supported-tools"></a>

| ツール | 統合方法 | セットアップ |
|---|---|---|
| Claude Code | PreToolUse + PostToolUseフック + MCPサーバー、透過型（transparent） | `sqz init` |
| Cursor | `.cursor/rules/sqz.mdc`のガイダンス、助言型（advisory）。MCPサーバーは手動で追加 | `sqz init` |
| Windsurf | `.windsurf/rules/sqz.md`のガイダンス、助言型（advisory）。MCPサーバーは手動で追加 | `sqz init` |
| Cline | `.clinerules`のガイダンス、助言型（advisory）。MCPサーバーは手動で追加 | `sqz init` |
| Gemini CLI | BeforeToolフック、透過型（transparent） | `sqz init` |
| Kiro | Steering + MCPサーバー、助言型（advisory） | `sqz init` |
| OpenCode | OpenCode 1と2向けのTypeScriptプラグイン + MCPサーバー、透過型（transparent） | `sqz init` |
| Codex CLI | AGENTS.mdのガイダンス + MCPサーバー、助言型（advisory） | `sqz init` |
| Zed | AGENTS.mdのガイダンス + MCPサーバー、助言型（advisory） | `sqz init` |
| Copilot CLI | preToolUseフック、透過型（transparent）。`~/.copilot`がある場合 | `sqz init` |
| Copilot coding agent（CI） | リポジトリ単位のフック。クラウドのサンドボックス内で自身をセットアップ | `sqz init --ci` + コミット |
| 任意のCIパイプライン | [setup-sqzアクション](https://github.com/ojuschugh1/setup-sqz)：LLMのステップの前にログを圧縮 | `uses: ojuschugh1/setup-sqz@v1` |
| 任意のMCPサーバー | `sqz-mcp proxy`でラップし、そのツールの結果を圧縮 | 下記を参照 |
| VS Code | [拡張機能](https://marketplace.visualstudio.com/items?itemName=ojuschugh1.sqz) | Marketplaceからインストール |
| JetBrains | [プラグイン](https://plugins.jetbrains.com/plugin/31240-sqz--context-intelligence/) | Marketplaceからインストール |
| Chrome | ブラウザー拡張機能 | ChatGPT、Claude.ai、Gemini、Grok、Perplexity |
| [Firefox](https://addons.mozilla.org/en-US/firefox/addon/sqz-context-compression/) | ブラウザー拡張機能 | 同じサイト |

透過型（transparent）のクライアントには、シェルコマンドを自動的にsqz経由にするフックがあります。助言型（advisory）のクライアントでは、エージェントが出力を`sqz compress`にパイプするか、sqzのMCPツールを呼び出した場合にだけ、sqzがトークンを節約します。クライアントに組み込まれたシェルやファイルのツールはsqzを経由しません。各クライアントの統合レベルは`sqz doctor`で確認できます。

## 任意のMCPサーバーの出力を圧縮する<a id="compress-any-mcp-server"></a>

`sqz-mcp proxy`はエージェントと任意のstdio MCPサーバーの間に入り、返ってくる内容を圧縮します。ツールの結果はsqzの完全なパイプラインを通り（繰り返しには重複排除の参照、スタックトレースと秘密情報にはセーフモードを使い、エラーの結果はそのまま）、冗長なツールの説明は短くまとめられるため、`tools/list`がコンテキストウィンドウを食いつぶすことがなくなります。プロキシは`sqz_expand`ツールを追加するので、エージェントはキャッシュされた元の内容をバイト単位で同一に復元できます。認証情報、スタックトレース、マイグレーションに触れる結果は決してキャッシュされません。

MCPの設定でサーバーのコマンドの前に付け足して、サーバーをラップします：

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

フラグ：`--lazy-tools`はすべてのツールの説明を一文に短縮し、元の完全なドキュメントを必要に応じて返す`sqz_tool_help`ツールを追加します（MCPサーバーによっては`tools/list`だけで10-40kトークンを使います）。`--no-desc`は説明を一字一句そのまま残します。`--no-cache`は何も保存しないため、重複排除の参照を送らず、`sqz_expand`ツールも追加しません。クライアントからは通常のMCPサーバーにしか見えないため、すべてのMCPクライアント（Claude Code、Cursor、Windsurf、Zed、Codex、Kiroなど）で動作します。サーバー自身の`sqz_read_file` / `sqz_grep` / `sqz_list_dir`ツールやクライアントごとの設定を含む完全なガイドは、[MCPのコンテキスト圧縮](../docs/mcp-context-compression.md)にあります。

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

### 重複排除の回避手段

sqzは同じ内容を二度目に見ると、全文の代わりに短い`§ref:HASH§`トークンを返します。ほとんどのモデルはこれを問題なく扱えますが、一部のモデル（GLM 5.1など）は参照の形式を解釈できず、ループに陥ります。回避する方法は四つあります：

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

### Recall：sqzが見たものをすべて検索する

sqzを通過したものはすべてローカルでインデックス化されます（SQLite FTS5を使い、手元のマシン上にあり、外部には何も送られません）。エージェントがコンテキストのコンパクションを行い、一時間前のエラーメッセージが失われたときは、コマンドを再実行する代わりにそれを検索できます：

```sh
$ sqz recall "connection refused"
  1. [git · 2h ago]  … curl: (7) Failed to connect: >>connection refused<< on port 5432 …
     full content: sqz expand 3f9a1c05e2d84b17
```

エージェントは同じ機能を`sqz_recall` MCPツールとして利用できます。

## 自分の節約量を確認する

シェルでいつでも`sqz gain`を実行すると、自分の日ごとの内訳を確認できます（出力の例は上の「トークンの節約」セクションを参照してください）。累積の完全なレポートは`sqz stats`で確認できます：

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

`--breakdown`を付けると、どのコマンドが最も多くのトークンを消費しているかを正確に確認できます：

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

`sqz stats`と`sqz gain`は、sqzが実際に出力した内容（略語の凡例と展開のヒントを含む）のcl100kトークンを数えます。長い区切り線のように、同じ文字種が2 KBを超えて続く部分は2 KBずつ区切って数えるため、約1%プラス4トークンの誤差が出ることがあります。

### 正直な集計：コストとリグレット

トークンの削減が、そのまま請求額の削減になるわけではありません。プロバイダーのプロンプトキャッシュがあると、コンテキストの大部分は約90%の割引で再送信されます。また、エージェントが必要としていたものを落とす圧縮は、かえって余分なターンというコストを生みます（[arXiv:2607.12161](https://arxiv.org/abs/2607.12161)）。sqzはごまかさずに両方の側面を測定します：

- `sqz stats --cost`は、プロンプトキャッシュを前提とした課金モデル（キャッシュの書き込みは一度だけ×1.25、キャッシュの読み込みはその後のターンごとに×0.10）で、節約できた金額をドルで推定します。前提はすべて表示され、上書きできます（`--price-in`、`--reread-turns`、`--cache-write-mult`、`--cache-read-mult`）。これは控えめな推定値です。キャッシュがなければ、同じ節約分は毎ターン、入力の正規料金で課金されます。
- `sqz stats`はリグレットシグナル（regret signals）を報告します。一つは短時間での再実行（quick re-runs）で、エージェントがバイト単位で同一の出力を2分以内に再び生成したことを示し、その繰り返しでは何も得られていません。ただし、Claude CodeのPostToolUseフックは参照で応答することがないため、その繰り返しは通常の圧縮として数えられます。もう一つは参照の展開（ref expands）で、`§ref§`トークンが元のバイト列に復元されたことを示します。どちらも「圧縮によって、モデルが必要としていたものが失われた」ことを示す代理指標です。同じコマンドが繰り返し現れる場合は、そのフォーマッターに改善が必要です。Issueを作成してください。

sqzの設計は、その研究が測定した失敗パターンをすでに回避しています。圧縮は決定的で、クエリに依存しません（プロバイダーのプロンプトキャッシュを無効にすることはありません）。手を加えるのは新しいツールの出力だけです（履歴が書き換えられることはありません）。出力を追い続けるコマンド、サーバーを動かすコマンド、ジョブをバックグラウンドに回すコマンドには手を加えません（バイパスモード以外のClaude Codeでは、PreToolUseフックはパイプライン、ループ、ヒアドキュメントを書かれたとおりに残し、実行が成功した後にPostToolUseフックがその出力を圧縮します）。グループ化されたリストとパイプライン（Gemini CLI、Copilot CLI、バイパスモードのClaude Code）では、コマンド別フォーマッターを使いません。また、16トークンの正味節約ゲートにより、自身のマーカーの分を回収できない圧縮はスキップされます。書き換えによって元とは異なる組み合わせのコマンドが実行されうる場合も、コマンドは書かれたとおりに残されます。`ls ${DEST:?}; echo next`で`ls`だけをパイプすると、展開の失敗でシェルが停止した後に`echo next`が実行されてしまいます。また、リストが一つのグループとしてsqzを通る場合、`cd build && make; make test`の`cd`が失敗すると、`make`だけでなくグループ全体がスキップされてしまいます。
<img width="3456" height="1918" alt="画像" src="https://github.com/user-attachments/assets/d308bc60-81f3-4935-9728-b30534e5b5e5" />


**プロジェクトごとの絞り込み：**

```sh
sqz stats --project .           # stats for current project only
sqz stats --project list        # list all tracked projects
sqz gain --project .            # daily gains for current project
sqz gain --days 30              # last 30 days instead of 7
sqz gain --days 30 --project .  # combine both
```

統計はローカルのSQLite（`~/.sqz/sessions.db`）に保存され、マシンの外には何も送られません。

## 圧縮の仕組み

1. 以下のコマンドに対応した**コマンド別フォーマッター**：

   | エコシステム | コマンド |
   |---|---|
   | Git | status、log、diff、show、stash、remote、fetch、push、pull、add、commit |
   | Rust | cargo build/test/clippy/check/nextest |
   | JavaScript | npm/pnpm/yarn/bun install/audit/outdated（testとrunは失敗した場合）、tsc、eslint、biome |
   | Python | pytest、ruff、mypy、pip |
   | Go | go test（`-json`ストリームを含む）、go build、go vet、golangci-lint |
   | クラウド | terraform plan/apply/destroy/refresh/init |
   | コンテナ | docker/podman ps/images、kubectl describe/apply |
   | JVM | gradle build/test、maven |
   | モバイル | xcodebuild（build + test）、adb logcat |
   | .NET | dotnet build/publish/test |
   | システム | grep/rg、tree、curl/wget |

   kubectl getとlogs、gh、aws、gcloud、ls、findを含むその他のコマンドは、汎用の圧縮パイプラインを通ります。

2. **テーブルコンパクター**：専用のフォーマッターがないツール（`ps aux`、`netstat`、データベースのCLI）の列揃えされた出力では、桁揃えのための連続した空白が、二つのスペースによる区切りに縮められます。検出は厳格で、インデントされた行、コード、YAML、JSONが一致することはありません。
3. **構造の要約**：エンジンには、コードファイルをインポート、関数のシグネチャ、コールグラフにまで縮約するAPIがあります。フックとMCPの経路ではこれを使わず、ソースを全文渡します。
4. **重複排除キャッシュ**：SHA-256によるコンテンツハッシュで、セッションをまたいで保持されます。モデルがすでに持っている、バイト単位で同一の出力は、13トークンの参照になります。モデルがすでに全体を持っているファイルの行範囲（`sed -n '40,80p'`や、`offset`/`limit`を指定した`sqz_read_file`）は`§ref:HASH:L40-80§`になります。小さな編集の後の再読み込みは、sqzのMCPツールを通した場合、変更された行と、その前後それぞれ一行の変更のない行になります（シェルフックでは編集後のファイルが再び送られます）。ファイル全体に散らばった編集の場合は、全文が返されます。参照が使われるのは、元の内容がまだコンテキストにある間だけです（有効期間は30分で、`SQZ_REF_TTL_SECS`で調整でき、コンパクションでリセットされます）。
5. **JSONパイプライン**：nullの除去 → 管理用フィールド（`__v`、`etag`、リクエストIDと相関ID）の削除 → 平坦化 → 配列の折りたたみ → TOONエンコード（可逆でコンパクトな形式）。値が空の文字列、配列、オブジェクトであるフィールドはどの深さでも削除され、500文字を超える文字列は先頭の500文字に切り詰められて、その後に`...`が付きます。
6. **セーフモード**：スタックトレース（トレースのヘッダーとそれに続くフレーム）、行頭のSQL DDL、PEMブロック、認証情報の行（秘密情報らしいキーを持つ`key=value`または`key: value`）は、ANSIカラーの有無にかかわらずパイプラインを経由せず、サイズに関係なく変更されずに渡されます。取り除かれるのはANSIコードだけです。また、これらはキャッシュされることもありません。シェルフックでは、それでもコマンド別フォーマッターが先に実行されます。その要約が使われるのは、それらのいずれかを含む各段落のすべての行を変更せずに残している場合だけです。そのため、トレースの周りにあるテスト実行の残りの部分が要約されることがあります。そうでない場合は、出力全体がそのまま渡されます。これらの構造の外でパスワード、キー、トレース、マイグレーションに触れているだけの出力は、通常どおり圧縮されますが、やはりキャッシュはされません。そのため、非可逆なステージがそこから落としたものは展開できません。

ステージがパニックした場合、`sqz compress`は入力を変更せずに出力し、stderrに一行を出して、コマンドの終了ステータスを維持します。また、Claude CodeのPostToolUseフックは結果を元のまま残します。

各ステージが何を落とし、何に決して手を触れないかを含む品質の測定結果は、[品質ベンチマーク](../docs/quality-benchmark.md)にあります。技術的な詳細は[docs/](../docs/README.md)をご覧ください。

## 設定

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

### プロジェクトごとのデータベース

sqzが保存するもの（統計、重複排除キャッシュ、セッション）は、すべて一つのSQLiteファイルにあります。デフォルトでは`~/.sqz/sessions.db`です。代わりにプロジェクトごとに分けたい場合は、`SQZ_DB_PATH`を設定します：

```sh
# e.g. in the project's .envrc (direnv)
export SQZ_DB_PATH="$PWD/.sqz/sessions.db"
```

シェルフック、`sqz stats`/`gain`/`expand`、MCPサーバー（MCPの設定の`env`の下に設定します）のすべてがこの設定に従います。絶対パスの使用をおすすめします。相対パスは、プロセスが実行されたディレクトリを基準に解決されます。親ディレクトリがなければ作成されます（`~/.sqz`と同じくモード0700）。

## プライバシー

- テレメトリーなし：データの送信もクラッシュレポートもありません
- 完全にオフライン：エアギャップ環境でも動作します
- すべての処理はローカルで行われます

## 開発

```sh
git clone https://github.com/ojuschugh1/sqz.git
cd sqz
cargo test --workspace
cargo build --release
```

## ライセンス

sqzのライセンスは[Apache License, Version 2.0](../LICENSE-APACHE)または[MIT License](../LICENSE-MIT)で、どちらかを選んで利用できます。

Copyright 2026 Ojus Chugh

2.0.0以前のリリースは[Elastic License 2.0](https://github.com/ojuschugh1/sqz/blob/v2.0.0/LICENSE)で公開されました。

コントリビューションは[コントリビューターライセンス契約（CLA）](../CLA.md)に基づいて受け付けています。

## リンク

- [ドキュメントの目次](../docs/README.md)
- [AIコーディングエージェントに同じファイルを読み直させない方法](../docs/stop-rereading-files.md)
- [MCPのコンテキスト圧縮](../docs/mcp-context-compression.md)
- [品質ベンチマーク：圧縮率と情報の保持の比較](../docs/quality-benchmark.md)
- [コンテキストの劣化とコンテキスト圧縮の比較](../docs/context-rot.md)
- [コンテキスト圧縮の手法の選び方](../docs/comparison.md)
- [ホワイトペーパー：注入前のコンテキスト圧縮](../docs/whitepaper.md)
- [トークン節約のベンチマーク](../docs/benchmark.md)
- [Discord](https://discord.gg/j8EEyH5dSB)
- [変更履歴](../CHANGELOG.md)
- MCP Registryでの名前：`mcp-name: io.github.ojuschugh1/sqz`

## スターの推移

今日sqzでトークンを節約できたなら、スターを付けていただけると、他の人がsqzを見つけやすくなります。自分の数値を見せたい場合は、`sqz stats --share`で表示できます。

<a href="https://star-history.dera.page/#ojuschugh1/sqz&Date">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://star-history.dera.page/svg?repos=ojuschugh1/sqz&type=Date&theme=dark" />
   <source media="(prefers-color-scheme: light)" srcset="https://star-history.dera.page/svg?repos=ojuschugh1/sqz&type=Date" />
   <img alt="スターの推移グラフ" src="https://star-history.dera.page/svg?repos=ojuschugh1/sqz&type=Date" width="600" />
 </picture>
</a>

## コントリビューター

sqzにコード、修正、アイデアを提供してくださったすべての方に感謝します：

<a href="https://github.com/ojuschugh1/sqz/graphs/contributors">
  <img src="https://contrib.rocks/image?repo=ojuschugh1/sqz" alt="sqzのコントリビューター" />
</a>

そして、修正のきっかけとなった詳細なバグ報告を寄せてくださったすべての方にも感謝します。正確な再現手順のおかげで、このプロジェクトはリリースのたびに良くなっています。仲間に加わりませんか。プルリクエストはすぐにレビューされます。まずは[未解決のIssue](https://github.com/ojuschugh1/sqz/issues)をご覧ください。

<sub>コントリビューターの一覧画像は[contrib.rocks](https://contrib.rocks)で作成しています。</sub>
