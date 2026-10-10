<!-- i18n-source: README.md sha256:e20e0153585ab14d62a29797338f75e9415f19639ba4f71317cabc907c46f4f6 -->
<!-- i18n:languages -->
[English](../README.md) | [한국어](README.ko.md)
<!-- /i18n:languages -->

<!-- i18n:note -->
> 이 문서는 영어 원문 [README.md](../README.md)를 번역한 것입니다.
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
  <strong>AI 코딩 에이전트를 위한 주입 전 컨텍스트 압축과 세션 중복 제거</strong>
</p>

<p align="center">
  <sub>
    <strong>실제 세션 통계:</strong>
    압축 3,003회 ·
    <strong>178,442 토큰 절약</strong> ·
    평균 24.7% 감소 ·
    모델이 이미 가진 출력에서는 최대 <strong>92%</strong>
  </sub>
</p>

<p align="center">
  <a href="https://thenextgentechinsider.com/pulse/sqz-tool-cuts-llm-token-use-by-92-for-file-heavy-ai-tasks"><img src="https://img.shields.io/badge/%231_Featured-NextGen_Tech_Insider-ff6600?style=for-the-badge&logo=newspaper&logoColor=white" alt="소개됨"></a>
  <a href="https://hysenlabs.com/en/projects/ojuschugh1-sqz"><img src="https://img.shields.io/badge/Featured-Hysen_Labs-1f6feb?style=for-the-badge&logo=readme&logoColor=white" alt="Hysen Labs에 소개됨"></a>
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
  <a href="https://ojuschugh1.github.io/sqz/">문서</a> ·
  <a href="#install">설치</a> ·
  <a href="#how-it-works">작동 방식</a> ·
  <a href="#supported-tools">지원 도구</a> ·
  <a href="#compress-any-mcp-server">MCP 프록시</a> ·
  <a href="../docs/quality-benchmark.md">벤치마크</a> ·
  <a href="../CHANGELOG.md">변경 내역</a> ·
  <a href="https://discord.gg/j8EEyH5dSB">Discord</a>
</p>

---

AI 코딩 에이전트는 입력 토큰의 대부분을 도구 출력에 씁니다. 빌드 로그, 테스트 실행 결과, `git status`, 그리고 편집할 때마다 다시 읽는 같은 파일이 여기에 해당합니다. 이 모든 내용이 가공되지 않은 채 컨텍스트 윈도우에 들어가고, 그 뒤로 매 턴마다 다시 전송됩니다.

sqz는 도구 출력이 컨텍스트에 들어가기 전에 압축합니다. 명령별 포매터는 에이전트가 행동의 근거로 삼는 정보(실패한 테스트, 어설션, `file:line`)는 남기고 그렇지 않은 정보는 버립니다. 모델이 이미 컨텍스트에 가지고 있는 내용은 본문 대신 짧은 참조로 돌아옵니다. 소스 코드는 변경 없이 그대로 통과하며, 스택 트레이스, 마이그레이션, 자격 증명 줄도 마찬가지입니다.

```sh
cargo install sqz-cli sqz-mcp     # or: brew install ojuschugh1/sqz/sqz · npm i -g sqz-cli · pipx install sqz
sqz init                          # hooks + MCP server + agent guidance for every client it finds
```

<p align="center">
  <img src="../assets/demo.gif" alt="sqz: 200줄짜리 파일은 전체가 제공되고, 41-80번째 줄을 다시 읽으면 줄 범위 참조가 되며, cargo test 출력은 실패 부분만 남도록 줄어들고, sqz stats는 그 두 가지를 모두 보여 줍니다" width="800">
</p>

<sub>릴리스 바이너리의 실제 출력입니다(<code>assets/demo.tape</code>). <code>vhs assets/demo.tape</code>로 다시 생성할 수 있습니다.</sub>

```
Without sqz:                              With sqz:

cat auth.py (200 lines):   1,300 tokens    1,300 tokens  (source is served in full)
cargo test (1 failure):      366 tokens      185 tokens  (failure, assertion, file:line)
sed -n '41,80p' auth.py:     260 tokens       35 tokens  (§ref:…:L41-80§ to the read above)
git status, unchanged:       166 tokens       37 tokens  (§ref:…§)
─────────────────────────────────────      ─────────────
Total:                     2,092 tokens    1,557 tokens  (26% saved, nothing dropped)
```

단일 Rust 바이너리로, 결정적으로 동작하고 LLM을 호출하지 않으며 오프라인에서 작동합니다. `sqz expand`를 사용하면 모든 압축 결과를 바이트 단위로 동일하게 복원할 수 있습니다. 단, 자격 증명, 스택 트레이스 또는 마이그레이션을 언급하는 출력은 예외입니다. sqz는 이런 출력을 절대 캐시하지 않습니다.

> [!NOTE]
> **이름 구분:** 이 저장소 [ojuschugh1/sqz](https://github.com/ojuschugh1/sqz)는 독립 프로젝트이며, 이름이나 동작 방식이 비슷한 다른 도구나 "squeeze"를 줄여 이름을 지은 다른 압축 프로젝트와는 관련이 없습니다. crates.io, npm, PyPI 또는 Homebrew에서 `sqz` / `sqz-cli` / `sqz-mcp`를 설치했다면 그것은 이 저장소에서 나온 것입니다.

## 토큰 절약

> 실제 압축 3,003회에서 평균 **24.7%** 감소 ·
> 모델이 이미 가진 출력에는 **13토큰** 참조 ·
> [품질 벤치마크](../docs/quality-benchmark.md)에서 에이전트에 중요한 사실을 **100%** 보존 ·
> 소스 코드, 그리고 스택 트레이스와 자격 증명 줄에서는 **0%**

다음은 실제 `sqz gain` 출력으로 측정한 한 개발자의 일주일입니다.

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

### 명령별 압축

단일 명령의 압축 결과입니다(`cargo test -p sqz-engine benchmarks`로 측정).

| 내용 | 압축 전 | 압축 후 | 절약 |
|---|---:|---:|---:|
| 반복되는 로그 줄 | 148 | 39 | **74%** |
| 큰 JSON 배열 | 259 | 111 | **57%** |
| JSON API 응답 | 64 | 53 | **17%** |
| Git diff | 61 | 54 | **11%** |
| 일반 문장/문서 | 124 | 122 | **2%** |
| 스택 트레이스(안전 모드) | 82 | 82 | **0%** |

### 세션 단위(중복 제거 사용 시)

모델이 이미 컨텍스트에 가지고 있는 내용은 참조로 돌아옵니다. 릴리스 바이너리와 일회용 데이터베이스를 사용해 `sqz/tests/quality_bench.rs`와 `demo/`의 픽스처로 측정했으며, 수치는 에이전트가 보는 모든 내용(참조와, sqz가 stderr에 남기는 한 줄짜리 안내)의 cl100k 토큰 수입니다.

| 반복 내용 | sqz 미사용 | sqz 사용 | 절약 |
|---|---:|---:|---:|
| 변경되지 않은 `git status`를 다시 실행 | 166 | 37 | **78%** |
| 앞서 읽은 200줄 파일의 41-80번째 줄 | 260 | 35 | **87%** |
| 아무것도 바뀌지 않은 같은 `cargo test` 출력 | 366 | 39 | **89%** |

실제로 무엇이 반복되는지 짚어 보겠습니다. 자신의 Claude Code 세션 92개를 직접 세어 본 한 사용자([스크립트](https://gist.github.com/harrison001/76250b61ad665bf9a377e7096dffdbe0))에 따르면, 542번의 읽기 가운데 파일 전체를 똑같이 다시 읽은 경우는 1번, 줄 범위를 다시 읽은 경우는 8.4%였습니다. 그래서 sqz는 "같은 파일을 다섯 번 읽는" 경우를 앞세우지 않습니다. 실제로 일어나는 반복은 변경되지 않은 명령 출력, 이미 읽은 파일의 줄 범위, 작은 편집 후 다시 읽는 파일이며, 각각 전용 참조 유형이 있습니다([자세히](../docs/stop-rereading-files.md)). 명령 출력이 장황하고 같은 명령을 반복하는 세션에서 효과가 가장 큽니다.

## 설치 <a id="install"></a>

**사전 빌드된 바이너리**(컴파일러가 필요 없으며 모든 플랫폼에서 작동합니다):

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

**Cargo로 소스에서 빌드:**

```sh
cargo install sqz-cli sqz-mcp
```

`sqz-cli`는 `sqz` 바이너리를, `sqz-mcp`는 MCP 서버를 제공합니다. `sqz-engine`은 라이브러리 의존성으로, 자동으로 컴파일되므로 따로 설치할 필요가 없습니다.

**소스에서 빌드**하는 방법(`cargo install sqz-cli`)도 사용할 수 있지만, C 툴체인이 필요합니다.

- Linux: `build-essential`(apt) 또는 그에 상응하는 패키지
- macOS: Xcode Command Line Tools(`xcode-select --install`)
- **Windows: "C++를 사용한 데스크톱 개발"(Desktop development with C++) 워크로드가 포함된 Visual Studio Build Tools.** 이 도구가 없으면 `cargo install`이 `linker link.exe not found` 오류를 내며 실패합니다. 아직 설치하지 않았다면 대신 위의 PowerShell 또는 npm 설치 방법을 사용하십시오.

그런 다음 초기화합니다.

```sh
sqz init --global     # hooks apply to every project on this machine
# or
sqz init              # hooks apply to just this project (.claude/settings.local.json)
```

`--global`은 `~/.claude/settings.json`([Anthropic 범위 표](https://docs.claude.com/en/docs/claude-code/settings)에서 말하는 사용자 범위)에 기록하므로, 이 컴퓨터의 모든 Claude Code 세션에서 sqz 훅이 실행됩니다. 처음 설치할 때는 보통 이 방법을 사용합니다. `~/.claude/settings.json`에 이미 있는 `permissions`, `env`, `statusLine`, 그리고 sqz와 관계없는 훅은 그대로 유지됩니다. sqz는 덮어쓰지 않고 자신의 항목을 병합합니다. 이 항목은 sqz의 훅과 두 가지 허용 규칙인 `Bash(set -o pipefail)`, `Bash(sqz compress --cmd *)`이며, 이 두 규칙은 sqz의 재작성이 덧붙이는 부분에만 적용됩니다. sqz는 명령을 승인하지 않으며, 결정은 여전히 사용자 자신의 권한 규칙이 내립니다. Claude Code가 재작성된 명령을 실행하기 전에 확인을 요청하거나 실행을 거부하면, 그 메시지에는 재작성된 명령이 인용됩니다. 예를 들어 `git tag v0`의 경우 `set -o pipefail; git tag v0 2>&1`이 표시됩니다.

옵션 없이 실행하는 `sqz init`(프로젝트 범위)은 sqz를 한 저장소 안에서만 활성화하고 싶을 때 유용합니다. `~/.claude/settings.json`에 이미 sqz 훅이 있으면, 그 훅은 이미 모든 프로젝트에서 실행되므로 프로젝트 파일에는 넣지 않습니다.

설치한 뒤에는 `sqz doctor`가 전체 연결 과정을 점검합니다. 바이너리, 데이터베이스, 셸 훅, 감지된 클라이언트 중 실제로 sqz를 거치는 클라이언트가 무엇인지, 최근에 압축된 것이 있는지를 확인하고, 발견한 문제마다 해결 방법을 출력합니다. `sqz doctor --fix`는 그 해결 방법을 적용합니다. sqz를 업그레이드한 뒤에는 이 명령을 한 번 실행하십시오. 이 명령은 이전 버전이 작성한 훅과 지침을 최신 상태로 갱신하고, 현재 sqz가 더 이상 사용하지 않는 훅 파일과 `~/.claude/settings.json`에 이미 있는 훅의 프로젝트 사본을 삭제하며, sqz 바이너리가 사라진 훅이 현재 실행 중인 바이너리를 가리키도록 바꾸고, 여전히 다른 sqz 바이너리를 실행하는 훅이 있으면 알려 줍니다. `sqz uninstall`은 sqz 자신의 항목만 제거하고, 파일에 다른 내용이 남아 있지 않을 때만 그 파일을 삭제하며, 마지막에 남겨 둔 파일 목록을 보여 줍니다. 프롬프트에 응답할 터미널이 없으면 `sqz reset`과 마찬가지로 `--yes`가 필요합니다.

### 훅을 설치하기 전에 sqz가 절약했을 양 확인하기

이미 Claude Code나 Kiro를 사용하고 있다면 대화 기록이 디스크에 있습니다. 이 기록을 sqz 엔진으로 재생하면 자신의 세션에 대한 반사실적 추정치를 얻을 수 있으며, 계산은 로컬에서 이루어지고 아무것도 업로드되지 않습니다. 다음은 작성자가 지난 일주일 동안 Kiro IDE에서 작업한 결과로, 요약 부분만 남긴 것입니다.

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

각 출력은 해당 클라이언트에서 sqz가 실제로 그 출력에 도달할 수 있었던 방식대로 재생됩니다. Kiro에서는 sqz가 도구 호출을 재작성할 수 없으므로 첫 번째 합계에 잡히는 토큰이 없습니다. Claude Code의 셸 행은 훅이 스스로 재작성하는 명령과 훅이 건너뛰는 명령으로 나뉩니다. 두 번째 합계는 에이전트가 셸 출력을 `sqz compress`로 파이프하고, sqz MCP 도구로 파일을 읽으며(무손실이므로 반복만 계산됩니다), MCP 서버를 `sqz-mcp proxy` 뒤에서 실행한다고 가정합니다. 보고서는 그중 프록시가 첫 처리에서 긴 문자열과 배열을 잘라 내서 줄인 양이 얼마인지도 알려 줍니다. 웹 가져오기, 편집, 하위 에이전트 결과는 모든 클라이언트에서 범위 밖(out of reach)이며, 재생하지 않고 집계만 합니다.

`~/.claude/projects`와 `~/.kiro/sessions`(Kiro CLI 및 IDE)를 스캔하며, `--transcripts PATH`로 원하는 위치를 지정할 수도 있습니다. 각 대화 기록은 새로 만든 일회용 캐시에서 재생되므로 중복 제거 참조가 세션을 넘나드는 척하는 일은 없습니다. 보고서에 "추정치(estimate)"라고 나오는 것은 실제로 추정치이기 때문입니다. 이 출력들은 당시에는 압축되지 않았습니다.

**에이전트를 하나만 사용합니까?** `--only`(또는 `--skip`)를 지정해 기록할 설정을 제한할 수 있습니다.

```sh
sqz init --only opencode              # just OpenCode, nothing else
sqz init --only opencode,codex        # OpenCode and Codex
sqz init --skip cursor,windsurf       # everything except Cursor and Windsurf
```

사용할 수 있는 이름은 `claude`, `cursor`, `windsurf`, `cline`, `gemini`, `kiro`, `opencode`, `codex`입니다. 별칭(`claude-code`, `gemini-cli`, `roo`, `kiro-cli`)도 사용할 수 있습니다. `--only`와 `--skip`은 함께 사용할 수 없습니다.

### 수동 설치(설정 파일의 주석 유지)

`sqz init`은 sqz 항목을 병합하기 위해 설정 파일을 JSON 파서로 읽었다가 다시 쓰므로, `opencode.jsonc`(그리고 다른 도구가 허용하는 비슷한 주석 포함 JSON 파일)에 있는 주석이 모두 사라집니다. 설정 파일에 주석을 꼼꼼히 달아 두었고 이를 유지하고 싶다면 대신 직접 설치하십시오.

**OpenCode**는 두 단계로 설치합니다.

1. 플러그인 파일을 정해진 위치에 둡니다. `sqz`가 생성한 TS를 stdout으로 출력하므로 경로 이스케이프 로직을 직접 작성할 필요가 없습니다.

   ```sh
   mkdir -p ~/.config/opencode/plugins
   sqz print-opencode-plugin > ~/.config/opencode/plugins/sqz.ts
   ```

2. 기존 `opencode.jsonc`에 MCP 항목을 직접 추가합니다. 최상위 `mcp` 객체 안에 이 블록을 덧붙이십시오(`mcp` 객체가 없다면 새로 만드십시오).

   ```jsonc
   "sqz": {
     "type": "local",
     "command": ["sqz-mcp", "--transport", "stdio"],
     "enabled": true
   }
   ```

파일의 나머지 부분에 있는 주석은 그대로 남습니다. OpenCode는 플러그인 파일을 자동으로 찾으므로 `plugin` 배열에 항목을 넣을 필요가 없습니다(넣으면 두 번 로드됩니다. 이슈 #10을 참고하십시오).

**다른 도구**: Claude Code와 Gemini CLI는 주석을 지원하지 않는 일반 JSON 설정을 사용하고, Cursor, Windsurf, Cline에는 마크다운 규칙 파일이 생성되며, Codex에는 `~/.codex/config.toml`에 주석을 보존하는 TOML 항목이 추가됩니다. 따라서 이 도구들에서는 자동 방식이 기존 내용을 망가뜨리지 않습니다. 이 도구들에는 `sqz init --only <tool>`을 사용하십시오. 기존 `.gemini/settings.json`에는 sqz의 훅이 병합됩니다. 이 파일에 주석이 있거나 파싱되지 않으면 `sqz init`은 파일을 건드리지 않고 직접 추가할 항목을 출력합니다.

그런 다음 `sqz doctor`를 실행해 각 클라이언트에 무엇이 설정되었는지, 어떤 통합 등급에 해당하는지 확인하십시오.

## 작동 방식 <a id="how-it-works"></a>

<p align="center">
  <img src="../assets/sqz-architecture.png" alt="sqz 시스템 아키텍처" width="700" />
</p>

투명형(transparent) 클라이언트인 Claude Code, Gemini CLI, Copilot CLI, OpenCode에서는 sqz가 셸 명령을 재작성하는 훅을 설치해, 명령 출력이 AI 도구에 도달하기 전에 sqz를 거치도록 합니다. 권고형(advisory) 클라이언트에서는 에이전트에게 출력을 `sqz compress`로 파이프하거나 sqz MCP 도구를 사용하도록 안내합니다. 각 클라이언트의 등급은 아래 표에 나와 있습니다.

```
Claude → git status → [sqz hook rewrites] → compressed output
```

압축되는 것:
- **셸 출력**: 명령별 포매터(git, cargo, npm/pnpm/yarn/bun, pytest, ruff, mypy, go, docker ps/images, kubectl describe/apply, terraform, gradle, maven, xcodebuild, adb logcat, dotnet, grep/rg, tree, curl 등)
- **JSON**: null과 빈 값을 제거하고, 500자를 넘는 문자열을 자르며, 간결하게 인코딩하고, TOON 형식을 사용합니다
- **로그**: 반복되는 줄을 축약합니다
- **테스트 출력**: 실패만 보여 줍니다(Rust, Go, Python, JS, JVM용 상태 기계 파서)

압축되지 않는 것:
- 소스 코드: 처음 읽을 때 전체를 제공합니다
- 스택 트레이스, SQL 마이그레이션, PEM 블록, 자격 증명 줄: 안전 모드로 보내져 크기와 관계없이 변경 없이 그대로 전달되며, 절대 캐시되지 않습니다. 단, 셸 훅을 거치는 경우에는 명령별 포매터가 이들을 둘러싼 테스트 실행의 나머지 부분을 요약할 수 있습니다
- 사용자의 프롬프트와 AI의 응답: sqz가 아니라 AI 도구가 제어합니다

## 지원 도구 <a id="supported-tools"></a>

| 도구 | 통합 방식 | 설정 |
|---|---|---|
| Claude Code | PreToolUse + PostToolUse 훅 + MCP 서버, 투명형(transparent) | `sqz init` |
| Cursor | `.cursor/rules/sqz.mdc` 지침, 권고형(advisory); MCP 서버는 직접 추가 | `sqz init` |
| Windsurf | `.windsurf/rules/sqz.md` 지침, 권고형(advisory); MCP 서버는 직접 추가 | `sqz init` |
| Cline | `.clinerules` 지침, 권고형(advisory); MCP 서버는 직접 추가 | `sqz init` |
| Gemini CLI | BeforeTool 훅, 투명형(transparent) | `sqz init` |
| Kiro | Steering + MCP 서버, 권고형(advisory) | `sqz init` |
| OpenCode | TypeScript 플러그인 + MCP 서버, 투명형(transparent) | `sqz init` |
| Codex CLI | AGENTS.md 지침 + MCP 서버, 권고형(advisory) | `sqz init` |
| Zed | AGENTS.md 지침 + MCP 서버, 권고형(advisory) | `sqz init` |
| Copilot CLI | preToolUse 훅, 투명형(transparent), `~/.copilot`이 있는 경우 | `sqz init` |
| Copilot coding agent(CI) | 저장소 수준 훅, 클라우드 샌드박스에서 스스로 부트스트랩 | `sqz init --ci` + 커밋 |
| 모든 CI 파이프라인 | [setup-sqz 액션](https://github.com/ojuschugh1/setup-sqz): LLM 단계 전에 로그 압축 | `uses: ojuschugh1/setup-sqz@v1` |
| 모든 MCP 서버 | `sqz-mcp proxy`로 감싸서 해당 서버의 도구 결과를 압축 | 아래 참고 |
| VS Code | [확장 프로그램](https://marketplace.visualstudio.com/items?itemName=ojuschugh1.sqz) | Marketplace에서 설치 |
| JetBrains | [플러그인](https://plugins.jetbrains.com/plugin/31240-sqz--context-intelligence/) | Marketplace에서 설치 |
| Chrome | 브라우저 확장 프로그램 | ChatGPT, Claude.ai, Gemini, Grok, Perplexity |
| [Firefox](https://addons.mozilla.org/en-US/firefox/addon/sqz-context-compression/) | 브라우저 확장 프로그램 | 같은 사이트 |

투명형(transparent) 클라이언트에는 셸 명령을 스스로 sqz로 보내는 훅이 있습니다. 권고형(advisory) 클라이언트에서는 에이전트가 출력을 `sqz compress`로 파이프하거나 sqz MCP 도구를 호출할 때만 sqz가 토큰을 절약합니다. 클라이언트에 내장된 셸 도구와 파일 도구는 sqz를 거치지 않습니다. `sqz doctor`는 각 클라이언트의 등급을 보여 줍니다.

## 어떤 MCP 서버든 압축하기 <a id="compress-any-mcp-server"></a>

`sqz-mcp proxy`는 에이전트와 임의의 stdio MCP 서버 사이에 위치해 돌아오는 내용을 압축합니다. 도구 결과는 sqz의 전체 파이프라인을 거치고(반복되는 내용에는 중복 제거 참조, 스택 트레이스와 비밀 정보에는 안전 모드를 적용하고, 오류 결과는 손대지 않습니다), 장황한 도구 설명은 간결하게 줄어들어 `tools/list`가 더 이상 컨텍스트 윈도우를 잡아먹지 않습니다. 프록시는 `sqz_expand` 도구를 주입하므로 에이전트는 캐시된 원본이라면 무엇이든 바이트 단위로 동일하게 복원할 수 있습니다. 자격 증명, 스택 트레이스 또는 마이그레이션을 언급하는 결과는 절대 캐시되지 않습니다.

MCP 설정에서 서버의 명령 앞에 접두어를 붙여 서버를 감쌉니다.

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

플래그: `--lazy-tools`는 모든 도구 설명을 한 문장으로 줄이고, 필요할 때 원래의 전체 문서를 제공하는 `sqz_tool_help` 도구를 주입합니다(일부 MCP 서버는 `tools/list` 하나에만 10-40k 토큰을 씁니다). `--no-desc`는 설명을 원문 그대로 유지합니다. `--no-cache`는 아무것도 저장하지 않으므로 중복 제거 참조를 보내지 않고 `sqz_expand` 도구도 추가하지 않습니다. 클라이언트에는 평범한 MCP 서버로 보일 뿐이므로 모든 MCP 클라이언트(Claude Code, Cursor, Windsurf, Zed, Codex, Kiro 등)에서 작동합니다. 서버 자체의 `sqz_read_file` / `sqz_grep` / `sqz_list_dir` 도구와 클라이언트별 설정을 포함한 전체 가이드는 [MCP 컨텍스트 압축](../docs/mcp-context-compression.md)에 있습니다.

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

### 중복 제거 우회 방법

sqz는 같은 내용을 두 번째로 보면 전체 텍스트 대신 짧은 `§ref:HASH§` 참조 마커를 반환합니다. 대부분의 모델은 이를 문제없이 처리하지만, 일부 모델(예: GLM 5.1)은 참조 형식을 해석하지 못해 루프에 빠집니다. 이 문제를 우회하는 방법은 네 가지입니다.

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

### Recall: sqz가 본 모든 내용 검색하기

sqz를 거친 모든 내용은 로컬에서 색인됩니다(SQLite FTS5를 사용하며, 사용자의 컴퓨터에 있고 아무것도 밖으로 나가지 않습니다). 에이전트가 컨텍스트를 컴팩션하는 과정에서 한 시간 전의 오류 메시지를 잃어버렸다면, 명령을 다시 실행하는 대신 그 메시지를 검색하십시오.

```sh
$ sqz recall "connection refused"
  1. [git · 2h ago]  … curl: (7) Failed to connect: >>connection refused<< on port 5432 …
     full content: sqz expand 3f9a1c05e2d84b17
```

에이전트는 같은 기능을 `sqz_recall` MCP 도구로 사용할 수 있습니다.

## 내 절약량 추적하기

셸에서 언제든 `sqz gain`을 실행하면 자신의 일별 내역을 볼 수 있고(출력 모양은 위의 "토큰 절약" 섹션을 참고하십시오), `sqz stats`를 실행하면 전체 누적 보고서를 볼 수 있습니다.

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

`--breakdown`을 추가하면 어떤 명령이 토큰을 가장 많이 소비하는지 정확히 볼 수 있습니다.

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

`sqz stats`와 `sqz gain`은 sqz가 실제로 출력한 내용의 cl100k 토큰을 세며, 약어 범례와 확장 힌트도 포함합니다. 긴 구분선처럼 한 종류의 문자가 2 KB를 넘게 이어지는 구간은 2 KB 단위로 나누어 세므로, 약 1%에 4토큰을 더한 만큼 오차가 생길 수 있습니다.

### 정직한 집계: 비용과 후회

토큰 감소가 자동으로 청구 비용 감소로 이어지지는 않습니다. 제공업체의 프롬프트 캐싱이 있으면 컨텍스트의 대부분은 약 90% 할인된 가격으로 재전송되고, 에이전트에게 필요한 내용을 버리는 압축은 오히려 턴을 더 쓰게 만듭니다([arXiv:2607.12161](https://arxiv.org/abs/2607.12161)). sqz는 얼버무리지 않고 양쪽을 모두 측정합니다.

- `sqz stats --cost`는 프롬프트 캐싱을 적용한 과금 모델(캐시 쓰기는 한 번 ×1.25, 캐시 읽기는 이후 턴마다 ×0.10)에서 절약한 금액을 달러로 추정합니다. 모든 가정은 출력되며 덮어쓸 수 있습니다(`--price-in`, `--reread-turns`, `--cache-write-mult`, `--cache-read-mult`). 이것은 보수적인 추정치입니다. 캐싱이 없다면 같은 절약분은 매 턴 전체 입력 가격만큼의 가치가 됩니다.
- `sqz stats`는 후회 신호(regret signals)를 보고합니다. 하나는 빠른 재실행(quick re-runs)으로, 에이전트가 2분 안에 바이트 단위로 동일한 출력을 다시 만들어 낸 경우이며 그 반복으로 얻은 것은 없습니다(Claude Code의 PostToolUse 훅은 참조로 응답하는 일이 없으므로 이 훅의 반복은 일반 압축으로 집계됩니다). 다른 하나는 참조 확장(ref expands)으로, `§ref§` 참조 마커가 원래 바이트로 복원된 경우입니다. 둘 다 "압축이 모델에게 필요한 내용을 버렸다"는 것을 나타내는 대리 지표입니다. 어떤 명령이 계속 나타난다면 그 포매터를 개선해야 하므로 이슈를 등록하십시오.

sqz의 설계는 그 연구가 측정한 실패 유형을 이미 피하고 있습니다. 압축은 결정적이고 쿼리와 무관하며(제공업체의 프롬프트 캐시를 무효화하는 일이 없습니다), 새로운 도구 출력만 다룹니다(이전 기록을 다시 쓰는 일은 없습니다). 출력을 계속 따라가는 명령, 서버를 실행하는 명령, 작업을 백그라운드로 보내는 명령은 그대로 둡니다(bypass 모드가 아닌 Claude Code에서는 PreToolUse 훅이 파이프라인, 루프, heredoc을 작성된 그대로 두고, 실행이 성공한 뒤 PostToolUse 훅이 그 출력을 압축합니다). 그룹으로 묶인 목록과 파이프라인(Gemini CLI, Copilot CLI, bypass 모드의 Claude Code)에서는 명령별 포매터를 건너뛰며, 16토큰 순이득 기준은 자체 마커 비용조차 회수하지 못할 압축을 건너뜁니다. 재작성하면 원래와 다른 조합의 명령이 실행될 수 있는 경우에도 명령을 작성된 그대로 둡니다. `ls ${DEST:?}; echo next`에서 `ls`만 따로 파이프한다면, 확장이 실패해 셸이 멈춘 뒤에도 `echo next`가 실행될 것입니다. 또한 목록이 하나의 그룹으로 sqz를 거치는 경우, `cd build && make; make test`에서 `cd`가 실패하면 `make`만 건너뛰는 대신 그룹 전체를 건너뛰게 될 것입니다.
<img width="3456" height="1918" alt="이미지" src="https://github.com/user-attachments/assets/d308bc60-81f3-4935-9728-b30534e5b5e5" />


**프로젝트별 필터링:**

```sh
sqz stats --project .           # stats for current project only
sqz stats --project list        # list all tracked projects
sqz gain --project .            # daily gains for current project
sqz gain --days 30              # last 30 days instead of 7
sqz gain --days 30 --project .  # combine both
```

통계는 로컬의 `~/.sqz/sessions.db`에 SQLite 형식으로 저장되며, 아무것도 사용자의 컴퓨터 밖으로 나가지 않습니다.

## 압축 방식

1. 아래 명령에 대한 **명령별 포매터**:

   | 생태계 | 명령 |
   |---|---|
   | Git | status, log, diff, show, stash, remote, fetch, push, pull, add, commit |
   | Rust | cargo build/test/clippy/check/nextest |
   | JavaScript | npm/pnpm/yarn/bun install/audit/outdated(test와 run은 실패할 때), tsc, eslint, biome |
   | Python | pytest, ruff, mypy, pip |
   | Go | go test(`-json` 스트림 포함), go build, go vet, golangci-lint |
   | 클라우드 | terraform plan/apply/destroy/refresh/init |
   | 컨테이너 | docker/podman ps/images, kubectl describe/apply |
   | JVM | gradle build/test, maven |
   | 모바일 | xcodebuild(build + test), adb logcat |
   | .NET | dotnet build/publish/test |
   | 시스템 | grep/rg, tree, curl/wget |

   kubectl get과 logs, gh, aws, gcloud, ls, find를 포함한 그 밖의 명령은 범용 압축 파이프라인을 거칩니다.

2. **표 압축기**: 전용 포매터가 없는 도구(`ps aux`, `netstat`, 데이터베이스 CLI)의 열 정렬 출력에서는 연속된 패딩 공백이 공백 두 칸짜리 구분자로 줄어듭니다. 감지기는 엄격해서 들여쓴 줄, 코드, YAML, JSON은 절대 감지되지 않습니다.
3. **구조 요약**: 엔진에는 코드 파일을 import, 함수 시그니처, 호출 그래프로 줄이는 API가 있습니다. 훅과 MCP 경로에서는 이 API를 사용하지 않고 소스를 전체 그대로 제공합니다.
4. **중복 제거 캐시**: SHA-256 콘텐츠 해시를 사용하며 세션이 바뀌어도 유지됩니다. 모델이 이미 가진 것과 바이트 단위로 동일한 출력 = 13토큰짜리 참조. 모델이 이미 전체를 가진 파일의 줄 범위(`sed -n '40,80p'`, `offset`/`limit`을 지정한 `sqz_read_file`) = `§ref:HASH:L40-80§`. 작은 편집 후 다시 읽기 = sqz MCP 도구를 거치면 변경된 줄과 그 양쪽의 변경되지 않은 줄 한 줄씩(셸 훅은 편집된 파일을 다시 보냅니다). 파일 전체에 흩어진 편집은 전체 내용으로 돌아옵니다. 참조는 원본이 아직 컨텍스트에 있는 동안에만 제공됩니다(유효 시간은 30분이며 `SQZ_REF_TTL_SECS`로 조정할 수 있고, 컴팩션 시 초기화됩니다).
5. **JSON 파이프라인**: null 제거 → 관리용 필드(`__v`, `etag`, 요청 ID와 상관관계 ID) 삭제 → 평탄화 → 배열 축약 → TOON 인코딩(무손실의 간결한 형식). 값이 빈 문자열, 빈 배열 또는 빈 객체인 필드는 깊이와 관계없이 삭제되며, 500자를 넘는 문자열은 앞의 500자만 남기고 그 뒤에 `...`이 붙습니다.
6. **안전 모드**: 스택 트레이스(트레이스 헤더와 그 뒤에 이어지는 프레임), 줄 맨 앞의 SQL DDL, PEM 블록, 자격 증명 줄(비밀 정보로 보이는 키를 가진 `key=value` 또는 `key: value`)은 ANSI 색상이 있든 없든 파이프라인을 건너뛰고, 크기와 관계없이 ANSI 코드만 제거된 채 변경 없이 그대로 전달되며, 절대 캐시되지 않습니다. 셸 훅에서는 그래도 명령별 포매터가 먼저 실행됩니다. 포매터의 요약은 이러한 구조 중 하나를 포함한 각 단락의 모든 줄을 변경 없이 유지할 때만 사용되므로, 포매터가 트레이스 주변에 있는 테스트 실행의 나머지 부분을 요약할 수도 있습니다. 그렇지 않으면 출력 전체가 그대로 전달됩니다. 이러한 구조 밖에서 비밀번호, 키, 트레이스 또는 마이그레이션을 언급하기만 하는 출력은 평소처럼 압축되지만 이 역시 캐시되지 않으므로, 손실 압축 단계가 이 출력에서 버린 내용은 확장할 수 없습니다.

어떤 단계에서 패닉이 발생하면 `sqz compress`는 입력을 변경 없이 그대로 출력하면서 stderr에 한 줄을 남기고 명령의 종료 상태를 유지하며, Claude Code의 PostToolUse 훅은 결과를 원래대로 둡니다.

각 단계가 무엇을 버리고 무엇을 절대 건드리지 않는지를 포함한 품질 측정 결과는 [품질 벤치마크](../docs/quality-benchmark.md)에 있습니다. 전체 기술 세부 사항은 [docs/](../docs/README.md)를 참고하십시오.

## 설정

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

### 프로젝트별 데이터베이스

sqz가 저장하는 모든 것(통계, 중복 제거 캐시, 세션)은 하나의 SQLite 파일에 들어 있으며, 기본 위치는 `~/.sqz/sessions.db`입니다. 대신 프로젝트별로 따로 두려면 `SQZ_DB_PATH`를 설정하십시오.

```sh
# e.g. in the project's .envrc (direnv)
export SQZ_DB_PATH="$PWD/.sqz/sessions.db"
```

모든 진입점이 이 설정을 따릅니다. 셸 훅, `sqz stats`/`gain`/`expand`, MCP 서버(MCP 설정의 `env` 아래에 지정합니다)가 여기에 해당합니다. 절대 경로를 사용하는 것이 좋습니다. 상대 경로 값은 프로세스가 어느 디렉터리에서 실행되든 그 디렉터리를 기준으로 해석됩니다. 상위 디렉터리가 없으면 새로 만들어집니다(`~/.sqz`와 같은 모드 0700).

## 개인정보 보호

- 텔레메트리 없음: 데이터를 전송하지 않으며 충돌 보고서도 보내지 않습니다
- 완전한 오프라인: 에어갭 환경에서도 작동합니다
- 모든 처리는 로컬에서 이루어집니다

## 개발

```sh
git clone https://github.com/ojuschugh1/sqz.git
cd sqz
cargo test --workspace
cargo build --release
```

## 라이선스

[Elastic License 2.0](../LICENSE)(ELv2): 자유롭게 사용, 포크, 수정할 수 있습니다. 제한은 두 가지로, 경쟁하는 호스팅 서비스를 제공할 수 없고 라이선스 고지를 제거할 수 없습니다.

## 링크

- [문서 색인](../docs/README.md)
- [AI 코딩 에이전트가 같은 파일을 다시 읽지 않게 하는 방법](../docs/stop-rereading-files.md)
- [MCP 컨텍스트 압축](../docs/mcp-context-compression.md)
- [품질 벤치마크: 압축률과 정보 보존 비교](../docs/quality-benchmark.md)
- [컨텍스트 부패와 컨텍스트 압축 비교](../docs/context-rot.md)
- [컨텍스트 압축 방식 선택하기](../docs/comparison.md)
- [백서: 주입 전 컨텍스트 압축](../docs/whitepaper.md)
- [토큰 절약 벤치마크](../docs/benchmark.md)
- [Discord](https://discord.gg/j8EEyH5dSB)
- [변경 내역](../CHANGELOG.md)
- MCP Registry 이름: `mcp-name: io.github.ojuschugh1/sqz`

## 스타 추이

오늘 sqz로 토큰을 절약했다면, 스타 하나가 다른 사람들이 sqz를 찾는 데 도움이 됩니다. 자신의 수치를 보여 주고 싶다면 `sqz stats --share`로 출력할 수 있습니다.

<a href="https://star-history.dera.page/#ojuschugh1/sqz&Date">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://star-history.dera.page/svg?repos=ojuschugh1/sqz&type=Date&theme=dark" />
   <source media="(prefers-color-scheme: light)" srcset="https://star-history.dera.page/svg?repos=ojuschugh1/sqz&type=Date" />
   <img alt="스타 추이 차트" src="https://star-history.dera.page/svg?repos=ojuschugh1/sqz&type=Date" width="600" />
 </picture>
</a>

## 기여자

sqz에 코드, 수정, 아이디어를 기여해 주신 모든 분께 감사드립니다.

<a href="https://github.com/ojuschugh1/sqz/graphs/contributors">
  <img src="https://contrib.rocks/image?repo=ojuschugh1/sqz" alt="sqz 기여자" />
</a>

그리고 수정의 바탕이 된 자세한 버그 보고서를 올려 주신 모든 분께도 감사드립니다. 정확한 재현 방법 덕분에 이 프로젝트는 릴리스마다 더 나아집니다. 함께하시겠습니까? PR은 빠르게 리뷰됩니다. [열린 이슈](https://github.com/ojuschugh1/sqz/issues)부터 살펴보십시오.

<sub>기여자 그리드는 [contrib.rocks](https://contrib.rocks)로 만들었습니다.</sub>
