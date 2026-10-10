<!-- i18n-source: CONTRIBUTING.md sha256:da3c749928a7f68e597cc1f0a0eba90065241548812430032de3fcc69b9d7188 -->
<!-- i18n:languages -->
[English](../CONTRIBUTING.md) | [简体中文](CONTRIBUTING.zh-CN.md) | [日本語](CONTRIBUTING.ja.md) | [한국어](CONTRIBUTING.ko.md) | [Español](CONTRIBUTING.es.md) | [Português (Brasil)](CONTRIBUTING.pt-BR.md)
<!-- /i18n:languages -->

<!-- i18n:note -->
> Esta es una traducción del [CONTRIBUTING.md en inglés](../CONTRIBUTING.md).
<!-- /i18n:note -->

# Cómo contribuir a sqz

Gracias por tu interés en contribuir. Estos son los primeros pasos.

## Preparación

```sh
git clone https://github.com/ojuschugh1/sqz.git
cd sqz
cargo test --workspace
```

Si todas las pruebas pasan, ya puedes empezar.

## Compilación

El espacio de trabajo tiene cuatro crates:

| Crate | Qué es |
|---|---|
| `sqz_engine` | Biblioteca de compresión principal: toda la lógica está aquí |
| `sqz` | Binario de la CLI (`sqz compress`, `sqz init`, etc.) |
| `sqz-mcp` | Servidor MCP (transporte stdio/SSE) |
| `sqz-wasm` | Compilación WASM para las extensiones de navegador |

Compila todo:

```sh
cargo build --workspace
```

Compila un solo crate:

```sh
cargo build -p sqz-engine
```

Genera la documentación:

```sh
cargo doc --workspace --no-deps --open
```

## Ejecución de las pruebas

Ejecuta todas las pruebas de todos los crates:

```sh
cargo test --workspace
```

Ejecuta las pruebas de un solo crate:

```sh
cargo test -p sqz-engine
```

Ejecuta una prueba concreta por su nombre:

```sh
cargo test -p sqz-engine test_compress_json_applies_toon
```

Ejecuta solo las pruebas basadas en propiedades (proptest):

```sh
cargo test -p sqz-engine prop_
```

## Estilo de código

- Ejecuta `cargo fmt` antes de hacer commit: formato estándar de Rust, sin reglas personalizadas
- Ejecuta `cargo clippy --workspace`: no se permiten advertencias
- Sigue los patrones que veas en el código existente. Si el archivo usa `thiserror`, usa `thiserror`. Si usa `serde`, usa `serde`. No introduzcas bibliotecas nuevas para cosas que ya resolvemos
- Comentarios de documentación en todos los elementos públicos. Que sean breves y prácticos: di qué hace la función, no por qué es increíble
- Usa comentarios de documentación `///` con ejemplos de código donde ayuden. Los ejemplos deben compilar (`cargo test --doc`)

## Añadir una nueva etapa de compresión

1. Crea el struct de tu etapa en `sqz_engine/src/stages.rs` (o en un archivo nuevo si es compleja)
2. Implementa el trait `CompressionStage`:
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
3. Añádela al pipeline en `sqz_engine/src/pipeline.rs`, tanto en `new()` como en `reload_preset()`
4. Añade los campos de configuración en `sqz_engine/src/preset.rs` (struct + deserialización TOML)
5. Conecta la configuración en `stage_config_from_preset()`, en `pipeline.rs`
6. Escribe pruebas basadas en propiedades (proptest) que validen los invariantes de la etapa
7. Actualiza el preset predeterminado en `Preset::default()` si la etapa debe estar activada por defecto

## Añadir un nuevo comando a la CLI

1. Añade una variante al enum `Command` en `sqz/src/main.rs`
2. Añade en `main()` el brazo del match que llama a tu función `cmd_*`
3. Escribe la función `cmd_*` siguiendo el patrón de los comandos existentes
4. Usa `require_engine()` para obtener una instancia de `SqzEngine`
5. Imprime la salida para el usuario en stdout y los diagnósticos en stderr (con el prefijo `[sqz]`)

## Reglas de arquitectura

- Toda la lógica de compresión está en `sqz_engine`. La CLI, el servidor MCP y el crate WASM son adaptadores ligeros: no deben contener lógica de compresión
- El motor principal no hace peticiones de red
- Sin telemetría, sin analítica, sin informes de fallos
- Pruebas basadas en propiedades (proptest) para todas las propiedades de corrección
- `cargo check --workspace` debe pasar antes de hacer commit

## Proceso de pull request

1. Haz un fork del repositorio y crea una rama a partir de `main`
2. Haz tus cambios
3. Ejecuta la comprobación completa:
   ```sh
   cargo fmt --check
   cargo clippy --workspace
   cargo test --workspace
   cargo doc --workspace --no-deps
   ```
4. Escribe un mensaje de commit claro que explique qué cambia y por qué
5. Envía un pull request contra `main`
6. Atiende los comentarios de la revisión

## Informar de problemas

- Usa GitHub Issues
- Incluye: sistema operativo, versión de sqz (`sqz --version`), pasos para reproducirlo, comportamiento esperado frente al real
- Para fallos en las pruebas: incluye la salida completa de `cargo test`
- Los issues y los pull requests se pueden escribir en inglés o en tu propio idioma. En inglés la respuesta llega antes

## Traducciones

Las traducciones del README y de esta guía están en `i18n/`. Consulta [i18n/README.md](README.md) para ver las reglas, cómo actualizar una traducción y cómo añadir un idioma.

## CLA

Al enviar un pull request, aceptas el [Acuerdo de Licencia de Contribuidor (CLA)](../CLA.md). El CLA solo está disponible en inglés, y el texto en inglés es el que se aplica.
