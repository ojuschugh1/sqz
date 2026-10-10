<!-- i18n-source: CONTRIBUTING.md sha256:da3c749928a7f68e597cc1f0a0eba90065241548812430032de3fcc69b9d7188 -->
<!-- i18n:languages -->
[English](../CONTRIBUTING.md) | [日本語](CONTRIBUTING.ja.md)
<!-- /i18n:languages -->

<!-- i18n:note -->
> この文書は英語版の[CONTRIBUTING.md](../CONTRIBUTING.md)を翻訳したものです。
<!-- /i18n:note -->

# sqzへのコントリビュート

コントリビュートに関心を持っていただき、ありがとうございます。始め方は以下のとおりです。

## セットアップ

```sh
git clone https://github.com/ojuschugh1/sqz.git
cd sqz
cargo test --workspace
```

すべてのテストが通れば、準備は完了です。

## ビルド

ワークスペースには四つのクレートがあります：

| クレート | 内容 |
|---|---|
| `sqz_engine` | 圧縮のコアライブラリ。すべてのロジックはここにあります |
| `sqz` | CLIバイナリ（`sqz compress`、`sqz init`など） |
| `sqz-mcp` | MCPサーバー（stdio/SSEトランスポート） |
| `sqz-wasm` | ブラウザー拡張機能向けのWASMビルド |

すべてをビルドします：

```sh
cargo build --workspace
```

単一のクレートをビルドします：

```sh
cargo build -p sqz-engine
```

ドキュメントを生成します：

```sh
cargo doc --workspace --no-deps --open
```

## テストの実行

すべてのクレートのテストをすべて実行します：

```sh
cargo test --workspace
```

単一のクレートのテストを実行します：

```sh
cargo test -p sqz-engine
```

名前を指定して特定のテストを実行します：

```sh
cargo test -p sqz-engine test_compress_json_applies_toon
```

プロパティベーステスト（proptest）だけを実行します：

```sh
cargo test -p sqz-engine prop_
```

## コードスタイル

- コミットする前に`cargo fmt`を実行してください。Rustの標準のフォーマットで、独自のルールはありません
- `cargo clippy --workspace`を実行してください。警告は一つも許されません
- 既存のコードに見られるパターンに合わせてください。そのファイルが`thiserror`を使っていれば`thiserror`を、`serde`を使っていれば`serde`を使ってください。すでに対応できていることのために、新しいライブラリを持ち込まないでください
- すべての公開アイテムにドキュメントコメントを付けてください。短く実用的に書いてください。書くのはその関数が何をするかで、それがなぜ素晴らしいかではありません
- 役に立つ場合は、コード例を含む`///`ドキュメントコメントを使ってください。例はコンパイルできなければなりません（`cargo test --doc`）

## 新しい圧縮ステージの追加

1. `sqz_engine/src/stages.rs`（複雑な場合は新しいファイル）にステージの構造体を作成します
2. `CompressionStage`トレイトを実装します：
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
3. `sqz_engine/src/pipeline.rs`でパイプラインに追加します。`new()`と`reload_preset()`の両方に追加してください
4. `sqz_engine/src/preset.rs`に設定のフィールドを追加します（構造体 + TOMLのデシリアライズ）
5. `pipeline.rs`の`stage_config_from_preset()`で設定をつなぎ込みます
6. ステージの不変条件を検証するプロパティベーステスト（proptest）を書きます
7. ステージをデフォルトで有効にする場合は、`Preset::default()`のデフォルトのプリセットを更新します

## 新しいCLIコマンドの追加

1. `sqz/src/main.rs`の`Command` enumにバリアントを追加します
2. `main()`に、作成する`cmd_*`関数へディスパッチするmatchアームを追加します
3. `cmd_*`関数を書きます。既存のコマンドのパターンに従ってください
4. `require_engine()`を使って`SqzEngine`のインスタンスを取得します
5. ユーザー向けの出力はstdoutに、診断メッセージはstderrに（先頭に`[sqz]`を付けて）出力します

## アーキテクチャのルール

- 圧縮のロジックはすべて`sqz_engine`に置きます。CLI、MCPサーバー、WASMクレートは薄いアダプターであり、圧縮のロジックを含めるべきではありません
- コアエンジンからはネットワークリクエストを送りません
- テレメトリー、アナリティクス、クラッシュレポートは一切入れません
- 正しさに関わるすべての性質に、プロパティベーステスト（proptest）を用意します
- コミットする前に`cargo check --workspace`が通る必要があります

## プルリクエストの手順

1. リポジトリをフォークし、`main`からブランチを作成します
2. 変更を加えます
3. すべてのチェックを実行します：
   ```sh
   cargo fmt --check
   cargo clippy --workspace
   cargo test --workspace
   cargo doc --workspace --no-deps
   ```
4. 何を、なぜ変更したのかを説明する明確なコミットメッセージを書きます
5. `main`に向けてプルリクエストを送ります
6. レビューでのフィードバックに対応します

## Issueの報告

- GitHub Issuesを使ってください
- 次の情報を含めてください：OS、sqzのバージョン（`sqz --version`）、再現手順、期待される動作と実際の動作
- テストが失敗する場合は、`cargo test`の出力全体を含めてください
- Issueとプルリクエストは、英語でも自分の言語でも書けます。英語で書くと、最も早く返信が届きます

## 翻訳

READMEとこのガイドの翻訳は`i18n/`にあります。ルール、翻訳の更新方法、言語の追加方法については[i18n/README.md](README.md)をご覧ください。

## CLA

プルリクエストを送ると、[コントリビューターライセンス契約（CLA）](../CLA.md)に同意したことになります。CLAは英語版のみで、適用されるのは英語の文面です。
