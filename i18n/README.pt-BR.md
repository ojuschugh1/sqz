<!-- i18n-source: README.md sha256:9de81c95b725cb7bc266ef2b2a2cbcd1982f6433ee6ee9c84cfaf284e8b2efc8 -->
<!-- i18n:languages -->
[English](../README.md) | [简体中文](README.zh-CN.md) | [日本語](README.ja.md) | [한국어](README.ko.md) | [Español](README.es.md) | [Português (Brasil)](README.pt-BR.md)
<!-- /i18n:languages -->

<!-- i18n:note -->
> Esta é uma tradução do [README.md em inglês](../README.md).
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
  <strong>Compressão de contexto antes da injeção e deduplicação na sessão para agentes de programação com IA</strong>
</p>

<p align="center">
  <sub>
    <strong>Estatísticas de sessões reais:</strong>
    3,003 compressões ·
    <strong>178,442 tokens economizados</strong> ·
    redução média de 24.7% · até
    <strong>92%</strong> em saídas que o modelo já tem
  </sub>
</p>

<p align="center">
  <a href="https://thenextgentechinsider.com/pulse/sqz-tool-cuts-llm-token-use-by-92-for-file-heavy-ai-tasks"><img src="https://img.shields.io/badge/%231_Featured-NextGen_Tech_Insider-ff6600?style=for-the-badge&logo=newspaper&logoColor=white" alt="Em destaque"></a>
  <a href="https://hysenlabs.com/en/projects/ojuschugh1-sqz"><img src="https://img.shields.io/badge/Featured-Hysen_Labs-1f6feb?style=for-the-badge&logo=readme&logoColor=white" alt="Em destaque no Hysen Labs"></a>
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
  <a href="https://ojuschugh1.github.io/sqz/">Documentação</a> ·
  <a href="#install">Instalação</a> ·
  <a href="#how-it-works">Como funciona</a> ·
  <a href="#supported-tools">Ferramentas compatíveis</a> ·
  <a href="#compress-any-mcp-server">Proxy MCP</a> ·
  <a href="../docs/quality-benchmark.md">Benchmark</a> ·
  <a href="../CHANGELOG.md">Registro de alterações</a> ·
  <a href="https://discord.gg/j8EEyH5dSB">Discord</a>
</p>

---

Agentes de programação com IA gastam a maior parte dos tokens de entrada com a saída das ferramentas: logs de build, execuções de testes, `git status`, o mesmo arquivo lido de novo depois de cada edição. Tudo isso entra na janela de contexto sem nenhum tratamento e é reenviado em todos os turnos seguintes.

O sqz comprime a saída das ferramentas antes que ela entre no contexto. Formatadores por comando mantêm o que o agente usa para agir (o teste que falhou, a asserção, o `file:line`) e descartam o que ele não usa. Tudo o que o modelo já tem no contexto volta como uma referência curta, em vez do conteúdo. O código-fonte passa sem alterações, assim como stack traces, migrações e linhas com credenciais.

```sh
cargo install sqz-cli sqz-mcp     # or: brew install ojuschugh1/sqz/sqz · npm i -g sqz-cli · pipx install sqz
sqz init                          # hooks + MCP server + agent guidance for every client it finds
```

<p align="center">
  <img src="../assets/demo.gif" alt="sqz: um arquivo de 200 linhas é entregue completo, uma releitura das linhas 41-80 vira uma referência a um intervalo de linhas, a saída de cargo test se reduz à falha, e o sqz stats mostra as duas coisas" width="800">
</p>

<sub>Saída real do binário de release (<code>assets/demo.tape</code>). Para gerar de novo, use <code>vhs assets/demo.tape</code>.</sub>

```
Without sqz:                              With sqz:

cat auth.py (200 lines):   1,300 tokens    1,300 tokens  (source is served in full)
cargo test (1 failure):      366 tokens      185 tokens  (failure, assertion, file:line)
sed -n '41,80p' auth.py:     260 tokens       35 tokens  (§ref:…:L41-80§ to the read above)
git status, unchanged:       166 tokens       37 tokens  (§ref:…§)
─────────────────────────────────────      ─────────────
Total:                     2,092 tokens    1,557 tokens  (26% saved, nothing dropped)
```

Um único binário em Rust, determinístico, sem chamadas a LLM e que funciona offline. Todo resultado comprimido pode ser recuperado idêntico byte a byte com `sqz expand`, exceto a saída que menciona credenciais, um stack trace ou uma migração, que o sqz nunca guarda em cache.

> [!NOTE]
> **Desambiguação do nome:** este repositório, [ojuschugh1/sqz](https://github.com/ojuschugh1/sqz), é um projeto independente e não tem relação com nenhuma outra ferramenta de nome ou funcionamento parecido, nem com outros projetos de compressão que abreviam "squeeze". Se você instalou `sqz` / `sqz-cli` / `sqz-mcp` pelo crates.io, npm, PyPI ou Homebrew, ele vem deste repositório.

## Economia de tokens

> **24.7%** de redução média em 3,003 compressões reais ·
> referências de **13 tokens** para saídas que o modelo já tem ·
> **100%** dos fatos críticos para o agente mantidos no [benchmark de qualidade](../docs/quality-benchmark.md) ·
> **0%** no código-fonte, assim como em stack traces e linhas com credenciais

A semana de um desenvolvedor, medida a partir da saída real do `sqz gain`:

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

### Compressão por comando

Compressão de um único comando (medida com `cargo test -p sqz-engine benchmarks`):

| Conteúdo | Antes | Depois | Economia |
|---|---:|---:|---:|
| Linhas de log repetidas | 148 | 39 | **74%** |
| Array JSON grande | 259 | 111 | **57%** |
| Resposta de API em JSON | 64 | 53 | **17%** |
| Diff do Git | 61 | 54 | **11%** |
| Prosa/documentação | 124 | 122 | **2%** |
| Stack trace (modo seguro) | 82 | 82 | **0%** |

### No nível da sessão, com deduplicação

Tudo o que o modelo já tem no contexto volta como uma referência. Medido com o binário de release e um banco de dados descartável, usando as fixtures de `sqz/tests/quality_bench.rs` e `demo/`, em tokens cl100k de tudo o que o agente vê (a referência mais a nota de uma linha que o sqz escreve no stderr):

| Repetição | Sem sqz | Com sqz | Economia |
|---|---:|---:|---:|
| `git status` sem mudanças executado de novo | 166 | 37 | **78%** |
| Linhas 41-80 de um arquivo de 200 linhas lido antes | 260 | 35 | **87%** |
| Mesma saída de `cargo test`, sem nenhuma mudança | 366 | 39 | **89%** |

Uma observação sobre o que realmente se repete. Um usuário que contou 92 das próprias sessões do Claude Code ([script](https://gist.github.com/harrison001/76250b61ad665bf9a377e7096dffdbe0)) encontrou releituras idênticas do arquivo inteiro em 1 de 542 leituras, e releituras de intervalos de linhas em 8.4% delas. Por isso, o destaque do sqz não é "o mesmo arquivo lido cinco vezes": as repetições que acontecem na prática são saídas de comandos sem mudanças, intervalos de linhas de um arquivo já lido e arquivos relidos depois de uma pequena edição, e cada uma tem o próprio tipo de referência ([detalhes](../docs/stop-rereading-files.md)). Sessões com saída de comandos ruidosa e comandos repetidos têm os maiores ganhos.

## Instalação <a id="install"></a>

**Binários pré-compilados** (não precisa de compilador e funciona em todas as plataformas):

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

**Compilar a partir do código-fonte com o Cargo:**

```sh
cargo install sqz-cli sqz-mcp
```

O `sqz-cli` fornece o binário `sqz`; o `sqz-mcp` fornece o servidor MCP. O `sqz-engine` é uma dependência de biblioteca: ele é compilado automaticamente e não precisa ser instalado separadamente.

**Compilar a partir do código-fonte** (`cargo install sqz-cli`) também funciona, mas exige uma toolchain C:

- Linux: `build-essential` (apt) ou equivalente
- macOS: Xcode Command Line Tools (`xcode-select --install`)
- **Windows: Visual Studio Build Tools com a carga de trabalho "Desktop development with C++".** Sem essas ferramentas, o `cargo install` falha com `linker link.exe not found`. Se você ainda não as tem, use a instalação por PowerShell ou npm mostrada acima.

Depois, inicialize:

```sh
sqz init --global     # hooks apply to every project on this machine
# or
sqz init              # hooks apply to just this project (.claude/settings.local.json)
```

O `--global` grava em `~/.claude/settings.json` (o escopo de usuário, segundo a [tabela de escopos da Anthropic](https://docs.claude.com/en/docs/claude-code/settings)), então o hook do sqz é disparado em todas as sessões do Claude Code nesta máquina. Esse é o caso comum na primeira instalação. As configurações `permissions`, `env` e `statusLine` e os hooks não relacionados que você já tem em `~/.claude/settings.json` são preservados: o sqz mescla as próprias entradas em vez de sobrescrever o arquivo. Essas entradas são os hooks dele e duas regras de permissão do tipo allow, `Bash(set -o pipefail)` e `Bash(sqz compress --cmd *)`, que cobrem só as partes que a reescrita dele acrescenta. O sqz não aprova comandos; as suas próprias regras de permissão continuam decidindo. Quando o Claude Code pede confirmação antes de um comando reescrito, ou o recusa, a mensagem cita o comando reescrito, como `set -o pipefail; git tag v0 2>&1` para `git tag v0`.

O `sqz init` sem opções (escopo de projeto) é útil quando você quer o sqz ativo só dentro de um repositório. Se `~/.claude/settings.json` já tem os hooks do sqz, ele os deixa de fora do arquivo do projeto, porque eles já rodam em todos os projetos.

Depois de instalar, o `sqz doctor` verifica toda a cadeia (binário, banco de dados, hook do shell, quais clientes foram detectados e quais realmente passam pelo sqz, e se algo foi comprimido recentemente) e mostra a correção para cada falha que encontrar. O `sqz doctor --fix` aplica essas correções. Depois de atualizar o sqz, rode-o uma vez: ele atualiza os hooks e as instruções que uma versão anterior escreveu, remove os arquivos de hook que o sqz atual não usa mais e uma cópia dos hooks no projeto que `~/.claude/settings.json` já tem, faz os hooks cujo binário do sqz não existe mais apontarem para o binário em execução e avisa sobre qualquer hook que ainda execute outro binário do sqz. O `sqz uninstall` remove só as entradas do próprio sqz, apaga um arquivo só quando não sobra mais nada nele e termina listando os arquivos que manteve. Sem um terminal para responder à pergunta dele, ele precisa de `--yes`, assim como o `sqz reset`.

### Veja o que o sqz teria economizado, antes de instalar qualquer hook

Se você já usa o Claude Code ou o Kiro, as suas transcrições estão no disco. Reproduza-as no motor do sqz e obtenha uma estimativa contrafactual sobre as suas próprias sessões, calculada localmente, sem enviar nada. Da última semana de trabalho do autor no Kiro IDE, reduzida ao resumo:

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

Cada saída é reproduzida do jeito que o sqz realmente poderia tê-la alcançado naquele cliente. O Kiro não deixa o sqz reescrever uma chamada de ferramenta, então lá o primeiro total é zero; no Claude Code, as linhas de shell se dividem entre comandos que o hook reescreve sozinho e comandos que ele pula. O segundo total supõe que o agente passa a saída do shell por `sqz compress`, lê arquivos com as ferramentas MCP do sqz (sem perdas, então só as repetições contam) e roda os servidores MCP atrás do `sqz-mcp proxy`, e o relatório diz quanto disso vem do proxy cortando strings e arrays longos na primeira passada. Requisições à web, edições e resultados de sub-agentes ficam fora de alcance (out of reach) em todos os clientes e são contados, não reproduzidos.

Ele examina `~/.claude/projects` e `~/.kiro/sessions` (Kiro CLI e IDE), ou você pode apontá-lo para qualquer lugar com `--transcripts PATH`. Cada transcrição é reproduzida com um cache descartável novo, então as referências de deduplicação nunca fingem atravessar sessões, e o relatório fala em estimativa ("estimate") porque é disso que se trata: essas saídas não foram comprimidas na época.

**Usa só um agente?** Passe `--only` (ou `--skip`) para limitar quais configurações são gravadas:

```sh
sqz init --only opencode              # just OpenCode, nothing else
sqz init --only opencode,codex        # OpenCode and Codex
sqz init --skip cursor,windsurf       # everything except Cursor and Windsurf
```

Nomes aceitos: `claude`, `cursor`, `windsurf`, `cline`, `gemini`, `kiro`, `opencode`, `codex`. Os nomes alternativos (`claude-code`, `gemini-cli`, `roo`, `kiro-cli`) também funcionam. `--only` e `--skip` não podem ser combinados.

### Instalação manual (para preservar os comentários da sua configuração)

O `sqz init` lê e regrava o seu arquivo de configuração com um parser JSON para mesclar a entrada do sqz, o que descarta qualquer comentário no seu `opencode.jsonc` (e nos arquivos análogos de JSON com comentários que outras ferramentas aceitam). Se você comentou a sua configuração com cuidado e quer manter os comentários, instale à mão.

**OpenCode**, em dois passos:

1. Coloque o arquivo do plugin no lugar. O `sqz` imprime o TS gerado no stdout, para que você não precise escrever à mão a lógica de escape de caminhos:

   ```sh
   mkdir -p ~/.config/opencode/plugins
   sqz print-opencode-plugin > ~/.config/opencode/plugins/sqz.ts
   ```

2. Adicione você mesmo a entrada MCP ao seu `opencode.jsonc` existente. Acrescente este bloco dentro do objeto `mcp` de nível superior (crie o objeto `mcp` se ele não existir):

   ```jsonc
   "sqz": {
     "type": "local",
     "command": ["sqz-mcp", "--transport", "stdio"],
     "enabled": true
   }
   ```

Os comentários no resto do arquivo continuam onde estão. O OpenCode encontra o arquivo do plugin automaticamente; não é preciso uma entrada no array `plugin` (adicionar uma faz o plugin carregar duas vezes, veja a issue #10).

O mesmo arquivo do plugin é carregado no OpenCode 1 e no 2. O OpenCode 2 não carrega o plugin escrito pelo sqz 2.0.0 e por versões anteriores, então, depois de atualizar o sqz, rode `sqz doctor --fix` (ou rode de novo a linha `print-opencode-plugin` acima) para reescrevê-lo.

**Outras ferramentas**: o Claude Code e o Gemini CLI usam configurações em JSON puro, sem suporte a comentários, o Cursor, o Windsurf e o Cline recebem arquivos de regras em markdown, e o Codex recebe uma entrada TOML em `~/.codex/config.toml` que mantém os seus comentários, então o caminho automático não é destrutivo nesses casos. Use `sqz init --only <tool>` para eles. Um `.gemini/settings.json` existente recebe o hook do sqz mesclado; se ele tiver comentários ou não puder ser interpretado, o `sqz init` não mexe nele e mostra a entrada para você adicionar à mão.

Depois, rode `sqz doctor` para ver o que foi configurado em cada cliente e em qual nível de integração ele está.

## Como funciona <a id="how-it-works"></a>

<p align="center">
  <img src="../assets/sqz-architecture.png" alt="Arquitetura do sistema sqz" width="700" />
</p>

Nos clientes transparentes (transparent), que são Claude Code, Gemini CLI, Copilot CLI e OpenCode, o sqz instala um hook que reescreve os comandos de shell para que a saída deles passe pelo sqz antes de chegar à ferramenta de IA. Nos clientes orientativos (advisory), o agente é instruído a passar a saída por `sqz compress` ou a usar as ferramentas MCP do sqz. A tabela abaixo mostra o nível de cada cliente.

```
Claude → git status → [sqz hook rewrites] → compressed output
```

O que é comprimido:
- **Saída do shell**: formatadores por comando (git, cargo, npm/pnpm/yarn/bun, pytest, ruff, mypy, go, docker ps/images, kubectl describe/apply, terraform, gradle, maven, xcodebuild, adb logcat, dotnet, grep/rg, tree, curl e outros)
- **JSON**: remove valores null e vazios, corta strings com mais de 500 caracteres, codificação compacta, formato TOON
- **Logs**: condensa linhas repetidas
- **Saída de testes**: mostra só as falhas (parsers de máquina de estados para Rust, Go, Python, JS, JVM)

O que não é comprimido:
- Código-fonte: entregue completo na primeira leitura
- Stack traces, migrações SQL, blocos PEM e linhas com credenciais: vão para o modo seguro, passam sem alterações em qualquer tamanho e nunca são guardados em cache; no hook do shell, um formatador por comando ainda pode resumir o resto de uma execução de testes ao redor deles
- Os seus prompts e as respostas da IA: controlados pela ferramenta de IA, não pelo sqz

## Ferramentas compatíveis <a id="supported-tools"></a>

| Ferramenta | Integração | Configuração |
|---|---|---|
| Claude Code | Hooks PreToolUse + PostToolUse + servidor MCP (transparente/transparent) | `sqz init` |
| Cursor | Instruções em `.cursor/rules/sqz.mdc` (orientativo/advisory); servidor MCP adicionado à mão | `sqz init` |
| Windsurf | Instruções em `.windsurf/rules/sqz.md` (orientativo/advisory); servidor MCP adicionado à mão | `sqz init` |
| Cline | Instruções em `.clinerules` (orientativo/advisory); servidor MCP adicionado à mão | `sqz init` |
| Gemini CLI | Hook BeforeTool (transparente/transparent) | `sqz init` |
| Kiro | Steering + servidor MCP (orientativo/advisory) | `sqz init` |
| OpenCode | Plugin TypeScript para OpenCode 1 e 2 + servidor MCP (transparente/transparent) | `sqz init` |
| Codex CLI | Instruções em AGENTS.md + servidor MCP (orientativo/advisory) | `sqz init` |
| Zed | Instruções em AGENTS.md + servidor MCP (orientativo/advisory) | `sqz init` |
| Copilot CLI | Hook preToolUse (transparente/transparent), quando `~/.copilot` existe | `sqz init` |
| Copilot coding agent (CI) | Hook no nível do repositório, que se instala sozinho no sandbox da nuvem | `sqz init --ci` + commit |
| Qualquer pipeline de CI | [Ação setup-sqz](https://github.com/ojuschugh1/setup-sqz): comprime logs antes de uma etapa com LLM | `uses: ojuschugh1/setup-sqz@v1` |
| Qualquer servidor MCP | O `sqz-mcp proxy` encapsula o servidor e comprime os resultados das ferramentas dele | veja abaixo |
| VS Code | [Extensão](https://marketplace.visualstudio.com/items?itemName=ojuschugh1.sqz) | Instale pelo Marketplace |
| JetBrains | [Plugin](https://plugins.jetbrains.com/plugin/31240-sqz--context-intelligence/) | Instale pelo Marketplace |
| Chrome | Extensão de navegador | ChatGPT, Claude.ai, Gemini, Grok, Perplexity |
| [Firefox](https://addons.mozilla.org/en-US/firefox/addon/sqz-context-compression/) | Extensão de navegador | Os mesmos sites |

Os clientes transparentes (transparent) têm um hook que encaminha os comandos de shell pelo sqz por conta própria. Nos clientes orientativos (advisory), o sqz só economiza tokens quando o agente passa a saída por `sqz compress` ou chama as ferramentas MCP do sqz; as ferramentas de shell e de arquivos embutidas no cliente passam por fora dele. O `sqz doctor` mostra o nível de cada cliente.

## Comprima qualquer servidor MCP <a id="compress-any-mcp-server"></a>

O `sqz-mcp proxy` fica entre o seu agente e qualquer servidor MCP via stdio e comprime o que volta: os resultados das ferramentas passam pelo pipeline completo do sqz (referências de deduplicação nas repetições, modo seguro para stack traces e segredos, resultados de erro intactos), e as descrições verbosas das ferramentas são compactadas para que o `tools/list` pare de consumir a sua janela de contexto. O proxy injeta uma ferramenta `sqz_expand` para que o agente possa recuperar qualquer original em cache, idêntico byte a byte; resultados que mencionam credenciais, um stack trace ou uma migração nunca vão para o cache.

Encapsule um servidor colocando um prefixo no comando dele na sua configuração MCP:

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

Opções: `--lazy-tools` encurta cada descrição de ferramenta para uma frase e injeta uma ferramenta `sqz_tool_help` que entrega a documentação original completa sob demanda (alguns servidores MCP gastam 10-40k tokens só com `tools/list`); `--no-desc` mantém as descrições sem alterações; `--no-cache` não guarda nada, então não envia referências de deduplicação nem adiciona a ferramenta `sqz_expand`. Funciona com todo cliente MCP (Claude Code, Cursor, Windsurf, Zed, Codex, Kiro, ...) porque o cliente vê apenas um servidor MCP normal. Guia completo, incluindo as ferramentas `sqz_read_file` / `sqz_grep` / `sqz_list_dir` do próprio servidor e a configuração por cliente: [Compressão de contexto com MCP](../docs/mcp-context-compression.md).

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

### Como contornar a deduplicação

Quando o sqz vê o mesmo conteúdo duas vezes, ele devolve um token compacto `§ref:HASH§` em vez do texto completo. A maioria dos modelos lida bem com isso, mas alguns (por exemplo, o GLM 5.1) não conseguem interpretar o formato da referência e entram em loop. Há quatro maneiras de contornar isso:

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

### Recall: pesquise tudo o que o sqz já viu

Tudo o que passa pelo sqz é indexado localmente (SQLite FTS5, na sua máquina, nada sai dela). Quando o seu agente compacta o contexto e perde aquela mensagem de erro de uma hora atrás, pesquise por ela em vez de executar o comando de novo:

```sh
$ sqz recall "connection refused"
  1. [git · 2h ago]  … curl: (7) Failed to connect: >>connection refused<< on port 5432 …
     full content: sqz expand 3f9a1c05e2d84b17
```

Os agentes têm acesso ao mesmo recurso pela ferramenta MCP `sqz_recall`.

## Acompanhe a sua própria economia

Rode `sqz gain` no seu shell a qualquer momento para ver o seu próprio detalhamento diário (veja na seção Economia de tokens, acima, como é a saída), e `sqz stats` para o relatório acumulado completo:

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

Adicione `--breakdown` para ver exatamente quais comandos consomem mais tokens:

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

O `sqz stats` e o `sqz gain` contam tokens cl100k do que o sqz realmente imprimiu, incluindo a legenda de abreviações e a dica de expansão. Uma sequência de uma mesma classe de caracteres com mais de 2 KB, como uma linha separadora longa, é contada em fatias de 2 KB, o que pode errar a contagem em cerca de 1% mais 4 tokens.

### Contabilidade honesta: custo e arrependimento

Redução de tokens não é automaticamente redução do custo cobrado: com o cache de prompts do provedor, a maior parte do contexto é retransmitida com um desconto de ~90%, e uma compressão que descarta algo de que o agente precisava acaba custando turnos extras ([arXiv:2607.12161](https://arxiv.org/abs/2607.12161)). O sqz mede os dois lados em vez de fazer afirmações vagas:

- O `sqz stats --cost` estima os dólares economizados em um modelo de cobrança com cache de prompts (escrita no cache ×1.25 uma vez, leitura do cache ×0.10 em cada turno seguinte), com todas as premissas impressas e ajustáveis (`--price-in`, `--reread-turns`, `--cache-write-mult`, `--cache-read-mult`). Essa é a estimativa conservadora: sem cache, a mesma economia valeria o preço cheio de entrada em todos os turnos.
- O `sqz stats` relata sinais de arrependimento (regret signals): as reexecuções rápidas (quick re-runs), em que o agente produziu de novo uma saída idêntica byte a byte em até 2 minutos e a repetição não rendeu nada (o hook PostToolUse do Claude Code nunca responde com uma referência, então as repetições dele contam como compressões comuns), e as expansões de referência (ref expands), que são tokens `§ref§` recuperados para os bytes originais. Os dois servem como indicadores indiretos de que "a compressão descartou algo de que o modelo precisava". Se um comando continua aparecendo, o formatador dele precisa de ajustes: abra uma issue.

O design do sqz já evita os modos de falha que esse estudo mediu: a compressão é determinística e independente da consulta (nunca invalida o cache de prompts do provedor), só a saída nova das ferramentas é alterada (o histórico nunca é reescrito), comandos que ficam acompanhando a saída, rodam um servidor ou colocam um job em segundo plano ficam como estão (no Claude Code fora do modo bypass, o hook PreToolUse deixa pipelines, loops e heredocs como foram escritos, e o hook PostToolUse comprime a saída deles depois de uma execução bem-sucedida), listas agrupadas e pipelines (Gemini CLI, Copilot CLI, Claude Code no modo bypass) pulam os formatadores por comando, e o filtro de ganho líquido de 16 tokens pula compressões que não compensariam os próprios marcadores. Um comando também fica como foi escrito quando a reescrita dele poderia executar um conjunto de comandos diferente do original. Em `ls ${DEST:?}; echo next`, passar só o `ls` pelo pipe deixaria o `echo next` rodar depois que a expansão com falha interrompeu o shell, e quando uma lista passa pelo sqz como um único grupo, um `cd` com falha em `cd build && make; make test` pularia o grupo inteiro em vez de só o `make`.
<img width="3456" height="1918" alt="imagem" src="https://github.com/user-attachments/assets/d308bc60-81f3-4935-9728-b30534e5b5e5" />


**Filtrar por projeto:**

```sh
sqz stats --project .           # stats for current project only
sqz stats --project list        # list all tracked projects
sqz gain --project .            # daily gains for current project
sqz gain --days 30              # last 30 days instead of 7
sqz gain --days 30 --project .  # combine both
```

As estatísticas ficam guardadas localmente no SQLite, em `~/.sqz/sessions.db`: nada sai da sua máquina.

## Como a compressão funciona

1. **Formatadores por comando** para os comandos abaixo:

   | Ecossistema | Comandos |
   |---|---|
   | Git | status, log, diff, show, stash, remote, fetch, push, pull, add, commit |
   | Rust | cargo build/test/clippy/check/nextest |
   | JavaScript | npm/pnpm/yarn/bun install/audit/outdated (test e run quando falham), tsc, eslint, biome |
   | Python | pytest, ruff, mypy, pip |
   | Go | go test (incluindo o stream `-json`), go build, go vet, golangci-lint |
   | Nuvem | terraform plan/apply/destroy/refresh/init |
   | Contêineres | docker/podman ps/images, kubectl describe/apply |
   | JVM | gradle build/test, maven |
   | Mobile | xcodebuild (build + test), adb logcat |
   | .NET | dotnet build/publish/test |
   | Sistema | grep/rg, tree, curl/wget |

   Outros comandos, incluindo kubectl get e logs, gh, aws, gcloud, ls e find, passam pelo pipeline de compressão genérico.

2. **Compactador de tabelas**: a saída em colunas alinhadas de ferramentas sem um formatador dedicado (`ps aux`, `netstat`, CLIs de banco de dados) tem as sequências de espaços de preenchimento reduzidas a separadores de dois espaços. O detector é rigoroso: linhas indentadas, código, YAML e JSON nunca são reconhecidos como tabela.
3. **Resumos estruturais**: o motor tem uma API que reduz um arquivo de código a imports, assinaturas de funções e um grafo de chamadas. Os caminhos do hook e do MCP não a usam; eles entregam o código-fonte completo.
4. **Cache de deduplicação**: hash SHA-256 do conteúdo, persistente entre sessões. Saída idêntica byte a byte que o modelo já tem = referência de 13 tokens. Um intervalo de linhas de um arquivo que o modelo já tem completo (`sed -n '40,80p'`, `sqz_read_file` com `offset`/`limit`) = `§ref:HASH:L40-80§`. Uma releitura depois de uma pequena edição = as linhas alteradas com uma linha inalterada de cada lado, pelas ferramentas MCP do sqz (o hook do shell envia o arquivo editado de novo); edições espalhadas pelo arquivo voltam completas. As referências só são entregues enquanto o original ainda está no contexto (janela de 30 minutos, ajustável com `SQZ_REF_TTL_SECS`, reiniciada na compactação).
5. **Pipeline de JSON**: remover nulls → descartar campos de controle interno (`__v`, `etag`, ids de requisição e de correlação) → achatar → colapsar arrays → codificação TOON (formato compacto sem perdas). Campos cujo valor é uma string, um array ou um objeto vazios são descartados em qualquer profundidade, e strings com mais de 500 caracteres são cortadas nos primeiros 500 caracteres, seguidos de `...`.
6. **Modo seguro**: stack traces (um cabeçalho de trace seguido dos frames dele), DDL de SQL no início de uma linha, blocos PEM e linhas com credenciais (`key=value` ou `key: value` com uma chave que parece secreta), com ou sem cores ANSI, pulam o pipeline e passam sem alterações em qualquer tamanho, com apenas os códigos ANSI removidos, e nunca vão para o cache. No hook do shell, um formatador por comando ainda roda primeiro. O resumo dele só é usado quando mantém inalteradas todas as linhas de cada parágrafo que contém uma dessas estruturas, então ele pode resumir o resto de uma execução de testes ao redor de um trace; caso contrário, a saída inteira passa direto. Uma saída que só menciona uma senha, uma chave, um trace ou uma migração fora dessas estruturas é comprimida normalmente, mas também não vai para o cache, então nada do que uma etapa com perdas descartar dela pode ser expandido.

Se uma etapa gerar um panic, o `sqz compress` imprime a entrada sem alterações, com uma linha no stderr, e mantém o código de saída do comando, e o hook PostToolUse do Claude Code deixa o resultado como estava.

Qualidade medida, incluindo o que cada etapa descarta e o que ela nunca toca: [benchmark de qualidade](../docs/quality-benchmark.md). Para todos os detalhes técnicos, veja [docs/](../docs/README.md).

## Configuração

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

### Banco de dados por projeto

Tudo o que o sqz persiste (estatísticas, cache de deduplicação, sessões) fica em um único arquivo SQLite, `~/.sqz/sessions.db` por padrão. Defina `SQZ_DB_PATH` para manter um arquivo por projeto:

```sh
# e.g. in the project's .envrc (direnv)
export SQZ_DB_PATH="$PWD/.sqz/sessions.db"
```

Todas as partes do sqz respeitam essa variável: o hook do shell, `sqz stats`/`gain`/`expand` e o servidor MCP (defina-a em `env` na sua configuração MCP). Prefira caminhos absolutos: valores relativos são resolvidos a partir do diretório em que o processo estiver rodando. O diretório pai é criado se não existir (modo 0700, como `~/.sqz`).

## Privacidade

- Telemetria zero: nenhum dado transmitido, nenhum relatório de falhas
- Totalmente offline: funciona em ambientes isolados da rede
- Todo o processamento é local

## Desenvolvimento

```sh
git clone https://github.com/ojuschugh1/sqz.git
cd sqz
cargo test --workspace
cargo build --release
```

## Licença

O sqz é licenciado sob a [Apache License, Version 2.0](../LICENSE-APACHE) ou a [MIT License](../LICENSE-MIT), à sua escolha.

Copyright 2026 Ojus Chugh

As versões até a 2.0.0, inclusive, foram publicadas sob a [Elastic License 2.0](https://github.com/ojuschugh1/sqz/blob/v2.0.0/LICENSE).

As contribuições são aceitas sob o [Contrato de Licença de Contribuidor (CLA)](../CLA.md).

## Links

- [Índice da documentação](../docs/README.md)
- [Como impedir que agentes de programação com IA releiam os mesmos arquivos](../docs/stop-rereading-files.md)
- [Compressão de contexto com MCP](../docs/mcp-context-compression.md)
- [Benchmark de qualidade: taxa de compressão versus preservação da informação](../docs/quality-benchmark.md)
- [Degradação do contexto versus compressão de contexto](../docs/context-rot.md)
- [Como escolher uma abordagem de compressão de contexto](../docs/comparison.md)
- [White paper: compressão de contexto antes da injeção](../docs/whitepaper.md)
- [Benchmark de economia de tokens](../docs/benchmark.md)
- [Discord](https://discord.gg/j8EEyH5dSB)
- [Registro de alterações](../CHANGELOG.md)
- Nome no MCP Registry: `mcp-name: io.github.ojuschugh1/sqz`

## Histórico de estrelas

Se o sqz economizou tokens para você hoje, uma estrela ajuda outras pessoas a encontrá-lo. O `sqz stats --share` imprime os seus próprios números, se você quiser mostrá-los.

<a href="https://star-history.dera.page/#ojuschugh1/sqz&Date">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://star-history.dera.page/svg?repos=ojuschugh1/sqz&type=Date&theme=dark" />
   <source media="(prefers-color-scheme: light)" srcset="https://star-history.dera.page/svg?repos=ojuschugh1/sqz&type=Date" />
   <img alt="Gráfico do histórico de estrelas" src="https://star-history.dera.page/svg?repos=ojuschugh1/sqz&type=Date" width="600" />
 </picture>
</a>

## Contribuidores

Obrigado a todas as pessoas que contribuíram com código, correções e ideias para o sqz:

<a href="https://github.com/ojuschugh1/sqz/graphs/contributors">
  <img src="https://contrib.rocks/image?repo=ojuschugh1/sqz" alt="Contribuidores do sqz" />
</a>

E a todas as pessoas que abriram os relatórios de bug detalhados por trás das nossas correções. Reproduções precisas tornam este projeto melhor a cada versão. Quer se juntar a elas? Os PRs são revisados rapidamente: veja as [issues abertas](https://github.com/ojuschugh1/sqz/issues) para começar.

<sub>Grade de contribuidores feita com [contrib.rocks](https://contrib.rocks).</sub>
