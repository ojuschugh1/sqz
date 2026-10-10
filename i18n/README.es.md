<!-- i18n-source: README.md sha256:e20e0153585ab14d62a29797338f75e9415f19639ba4f71317cabc907c46f4f6 -->
<!-- i18n:languages -->
[English](../README.md) | [Español](README.es.md)
<!-- /i18n:languages -->

<!-- i18n:note -->
> Esta es una traducción del [README.md en inglés](../README.md).
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
  <strong>Compresión de contexto previa a la inyección y deduplicación a nivel de sesión para agentes de programación con IA</strong>
</p>

<p align="center">
  <sub>
    <strong>Estadísticas de sesiones reales:</strong>
    3,003 compresiones ·
    <strong>178,442 tokens ahorrados</strong> ·
    24.7% de reducción media · hasta un
    <strong>92%</strong> en la salida que el modelo ya tiene
  </sub>
</p>

<p align="center">
  <a href="https://thenextgentechinsider.com/pulse/sqz-tool-cuts-llm-token-use-by-92-for-file-heavy-ai-tasks"><img src="https://img.shields.io/badge/%231_Featured-NextGen_Tech_Insider-ff6600?style=for-the-badge&logo=newspaper&logoColor=white" alt="Destacado"></a>
  <a href="https://hysenlabs.com/en/projects/ojuschugh1-sqz"><img src="https://img.shields.io/badge/Featured-Hysen_Labs-1f6feb?style=for-the-badge&logo=readme&logoColor=white" alt="Destacado en Hysen Labs"></a>
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
  <a href="https://ojuschugh1.github.io/sqz/">Documentación</a> ·
  <a href="#install">Instalación</a> ·
  <a href="#how-it-works">Cómo funciona</a> ·
  <a href="#supported-tools">Herramientas compatibles</a> ·
  <a href="#compress-any-mcp-server">Proxy MCP</a> ·
  <a href="../docs/quality-benchmark.md">Benchmark</a> ·
  <a href="../CHANGELOG.md">Registro de cambios</a> ·
  <a href="https://discord.gg/j8EEyH5dSB">Discord</a>
</p>

---

Los agentes de programación con IA gastan la mayor parte de sus tokens de entrada en la salida de las herramientas: logs de compilación, ejecuciones de pruebas, `git status`, el mismo archivo leído otra vez después de cada edición. Todo eso entra en bruto en la ventana de contexto y se vuelve a enviar en cada turno a partir de ese momento.

sqz comprime la salida de las herramientas antes de que entre en el contexto. Los formateadores por comando conservan aquello sobre lo que actúa un agente (la prueba que falla, la aserción, el `file:line`) y descartan lo demás. Todo lo que el modelo ya tiene en el contexto vuelve como una referencia corta en lugar del contenido. El código fuente pasa sin cambios, y lo mismo ocurre con las trazas de pila, las migraciones y las líneas con credenciales.

```sh
cargo install sqz-cli sqz-mcp     # or: brew install ojuschugh1/sqz/sqz · npm i -g sqz-cli · pipx install sqz
sqz init                          # hooks + MCP server + agent guidance for every client it finds
```

<p align="center">
  <img src="../assets/demo.gif" alt="sqz: un archivo de 200 líneas se sirve completo, una relectura de las líneas 41-80 se convierte en una referencia a un rango de líneas, la salida de cargo test se reduce al fallo y sqz stats muestra ambas cosas" width="800">
</p>

<sub>Salida real del binario de release (<code>assets/demo.tape</code>). Para regenerarla, ejecuta <code>vhs assets/demo.tape</code>.</sub>

```
Without sqz:                              With sqz:

cat auth.py (200 lines):   1,300 tokens    1,300 tokens  (source is served in full)
cargo test (1 failure):      366 tokens      185 tokens  (failure, assertion, file:line)
sed -n '41,80p' auth.py:     260 tokens       35 tokens  (§ref:…:L41-80§ to the read above)
git status, unchanged:       166 tokens       37 tokens  (§ref:…§)
─────────────────────────────────────      ─────────────
Total:                     2,092 tokens    1,557 tokens  (26% saved, nothing dropped)
```

Un único binario de Rust, determinista, sin llamadas a LLM y que funciona sin conexión. Cualquier resultado comprimido se puede recuperar idéntico byte a byte con `sqz expand`, salvo la salida que menciona credenciales, una traza de pila o una migración, que sqz nunca guarda en caché.

> [!NOTE]
> **Desambiguación del nombre:** este repositorio, [ojuschugh1/sqz](https://github.com/ojuschugh1/sqz), es un proyecto independiente y no está afiliado a ninguna otra herramienta con un nombre o un funcionamiento parecidos, ni a otros proyectos de compresión que abrevian "squeeze". Si instalaste `sqz` / `sqz-cli` / `sqz-mcp` desde crates.io, npm, PyPI o Homebrew, proviene de este repositorio.

## Ahorro de tokens

> **24.7%** de reducción media en 3,003 compresiones reales ·
> referencias de **13 tokens** para la salida que el modelo ya tiene ·
> **100%** de los datos críticos para el agente conservados en el [benchmark de calidad](../docs/quality-benchmark.md) ·
> **0%** en el código fuente, y también en las trazas de pila y las líneas con credenciales

La semana de un desarrollador, medida a partir de la salida real de `sqz gain`:

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

### Compresión por comando

Compresión de un solo comando (medida con `cargo test -p sqz-engine benchmarks`):

| Contenido | Antes | Después | Ahorro |
|---|---:|---:|---:|
| Líneas de log repetidas | 148 | 39 | **74%** |
| Array JSON grande | 259 | 111 | **57%** |
| Respuesta de una API JSON | 64 | 53 | **17%** |
| Diff de Git | 61 | 54 | **11%** |
| Prosa/documentación | 124 | 122 | **2%** |
| Traza de pila (modo seguro) | 82 | 82 | **0%** |

### A nivel de sesión, con deduplicación

Todo lo que el modelo ya tiene en su contexto vuelve como una referencia. Medido con el binario de release contra una base de datos desechable, con los datos de prueba de `sqz/tests/quality_bench.rs` y `demo/`, en tokens cl100k de todo lo que ve el agente (la referencia más la nota de una línea que sqz escribe en stderr):

| Repetición | Sin sqz | Con sqz | Ahorro |
|---|---:|---:|---:|
| `git status` sin cambios, ejecutado otra vez | 166 | 37 | **78%** |
| Líneas 41-80 de un archivo de 200 líneas leído antes | 260 | 35 | **87%** |
| La misma salida de `cargo test`, sin cambios | 366 | 39 | **89%** |

Unas palabras sobre lo que de verdad se repite. Un usuario que contó 92 de sus propias sesiones de Claude Code ([script](https://gist.github.com/harrison001/76250b61ad665bf9a377e7096dffdbe0)) encontró relecturas idénticas de un archivo completo en 1 de 542 lecturas, y relecturas de rangos de líneas en el 8.4% de ellas. Por eso sqz no pone por delante "el mismo archivo leído cinco veces": las repeticiones que ocurren en la práctica son salidas de comandos sin cambios, rangos de líneas de un archivo ya leído y archivos que se vuelven a leer tras una edición pequeña, y cada una tiene su propio tipo de referencia ([detalles](../docs/stop-rereading-files.md)). Las sesiones con salidas de comandos ruidosas y comandos repetidos son las que más ahorran.

## Instalación <a id="install"></a>

**Binarios precompilados** (no hace falta compilador; funcionan en todas las plataformas):

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

**Compilar desde el código fuente con Cargo:**

```sh
cargo install sqz-cli sqz-mcp
```

`sqz-cli` proporciona el binario `sqz`; `sqz-mcp` proporciona el servidor MCP. `sqz-engine` es una dependencia de biblioteca: se compila automáticamente y no hace falta instalarla por separado.

**Compilar desde el código fuente** (`cargo install sqz-cli`) también funciona, pero necesita una cadena de herramientas de C:

- Linux: `build-essential` (apt) o equivalente
- macOS: Xcode Command Line Tools (`xcode-select --install`)
- **Windows: Visual Studio Build Tools con la carga de trabajo "Desktop development with C++".** Sin ellas, `cargo install` falla con `linker link.exe not found`. Si todavía no las tienes, usa en su lugar la instalación con PowerShell o npm de más arriba.

Después, inicializa:

```sh
sqz init --global     # hooks apply to every project on this machine
# or
sqz init              # hooks apply to just this project (.claude/settings.local.json)
```

`--global` escribe en `~/.claude/settings.json` (el ámbito de usuario según la [tabla de ámbitos de Anthropic](https://docs.claude.com/en/docs/claude-code/settings)), así que el hook de sqz se activa en todas las sesiones de Claude Code de este equipo. Es el caso habitual en la primera instalación. Se conservan tus `permissions`, `env` y `statusLine` existentes, y también los hooks no relacionados de `~/.claude/settings.json`: sqz fusiona sus entradas en lugar de sobrescribirlo. Esas entradas son sus hooks y dos reglas de autorización, `Bash(set -o pipefail)` y `Bash(sqz compress --cmd *)`, que solo cubren las partes que añade su reescritura. sqz no aprueba comandos; siguen decidiendo tus propias reglas de permisos. Cuando Claude Code pregunta antes de ejecutar un comando reescrito, o lo deniega, su mensaje cita el comando reescrito, por ejemplo `set -o pipefail; git tag v0 2>&1` para `git tag v0`.

`sqz init` sin opciones (ámbito de proyecto) es útil cuando quieres que sqz esté activo solo dentro de un repositorio. Si `~/.claude/settings.json` ya tiene los hooks de sqz, no los incluye en el archivo del proyecto, porque ya se ejecutan en todos los proyectos.

Después de instalar, `sqz doctor` revisa toda la cadena (el binario, la base de datos, el hook de shell, qué clientes se detectan frente a cuáles pasan realmente por sqz y si se ha comprimido algo recientemente) y muestra la corrección para cada problema que encuentra. `sqz doctor --fix` aplica esas correcciones. Después de actualizar sqz, ejecútalo una vez: pone al día los hooks y las instrucciones que escribió una versión anterior, elimina los archivos de hooks que la versión actual de sqz ya no usa y la copia en el proyecto de los hooks que `~/.claude/settings.json` ya tiene, hace que los hooks cuyo binario de sqz ya no existe apunten al que se está ejecutando, y avisa de cualquier hook que siga ejecutando un binario de sqz distinto. `sqz uninstall` elimina solo las entradas propias de sqz, borra un archivo solo cuando no queda nada más en él y termina mostrando la lista de los archivos que conservó. Si no hay una terminal donde responder a su pregunta, necesita `--yes`, igual que `sqz reset`.

### Comprueba cuánto habría ahorrado sqz, antes de instalar ningún hook

Si ya usas Claude Code o Kiro, tus transcripciones están en el disco. Reprodúcelas con el motor de sqz y obtén una estimación contrafactual sobre tus propias sesiones, calculada localmente y sin subir nada. Salida de la última semana de trabajo del autor en Kiro IDE, reducida al resumen:

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

Cada salida se reproduce tal como sqz podría haberla alcanzado realmente en ese cliente. Kiro no permite que sqz reescriba una llamada a una herramienta, así que allí el primer total es cero; en Claude Code, las filas de shell se dividen entre los comandos que el hook reescribe por sí solo y los que omite. El segundo total supone que el agente pasa la salida del shell por `sqz compress`, lee los archivos con las herramientas MCP de sqz (sin pérdida, así que solo cuentan las repeticiones) y ejecuta los servidores MCP detrás de `sqz-mcp proxy`, y el informe indica qué parte de eso corresponde al proxy cuando recorta cadenas y arrays largos en la primera pasada. Las peticiones web, las ediciones y los resultados de subagentes quedan fuera de alcance (out of reach) en todos los clientes y se cuentan, pero no se reproducen.

Analiza `~/.claude/projects` y `~/.kiro/sessions` (Kiro CLI e IDE), o puedes indicarle cualquier otra ubicación con `--transcripts PATH`. Cada transcripción se reproduce contra una caché desechable nueva, así que las referencias de deduplicación nunca fingen abarcar varias sesiones, y el informe dice que es una estimación porque eso es lo que es: estas salidas no se comprimieron en su momento.

**¿Solo usas un agente?** Pasa `--only` (o `--skip`) para limitar qué configuraciones se escriben:

```sh
sqz init --only opencode              # just OpenCode, nothing else
sqz init --only opencode,codex        # OpenCode and Codex
sqz init --skip cursor,windsurf       # everything except Cursor and Windsurf
```

Nombres aceptados: `claude`, `cursor`, `windsurf`, `cline`, `gemini`, `kiro`, `opencode`, `codex`. También funcionan los alias (`claude-code`, `gemini-cli`, `roo`, `kiro-cli`). `--only` y `--skip` no se pueden combinar.

### Instalación manual (para conservar los comentarios de tu configuración)

`sqz init` lee y vuelve a escribir tu archivo de configuración con un parser de JSON para fusionar la entrada de sqz, y eso elimina todos los comentarios de tu `opencode.jsonc` (y de los archivos análogos de JSON con comentarios que aceptan otras herramientas). Si has comentado tu configuración con cuidado y quieres conservar los comentarios, haz la instalación a mano.

**OpenCode** (dos pasos):

1. Coloca el archivo del plugin en su sitio. `sqz` imprime el TS generado en stdout para que no tengas que escribir a mano la lógica de escape de rutas:

   ```sh
   mkdir -p ~/.config/opencode/plugins
   sqz print-opencode-plugin > ~/.config/opencode/plugins/sqz.ts
   ```

2. Añade tú mismo la entrada MCP a tu `opencode.jsonc` existente. Agrega este bloque dentro del objeto `mcp` de nivel superior (crea el objeto `mcp` si no existe):

   ```jsonc
   "sqz": {
     "type": "local",
     "command": ["sqz-mcp", "--transport", "stdio"],
     "enabled": true
   }
   ```

Los comentarios del resto del archivo se quedan como están. OpenCode descubre el archivo del plugin automáticamente; no hace falta ninguna entrada en el array `plugin` (si la añades, se carga dos veces; consulta el issue #10).

**Otras herramientas**: Claude Code y Gemini CLI usan configuraciones JSON simples que no admiten comentarios, Cursor, Windsurf y Cline reciben archivos de reglas en markdown, y Codex recibe una entrada TOML en `~/.codex/config.toml` que conserva tus comentarios, así que en esos casos la vía automática no destruye nada. Para ellas, usa `sqz init --only <tool>`. En un `.gemini/settings.json` existente se fusiona el hook de sqz; si tiene comentarios o no se puede analizar, `sqz init` no lo toca y muestra la entrada que debes añadir a mano.

Después, ejecuta `sqz doctor` para ver qué se configuró en cada cliente y en qué nivel de integración está.

## Cómo funciona <a id="how-it-works"></a>

<p align="center">
  <img src="../assets/sqz-architecture.png" alt="Arquitectura del sistema de sqz" width="700" />
</p>

En los clientes transparentes (transparent), que son Claude Code, Gemini CLI, Copilot CLI y OpenCode, sqz instala un hook que reescribe los comandos de shell para que su salida pase por sqz antes de llegar a la herramienta de IA. En los clientes orientativos (advisory), se le indica al agente que pase la salida por `sqz compress` o que use las herramientas MCP de sqz. La tabla de más abajo indica el nivel de cada cliente.

```
Claude → git status → [sqz hook rewrites] → compressed output
```

Qué se comprime:
- **Salida del shell**: formateadores por comando (git, cargo, npm/pnpm/yarn/bun, pytest, ruff, mypy, go, docker ps/images, kubectl describe/apply, terraform, gradle, maven, xcodebuild, adb logcat, dotnet, grep/rg, tree, curl y más)
- **JSON**: elimina los nulos y los valores vacíos, recorta las cadenas de más de 500 caracteres, codificación compacta, formato TOON
- **Logs**: colapsa las líneas repetidas
- **Salida de las pruebas**: muestra solo los fallos (parsers basados en máquinas de estados para Rust, Go, Python, JS, JVM)

Qué no se comprime:
- Código fuente: se sirve completo en la primera lectura
- Trazas de pila, migraciones SQL, bloques PEM y líneas con credenciales: se envían al modo seguro, pasan sin cambios sea cual sea su tamaño y nunca se guardan en caché; en el hook de shell, un formateador por comando todavía puede resumir el resto de una ejecución de pruebas a su alrededor
- Tus prompts y las respuestas de la IA: los controla la herramienta de IA, no sqz

## Herramientas compatibles <a id="supported-tools"></a>

| Herramienta | Integración | Configuración |
|---|---|---|
| Claude Code | Hooks PreToolUse + PostToolUse + servidor MCP (transparente/transparent) | `sqz init` |
| Cursor | Instrucciones en `.cursor/rules/sqz.mdc` (orientativo/advisory); servidor MCP añadido a mano | `sqz init` |
| Windsurf | Instrucciones en `.windsurf/rules/sqz.md` (orientativo/advisory); servidor MCP añadido a mano | `sqz init` |
| Cline | Instrucciones en `.clinerules` (orientativo/advisory); servidor MCP añadido a mano | `sqz init` |
| Gemini CLI | Hook BeforeTool (transparente/transparent) | `sqz init` |
| Kiro | Steering + servidor MCP (orientativo/advisory) | `sqz init` |
| OpenCode | Plugin de TypeScript + servidor MCP (transparente/transparent) | `sqz init` |
| Codex CLI | Instrucciones en AGENTS.md + servidor MCP (orientativo/advisory) | `sqz init` |
| Zed | Instrucciones en AGENTS.md + servidor MCP (orientativo/advisory) | `sqz init` |
| Copilot CLI | Hook preToolUse (transparente/transparent), cuando existe `~/.copilot` | `sqz init` |
| Copilot coding agent (CI) | Hook a nivel de repositorio, que se instala por sí solo en el sandbox de la nube | `sqz init --ci` + commit |
| Cualquier pipeline de CI | [Acción setup-sqz](https://github.com/ojuschugh1/setup-sqz): comprime los logs antes de un paso con un LLM | `uses: ojuschugh1/setup-sqz@v1` |
| Cualquier servidor MCP | `sqz-mcp proxy` lo envuelve y comprime los resultados de sus herramientas | Consulta más abajo |
| VS Code | [Extensión](https://marketplace.visualstudio.com/items?itemName=ojuschugh1.sqz) | Instala desde el Marketplace |
| JetBrains | [Plugin](https://plugins.jetbrains.com/plugin/31240-sqz--context-intelligence/) | Instala desde el Marketplace |
| Chrome | Extensión de navegador | ChatGPT, Claude.ai, Gemini, Grok, Perplexity |
| [Firefox](https://addons.mozilla.org/en-US/firefox/addon/sqz-context-compression/) | Extensión de navegador | Los mismos sitios |

Los clientes transparentes (transparent) tienen un hook que, por sí solo, hace pasar los comandos de shell por sqz. En los clientes orientativos (advisory), sqz solo ahorra tokens cuando el agente pasa la salida por `sqz compress` o llama a las herramientas MCP de sqz; las herramientas de shell y de archivos integradas en el cliente no pasan por sqz. `sqz doctor` muestra el nivel de cada cliente.

## Comprime cualquier servidor MCP <a id="compress-any-mcp-server"></a>

`sqz-mcp proxy` se coloca entre tu agente y cualquier servidor MCP de stdio y comprime lo que vuelve: los resultados de las herramientas pasan por todo el pipeline de sqz (referencias de deduplicación en las repeticiones, modo seguro para las trazas de pila y los secretos, resultados de error sin tocar), y las descripciones extensas de las herramientas se compactan para que `tools/list` deje de comerse tu ventana de contexto. El proxy inyecta una herramienta `sqz_expand` para que el agente pueda recuperar cualquier original guardado en caché, idéntico byte a byte; los resultados que mencionan credenciales, una traza de pila o una migración nunca se guardan en caché.

Para envolver un servidor, pon un prefijo delante de su comando en tu configuración MCP:

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

Opciones: `--lazy-tools` acorta cada descripción de herramienta a una sola frase e inyecta una herramienta `sqz_tool_help` que sirve la documentación original completa bajo demanda (algunos servidores MCP gastan 10-40k tokens solo en `tools/list`); `--no-desc` conserva las descripciones tal cual; `--no-cache` no guarda nada, así que no envía referencias de deduplicación ni añade la herramienta `sqz_expand`. Funciona con cualquier cliente MCP (Claude Code, Cursor, Windsurf, Zed, Codex, Kiro, ...) porque el cliente solo ve un servidor MCP normal. Guía completa, con las herramientas propias del servidor `sqz_read_file` / `sqz_grep` / `sqz_list_dir` y la configuración de cada cliente: [compresión de contexto MCP](../docs/mcp-context-compression.md).

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

### Vía de escape de la deduplicación

Cuando sqz ve el mismo contenido dos veces, devuelve un token compacto `§ref:HASH§` en lugar del texto completo. La mayoría de los modelos lo manejan bien, pero algunos (por ejemplo, GLM 5.1) no saben interpretar el formato de las referencias y entran en un bucle. Cuatro formas de sortearlo:

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

### Recall: busca en todo lo que sqz ha visto

Todo lo que pasa por sqz se indexa localmente (SQLite FTS5, en tu equipo; nada sale de él). Cuando tu agente compacta su contexto y pierde aquel mensaje de error de hace una hora, búscalo en lugar de volver a ejecutar el comando:

```sh
$ sqz recall "connection refused"
  1. [git · 2h ago]  … curl: (7) Failed to connect: >>connection refused<< on port 5432 …
     full content: sqz expand 3f9a1c05e2d84b17
```

Los agentes tienen lo mismo en forma de la herramienta MCP `sqz_recall`.

## Mide tu propio ahorro

Ejecuta `sqz gain` en tu shell cuando quieras para ver tu propio desglose diario (la sección Ahorro de tokens, más arriba, muestra cómo es la salida), y `sqz stats` para ver el informe acumulado completo:

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

Añade `--breakdown` para ver exactamente qué comandos consumen más tokens:

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

`sqz stats` y `sqz gain` cuentan los tokens cl100k de lo que sqz imprimió realmente, incluidas la leyenda de abreviaturas y la indicación para expandir. Una secuencia de caracteres de una misma clase que supera los 2 KB, como una línea separadora larga, se cuenta en fragmentos de 2 KB, lo que puede desviar el recuento en torno a un 1% más 4 tokens.

### Contabilidad honesta: costo y arrepentimiento

Reducir tokens no equivale automáticamente a reducir el costo facturado: con la caché de prompts del proveedor, la mayor parte del contexto se retransmite con un descuento de ~90%, y una compresión que elimina algo que el agente necesitaba cuesta, en cambio, turnos adicionales ([arXiv:2607.12161](https://arxiv.org/abs/2607.12161)). sqz mide ambos lados en lugar de quedarse en vaguedades:

- `sqz stats --cost` estima los dólares ahorrados según un modelo de facturación con caché de prompts (escritura en caché ×1.25 una vez, lectura de caché ×0.10 en cada turno posterior), con todos los supuestos impresos y modificables (`--price-in`, `--reread-turns`, `--cache-write-mult`, `--cache-read-mult`). Es la estimación conservadora: sin caché, el mismo ahorro se facturaría al precio de entrada completo en cada turno.
- `sqz stats` informa de señales de arrepentimiento (regret signals): reejecuciones rápidas (quick re-runs: el agente volvió a producir una salida idéntica byte a byte en menos de 2 minutos y la repetición no sirvió de nada; el hook PostToolUse de Claude Code nunca responde con una referencia, así que sus repeticiones cuentan como compresiones normales) y expansiones de referencia (ref expands: tokens `§ref§` recuperados a sus bytes originales). Ambas son indicadores indirectos de que "la compresión eliminó algo que el modelo necesitaba". Si un comando aparece una y otra vez, su formateador necesita mejoras: abre un issue.

El diseño de sqz ya evita los modos de fallo que midió ese estudio: la compresión es determinista e independiente de la consulta (nunca invalida la caché de prompts del proveedor), solo se toca la salida nueva de las herramientas (el historial nunca se reescribe), los comandos que se quedan siguiendo una salida, ejecutan un servidor o mandan un trabajo a segundo plano no se tocan (en Claude Code fuera del modo bypass, el hook PreToolUse deja tal cual las tuberías, los bucles y los heredocs, y el hook PostToolUse comprime su salida después de una ejecución correcta), las listas agrupadas y las tuberías (Gemini CLI, Copilot CLI, Claude Code en modo bypass) se saltan los formateadores por comando, y el umbral de ganancia neta de 16 tokens descarta las compresiones que no compensarían sus propios marcadores. Un comando también se deja tal cual cuando su reescritura podría ejecutar un conjunto de comandos distinto del original. En `ls ${DEST:?}; echo next`, canalizar `ls` por separado permitiría que `echo next` se ejecutara después de que la expansión fallida detuviera el shell, y cuando una lista pasa por sqz como un solo grupo, un `cd` fallido en `cd build && make; make test` haría que se saltara el grupo en lugar de solo `make`.
<img width="3456" height="1918" alt="imagen" src="https://github.com/user-attachments/assets/d308bc60-81f3-4935-9728-b30534e5b5e5" />


**Filtrado por proyecto:**

```sh
sqz stats --project .           # stats for current project only
sqz stats --project list        # list all tracked projects
sqz gain --project .            # daily gains for current project
sqz gain --days 30              # last 30 days instead of 7
sqz gain --days 30 --project .  # combine both
```

Las estadísticas se guardan localmente en SQLite, en `~/.sqz/sessions.db`: nada sale de tu equipo.

## Cómo funciona la compresión

1. **Formateadores por comando** para los comandos siguientes:

   | Ecosistema | Comandos |
   |---|---|
   | Git | status, log, diff, show, stash, remote, fetch, push, pull, add, commit |
   | Rust | cargo build/test/clippy/check/nextest |
   | JavaScript | npm/pnpm/yarn/bun install/audit/outdated (test y run cuando fallan), tsc, eslint, biome |
   | Python | pytest, ruff, mypy, pip |
   | Go | go test (incluido el flujo `-json`), go build, go vet, golangci-lint |
   | Nube | terraform plan/apply/destroy/refresh/init |
   | Contenedores | docker/podman ps/images, kubectl describe/apply |
   | JVM | gradle build/test, maven |
   | Móvil | xcodebuild (build + test), adb logcat |
   | .NET | dotnet build/publish/test |
   | Sistema | grep/rg, tree, curl/wget |

   Los demás comandos, incluidos kubectl get y logs, gh, aws, gcloud, ls y find, pasan por el pipeline de compresión genérico.

2. **Compactador de tablas**: la salida en columnas alineadas de las herramientas sin un formateador propio (`ps aux`, `netstat`, CLIs de bases de datos) reduce sus series de espacios de relleno a separadores de dos espacios. El detector es estricto: las líneas sangradas, el código, YAML y JSON nunca coinciden.
3. **Resúmenes estructurales**: el motor tiene una API que reduce un archivo de código a los imports, las firmas de las funciones y un grafo de llamadas. Las vías del hook y de MCP no la usan: sirven el código fuente completo.
4. **Caché de deduplicación**: hash SHA-256 del contenido, persistente entre sesiones. Salida idéntica byte a byte que el modelo ya tiene = referencia de 13 tokens. Un rango de líneas de un archivo que el modelo ya tiene completo (`sed -n '40,80p'`, `sqz_read_file` con `offset`/`limit`) = `§ref:HASH:L40-80§`. Una relectura tras una edición pequeña = las líneas modificadas con una línea sin cambios a cada lado, a través de las herramientas MCP de sqz (el hook de shell vuelve a enviar el archivo editado); si las ediciones están repartidas por todo el archivo, este vuelve completo. Las referencias solo se sirven mientras el original sigue en el contexto (ventana de 30 minutos, ajustable con `SQZ_REF_TTL_SECS` y reiniciada con la compactación).
5. **Pipeline de JSON**: eliminar nulos → descartar campos de control (`__v`, `etag`, ids de petición y de correlación) → aplanar → colapsar arrays → codificación TOON (formato compacto sin pérdida). Los campos cuyo valor es una cadena, un array o un objeto vacíos se descartan a cualquier profundidad, y las cadenas de más de 500 caracteres se recortan a sus primeros 500 caracteres seguidos de `...`.
6. **Modo seguro**: las trazas de pila (una cabecera de traza seguida de sus marcos), el DDL de SQL al inicio de una línea, los bloques PEM y las líneas con credenciales (`key=value` o `key: value` con una clave que parece secreta), con o sin colores ANSI, se saltan el pipeline y pasan sin cambios sea cual sea su tamaño, salvo que se eliminan los códigos ANSI, y nunca se guardan en caché. En el hook de shell, un formateador por comando se sigue ejecutando primero. Su resumen solo se usa cuando conserva sin cambios todas las líneas de cada párrafo que contiene alguna de esas estructuras, así que puede resumir el resto de una ejecución de pruebas alrededor de una traza; si no, toda la salida pasa sin cambios. La salida que solo menciona una contraseña, una clave, una traza o una migración fuera de esas estructuras se comprime como siempre, pero tampoco se guarda en caché, así que lo que una etapa con pérdida elimine de ella no se puede expandir.

Si una etapa entra en pánico, `sqz compress` imprime la entrada sin cambios con una línea en stderr y conserva el código de salida del comando, y el hook PostToolUse de Claude Code deja el resultado como estaba.

Calidad medida, incluido lo que descarta cada etapa y lo que nunca toca: [benchmark de calidad](../docs/quality-benchmark.md). Para ver todos los detalles técnicos, consulta [docs/](../docs/README.md).

## Configuración

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

### Base de datos por proyecto

Todo lo que sqz guarda (estadísticas, caché de deduplicación, sesiones) está en un único archivo SQLite, `~/.sqz/sessions.db` de forma predeterminada. Define `SQZ_DB_PATH` para tener uno por proyecto:

```sh
# e.g. in the project's .envrc (direnv)
export SQZ_DB_PATH="$PWD/.sqz/sessions.db"
```

Todas las interfaces la respetan: el hook de shell, `sqz stats`/`gain`/`expand` y el servidor MCP (defínela en `env` en tu configuración MCP). Es preferible usar rutas absolutas: los valores relativos se resuelven respecto al directorio desde el que se ejecute el proceso. Si el directorio padre no existe, se crea (modo 0700, como `~/.sqz`).

## Privacidad

- Cero telemetría: no se transmite ningún dato y no hay informes de fallos
- Totalmente sin conexión: funciona en entornos aislados de la red
- Todo el procesamiento es local

## Desarrollo

```sh
git clone https://github.com/ojuschugh1/sqz.git
cd sqz
cargo test --workspace
cargo build --release
```

## Licencia

[Elastic License 2.0](../LICENSE) (ELv2): puedes usarlo, hacer un fork y modificarlo libremente. Dos restricciones: no ofrecer un servicio alojado que compita con él y no eliminar los avisos de licencia.

## Enlaces

- [Índice de la documentación](../docs/README.md)
- [Cómo evitar que los agentes de programación con IA vuelvan a leer los mismos archivos](../docs/stop-rereading-files.md)
- [Compresión de contexto MCP](../docs/mcp-context-compression.md)
- [Benchmark de calidad: tasa de compresión frente a conservación de la información](../docs/quality-benchmark.md)
- [Degradación del contexto frente a compresión del contexto](../docs/context-rot.md)
- [Cómo elegir un enfoque de compresión de contexto](../docs/comparison.md)
- [Documento técnico: compresión de contexto previa a la inyección](../docs/whitepaper.md)
- [Benchmark de ahorro de tokens](../docs/benchmark.md)
- [Discord](https://discord.gg/j8EEyH5dSB)
- [Registro de cambios](../CHANGELOG.md)
- Nombre en el MCP Registry: `mcp-name: io.github.ojuschugh1/sqz`

## Historial de estrellas

Si sqz te ha ahorrado tokens hoy, una estrella ayuda a que otras personas lo encuentren. `sqz stats --share` imprime tus propias cifras si quieres mostrarlas.

<a href="https://star-history.dera.page/#ojuschugh1/sqz&Date">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://star-history.dera.page/svg?repos=ojuschugh1/sqz&type=Date&theme=dark" />
   <source media="(prefers-color-scheme: light)" srcset="https://star-history.dera.page/svg?repos=ojuschugh1/sqz&type=Date" />
   <img alt="Gráfico del historial de estrellas" src="https://star-history.dera.page/svg?repos=ojuschugh1/sqz&type=Date" width="600" />
 </picture>
</a>

## Contribuidores

Gracias a todas las personas que han aportado código, correcciones e ideas a sqz:

<a href="https://github.com/ojuschugh1/sqz/graphs/contributors">
  <img src="https://contrib.rocks/image?repo=ojuschugh1/sqz" alt="Contribuidores de sqz" />
</a>

Y a todas las personas que enviaron los informes de errores detallados que hay detrás de nuestras correcciones. Las reproducciones precisas mejoran este proyecto con cada versión. ¿Quieres sumarte? Los pull requests se revisan rápido: consulta los [issues abiertos](https://github.com/ojuschugh1/sqz/issues) para empezar.

<sub>Cuadrícula de contribuidores creada con [contrib.rocks](https://contrib.rocks).</sub>
