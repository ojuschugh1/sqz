# JetBrains Marketplace Publishing Guide

The sqz JetBrains plugin is published to the [JetBrains Marketplace](https://plugins.jetbrains.com/) and works with IntelliJ IDEA, PyCharm, GoLand, and all other IntelliJ-platform IDEs.

## Prerequisites

- JDK 17+
- Gradle (wrapper included in `jetbrains-plugin/`)
- A JetBrains Marketplace account and plugin token

## Required `plugin.xml` fields

The following fields in `src/main/resources/META-INF/plugin.xml` must be set before publishing:

```xml
<idea-plugin>
  <id>com.sqz.jetbrains-plugin</id>          <!-- unique, never change after first publish -->
  <name>sqz — Context Intelligence</name>
  <version>0.1.0</version>
  <vendor email="sqz@example.com"
          url="https://github.com/ojuschugh1/sqz">sqz</vendor>
  <description><![CDATA[...]]></description>
  <change-notes><![CDATA[Initial release.]]></change-notes>
  <idea-version since-build="233"/>           <!-- minimum IDE build number -->
  <depends>com.intellij.modules.platform</depends>
</idea-plugin>
```

Key rules:
- `<id>` must be globally unique and must not change after the first publish
- `<version>` must be bumped for each publish (semver)
- `<idea-version since-build>` controls the minimum compatible IDE version
- `<change-notes>` is shown in the Marketplace changelog tab

## Build the plugin

```sh
cd jetbrains-plugin
./gradlew buildPlugin
# produces build/distributions/sqz-0.1.0.zip
```

Run the plugin in a sandboxed IDE to verify before publishing:

```sh
./gradlew runIde
```

## Publish

Set your Marketplace token as an environment variable:

```sh
export JETBRAINS_TOKEN=<your-token>
./gradlew publishPlugin
```

Or pass it directly:

```sh
./gradlew publishPlugin -Pplugin.verifier.home.dir=/tmp/verifier \
  -Dorg.gradle.project.intellijPublishToken=$JETBRAINS_TOKEN
```

The `publishPlugin` task in `build.gradle.kts` should be configured as:

```kotlin
publishPlugin {
    token.set(System.getenv("JETBRAINS_TOKEN"))
    channels.set(listOf("default"))   // use "eap" for pre-release
}
```

## CI publishing (GitHub Actions)

```yaml
- name: Publish to JetBrains Marketplace
  run: ./gradlew publishPlugin
  working-directory: jetbrains-plugin
  env:
    JETBRAINS_TOKEN: ${{ secrets.JETBRAINS_TOKEN }}
```

## Verify

After publishing, the plugin appears at:
`https://plugins.jetbrains.com/plugin/<plugin-id>`

Users can install from within any JetBrains IDE via **Settings → Plugins → Marketplace → search "sqz"**.
