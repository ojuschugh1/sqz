<!-- i18n-source: CONTRIBUTING.md sha256:da3c749928a7f68e597cc1f0a0eba90065241548812430032de3fcc69b9d7188 -->
<!-- i18n:languages -->
[English](../CONTRIBUTING.md) | [简体中文](CONTRIBUTING.zh-CN.md) | [日本語](CONTRIBUTING.ja.md) | [한국어](CONTRIBUTING.ko.md) | [Español](CONTRIBUTING.es.md) | [Português (Brasil)](CONTRIBUTING.pt-BR.md)
<!-- /i18n:languages -->

<!-- i18n:note -->
> 이 문서는 영어 원문 [CONTRIBUTING.md](../CONTRIBUTING.md)를 번역한 것입니다.
<!-- /i18n:note -->

# sqz에 기여하기

기여에 관심을 가져 주셔서 감사합니다. 시작하는 방법은 다음과 같습니다.

## 환경 설정

```sh
git clone https://github.com/ojuschugh1/sqz.git
cd sqz
cargo test --workspace
```

모든 테스트가 통과하면 준비가 끝난 것입니다.

## 빌드

워크스페이스에는 크레이트가 네 개 있습니다.

| 크레이트 | 설명 |
|---|---|
| `sqz_engine` | 핵심 압축 라이브러리이며, 모든 로직이 여기에 있습니다 |
| `sqz` | CLI 바이너리(`sqz compress`, `sqz init` 등) |
| `sqz-mcp` | MCP 서버(stdio/SSE 전송) |
| `sqz-wasm` | 브라우저 확장 프로그램용 WASM 빌드 |

전체를 빌드합니다.

```sh
cargo build --workspace
```

단일 크레이트를 빌드합니다.

```sh
cargo build -p sqz-engine
```

문서를 생성합니다.

```sh
cargo doc --workspace --no-deps --open
```

## 테스트 실행

모든 크레이트의 테스트를 전부 실행합니다.

```sh
cargo test --workspace
```

단일 크레이트의 테스트를 실행합니다.

```sh
cargo test -p sqz-engine
```

이름으로 특정 테스트를 실행합니다.

```sh
cargo test -p sqz-engine test_compress_json_applies_toon
```

속성 기반 테스트(proptest)만 실행합니다.

```sh
cargo test -p sqz-engine prop_
```

## 코드 스타일

- 커밋하기 전에 `cargo fmt`를 실행하십시오. 표준 Rust 포맷을 따르며 별도 규칙은 없습니다
- `cargo clippy --workspace`를 실행하십시오. 경고는 하나도 허용되지 않습니다
- 기존 코드에 보이는 패턴을 따르십시오. 파일이 `thiserror`를 사용하면 `thiserror`를 사용하고, `serde`를 사용하면 `serde`를 사용하십시오. 이미 처리하고 있는 일에 새 라이브러리를 들여오지 마십시오
- 모든 공개 항목에 문서 주석을 다십시오. 짧고 실용적으로 쓰십시오. 함수가 무엇을 하는지를 쓰고, 왜 대단한지는 쓰지 마십시오
- 도움이 되는 곳에서는 코드 예제가 포함된 `///` 문서 주석을 사용하십시오. 예제는 컴파일되어야 합니다(`cargo test --doc`)

## 새 압축 단계 추가하기

1. `sqz_engine/src/stages.rs`(복잡하다면 새 파일)에 단계 구조체를 만듭니다
2. `CompressionStage` 트레이트를 구현합니다.
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
3. `sqz_engine/src/pipeline.rs`의 파이프라인에 추가합니다. `new()`와 `reload_preset()` 양쪽에 모두 추가하십시오
4. `sqz_engine/src/preset.rs`에 설정 필드를 추가합니다(구조체 + TOML 역직렬화)
5. `pipeline.rs`의 `stage_config_from_preset()`에서 설정을 연결합니다
6. 단계의 불변 조건을 검증하는 속성 기반 테스트(proptest)를 작성합니다
7. 기본으로 켜져 있어야 하는 단계라면 `Preset::default()`의 기본 프리셋을 업데이트합니다

## 새 CLI 명령 추가하기

1. `sqz/src/main.rs`의 `Command` 열거형에 배리언트를 추가합니다
2. `main()`에 작성할 `cmd_*` 함수로 디스패치하는 match 갈래를 추가합니다
3. `cmd_*` 함수를 작성합니다. 기존 명령의 패턴을 따르십시오
4. `require_engine()`으로 `SqzEngine` 인스턴스를 얻습니다
5. 사용자에게 보여 주는 출력은 stdout으로, 진단 메시지는 stderr로(앞에 `[sqz]`를 붙여) 출력합니다

## 아키텍처 규칙

- 모든 압축 로직은 `sqz_engine`에 있습니다. CLI, MCP 서버, WASM 크레이트는 얇은 어댑터이며 압축 로직을 포함하지 않아야 합니다
- 핵심 엔진은 네트워크 요청을 보내지 않습니다
- 텔레메트리, 분석, 충돌 보고서는 없습니다
- 모든 정확성 속성에는 속성 기반 테스트(proptest)를 작성합니다
- 커밋하기 전에 `cargo check --workspace`가 통과해야 합니다

## PR 절차

1. 저장소를 포크하고 `main`에서 브랜치를 만듭니다
2. 변경 사항을 만듭니다
3. 전체 검사를 실행합니다.
   ```sh
   cargo fmt --check
   cargo clippy --workspace
   cargo test --workspace
   cargo doc --workspace --no-deps
   ```
4. 무엇을 왜 바꿨는지 설명하는 명확한 커밋 메시지를 작성합니다
5. `main`을 대상으로 풀 리퀘스트를 제출합니다
6. 리뷰 피드백을 반영합니다

## 이슈 보고

- GitHub Issues를 사용하십시오
- 다음을 포함하십시오: OS, sqz 버전(`sqz --version`), 재현 단계, 예상 동작과 실제 동작
- 테스트가 실패하는 경우: `cargo test` 출력 전체를 포함하십시오
- 이슈와 풀 리퀘스트는 영어나 자신의 언어로 작성할 수 있습니다. 영어로 작성하면 가장 빨리 답변을 받을 수 있습니다

## 번역

README와 이 가이드의 번역은 `i18n/`에 있습니다. 규칙, 번역을 업데이트하는 방법, 언어를 추가하는 방법은 [i18n/README.md](README.md)를 참고하십시오.

## CLA

풀 리퀘스트를 제출하면 [기여자 라이선스 계약(CLA)](../CLA.md)에 동의하는 것으로 간주됩니다. CLA는 영어로만 제공되며, 적용되는 것은 영어 원문입니다.
