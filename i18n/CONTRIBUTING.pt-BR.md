<!-- i18n-source: CONTRIBUTING.md sha256:da3c749928a7f68e597cc1f0a0eba90065241548812430032de3fcc69b9d7188 -->
<!-- i18n:languages -->
[English](../CONTRIBUTING.md) | [Português (Brasil)](CONTRIBUTING.pt-BR.md)
<!-- /i18n:languages -->

<!-- i18n:note -->
> Esta é uma tradução do [CONTRIBUTING.md em inglês](../CONTRIBUTING.md).
<!-- /i18n:note -->

# Como contribuir com o sqz

Obrigado pelo interesse em contribuir. Veja como começar.

## Preparação do ambiente

```sh
git clone https://github.com/ojuschugh1/sqz.git
cd sqz
cargo test --workspace
```

Se todos os testes passarem, está tudo pronto.

## Compilação

O workspace tem quatro crates:

| Crate | O que é |
|---|---|
| `sqz_engine` | Biblioteca central de compressão: toda a lógica fica aqui |
| `sqz` | Binário da CLI (`sqz compress`, `sqz init` etc.) |
| `sqz-mcp` | Servidor MCP (transporte stdio/SSE) |
| `sqz-wasm` | Build WASM para extensões de navegador |

Compile tudo:

```sh
cargo build --workspace
```

Compile um único crate:

```sh
cargo build -p sqz-engine
```

Gere a documentação:

```sh
cargo doc --workspace --no-deps --open
```

## Como rodar os testes

Rode todos os testes de todos os crates:

```sh
cargo test --workspace
```

Rode os testes de um único crate:

```sh
cargo test -p sqz-engine
```

Rode um teste específico pelo nome:

```sh
cargo test -p sqz-engine test_compress_json_applies_toon
```

Rode só os testes baseados em propriedades (proptest):

```sh
cargo test -p sqz-engine prop_
```

## Estilo de código

- Rode `cargo fmt` antes de fazer commit: formatação padrão do Rust, sem regras personalizadas
- Rode `cargo clippy --workspace`: nenhum aviso é permitido
- Siga os padrões que você vê no código existente. Se o arquivo usa `thiserror`, use `thiserror`. Se usa `serde`, use `serde`. Não introduza bibliotecas novas para coisas que já resolvemos
- Comentários de documentação em todos os itens públicos. Mantenha-os curtos e práticos: diga o que a função faz, não por que ela é incrível
- Use comentários de documentação `///` com exemplos de código quando ajudar. Os exemplos precisam compilar (`cargo test --doc`)

## Como adicionar uma nova etapa de compressão

1. Crie a struct da sua etapa em `sqz_engine/src/stages.rs` (ou em um arquivo novo, se ela for complexa)
2. Implemente o trait `CompressionStage`:
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
3. Adicione a etapa ao pipeline em `sqz_engine/src/pipeline.rs`, tanto em `new()` quanto em `reload_preset()`
4. Adicione os campos de configuração em `sqz_engine/src/preset.rs` (struct + desserialização TOML)
5. Conecte a configuração em `stage_config_from_preset()`, no `pipeline.rs`
6. Escreva testes baseados em propriedades (proptest) que validem os invariantes da etapa
7. Atualize o preset padrão em `Preset::default()` se a etapa tiver de vir ativada por padrão

## Como adicionar um novo comando à CLI

1. Adicione uma variante ao enum `Command` em `sqz/src/main.rs`
2. Adicione em `main()` o braço do match que despacha para a sua função `cmd_*`
3. Escreva a função `cmd_*` seguindo o padrão dos comandos existentes
4. Use `require_engine()` para obter uma instância de `SqzEngine`
5. Imprima a saída voltada ao usuário no stdout e os diagnósticos no stderr (com o prefixo `[sqz]`)

## Regras de arquitetura

- Toda a lógica de compressão fica em `sqz_engine`. A CLI, o servidor MCP e o crate WASM são adaptadores finos: eles não devem conter lógica de compressão
- Nenhuma requisição de rede a partir do motor central
- Sem telemetria, sem análise de uso, sem relatórios de falhas
- Testes baseados em propriedades (proptest) para todas as propriedades de corretude
- `cargo check --workspace` precisa passar antes de fazer commit

## Processo de PR

1. Faça um fork do repositório e crie uma branch a partir de `main`
2. Faça as suas alterações
3. Rode a verificação completa:
   ```sh
   cargo fmt --check
   cargo clippy --workspace
   cargo test --workspace
   cargo doc --workspace --no-deps
   ```
4. Escreva uma mensagem de commit clara, explicando o quê e o porquê
5. Envie um pull request para `main`
6. Atenda aos comentários da revisão

## Como abrir issues

- Use o GitHub Issues
- Inclua: sistema operacional, versão do sqz (`sqz --version`), passos para reproduzir, comportamento esperado versus comportamento real
- Para falhas de teste: inclua a saída completa do `cargo test`
- Issues e pull requests podem ser escritos em inglês ou no seu próprio idioma. Os escritos em inglês recebem a resposta mais rápida

## Traduções

O README e este guia têm traduções em `i18n/`. Veja o [i18n/README.md](README.md) para as regras, como atualizar uma tradução e como adicionar um idioma.

## CLA

Ao enviar um pull request, você concorda com o [Contrato de Licença de Contribuidor (CLA)](../CLA.md). O CLA só está disponível em inglês, e o texto em inglês é o que vale.
