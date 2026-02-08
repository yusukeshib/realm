# realm

[English](README.md)

[![Crates.io](https://img.shields.io/crates/v/realm-cli)](https://crates.io/crates/realm-cli)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![CI](https://github.com/yusukeshib/realm/actions/workflows/ci.yml/badge.svg)](https://github.com/yusukeshib/realm/actions/workflows/ci.yml)

gitリポジトリ用のサンドボックスDocker環境 — AIコーディングエージェントのための安全な実験場。

![demo](./docs/demo.gif)

## なぜ realm？

AIコーディングエージェント（Claude Code、Cursor、Copilot）は強力ですが、実際の作業ツリーで自由に動かすのはリスクがあります。Realmは**安全で隔離されたサンドボックス**を提供し、エージェントが影響を気にせず自由に実験できる環境を作ります。

- **コードの安全性** — 独立したクローンを使用し、ホストのファイルは一切変更されません
- **AIエージェントが自由に実験可能** — コミット、ブランチ作成、書き換え、破壊 — 作業ツリーには影響なし
- **永続的なセッション** — 終了しても再開時にそのまま続行、ファイルは保持されます
- **名前付きセッション** — 複数の実験を並行して実行可能
- **自分のツールチェーンを使用** — 任意のDockerイメージで動作

## インストール

### クイックインストール

```bash
curl -fsSL https://raw.githubusercontent.com/yusukeshib/realm/main/install.sh | bash
```

### crates.ioから

```bash
cargo install realm-cli
```

### ソースから

```bash
cargo install --git https://github.com/yusukeshib/realm
```

### Nix

```bash
nix run github:yusukeshib/realm
```

### バイナリダウンロード

ビルド済みバイナリは[GitHub Releases](https://github.com/yusukeshib/realm/releases)ページからダウンロードできます。

## クイックスタート

```bash
curl -fsSL https://raw.githubusercontent.com/yusukeshib/realm/main/install.sh | bash
realm my-feature --image ubuntu:latest -- bash
# gitアクセス可能な隔離コンテナの中にいます
```

Realmはgitリポジトリ内で実行する必要があります — 現在のリポジトリをコンテナ内にクローンします。

フラグ不要のワークフローについては、下記の[カスタムイメージのセットアップ](#カスタムイメージのセットアップ)を参照してください。

## カスタムイメージのセットアップ

Realmの推奨ワークフロー：イメージを一度ビルドし、環境変数を設定するだけ。以降はフラグなしで使えます。

**1. ツールチェーンを含むDockerfileを作成**

ワークフローに必要なツール（言語、ランタイム、CLIツールなど）をすべて含めます。

**2. イメージをビルド**

```bash
docker build -t mydev .
```

**3. 対話的に実行して認証・設定を行う**

Claude Codeなど一部のツールは、Dockerfileに組み込めない対話的な認証が必要です。イメージを実行してセットアップを完了し、その状態を保存します：

```bash
docker run -it mydev
# コンテナ内で: `claude` を実行して認証、追加ツールのインストールなど
# 完了したら:
exit
```

```bash
# コンテナIDを確認し、設定済みの状態をコミット
docker commit <container_id> mydev:ready
```

**4. 環境変数を設定**

`.zshrc` または `.bashrc` に以下を追加：

```bash
export REALM_DEFAULT_IMAGE=mydev:ready        # カスタムイメージ
export REALM_DOCKER_ARGS="--network host"     # 常に使いたいDockerフラグ
```

**5. 完了 — あとは realm を使うだけ**

環境変数を設定すれば、すべてのセッションがフラグなしでカスタムイメージを使用します：

```bash
# これだけです。以降は:
realm feature-1
realm bugfix-auth
realm experiment-v2
# それぞれが完全なツールチェーンを持つ隔離サンドボックスになります。
```

## 使い方

```bash
realm                                               全セッション一覧（TUI）
realm <name> [options] [-- cmd...]                  セッションの作成または再開
realm <name> -d                                     セッションの削除
realm upgrade                                       最新版にアップグレード
```

### セッションの作成または再開

```bash
# デフォルト: alpine/git イメージ、sh シェル、カレントディレクトリ
realm my-feature

# プロジェクトディレクトリを指定（作成時のみ有効）
realm my-feature --dir ~/projects/my-app

# カスタムイメージでbashを使用（作成時のみ有効）
realm my-feature --image ubuntu:latest -- bash

# コンテナ内のマウントパスを指定（作成時のみ有効）
realm my-feature --mount /src

# 環境変数（作成時のみ有効）
realm my-feature -e KEY=VALUE -e ANOTHER_KEY

# セッションが存在すれば元の設定で再開
# 存在しなければ新規作成
realm my-feature
```

セッションが存在しない場合は自動的に作成されます。既存のセッションを再開する場合、`--image`、`--mount`、`--dir`、`-e` などの作成時オプションは無視されます。

### セッション一覧

```bash
realm
```

```
NAME                 PROJECT                        IMAGE                CREATED
----                 -------                        -----                -------
my-feature           /Users/you/projects/app        alpine/git           2026-02-07 12:00:00 UTC
test                 /Users/you/projects/other      ubuntu:latest        2026-02-07 12:30:00 UTC
```

### セッションの削除

```bash
realm my-feature -d
```

## オプション

| オプション | 説明 |
|--------|-------------|
| `-d` | セッションを削除 |
| `--image <image>` | 使用するDockerイメージ（デフォルト: `alpine/git`）- 作成時のみ有効 |
| `--mount <path>` | コンテナ内のマウントパス（デフォルト: `/workspace/<dir-name>`）- 作成時のみ有効 |
| `--dir <path>` | プロジェクトディレクトリ（デフォルト: カレントディレクトリ）- 作成時のみ有効 |
| `-e, --env <KEY[=VALUE]>` | コンテナに渡す環境変数 - 作成時のみ有効 |
| `--no-ssh` | SSHエージェント転送を無効化（デフォルトは有効）- 作成時のみ有効 |

## 環境変数

CLIフラグを完全に省略するためのデフォルト設定です。`.zshrc` や `.bashrc` に設定すれば、`realm <name>` を実行するだけで自動的に適用されます。

| 変数 | 説明 |
|----------|-------------|
| `REALM_DEFAULT_IMAGE` | 新規セッションのデフォルトDockerイメージ（デフォルト: `alpine/git`） |
| `REALM_DOCKER_ARGS` | 追加のDockerフラグ（例: `--network host`、追加の `-v` マウント） |

```bash
# 例: 常にホストネットワークとデータボリュームを使用
REALM_DOCKER_ARGS="--network host -v /data:/data:ro" realm my-session
```

## 仕組み

初回実行時、`git clone --local` でリポジトリの独立したコピーをワークスペースディレクトリに作成します。コンテナは完全に自己完結したgitリポジトリを取得します — 特別なマウントやentrypointスクリプトは不要です。ホストの作業ディレクトリは一切変更されません。

- **独立したクローン** — 各セッションは `git clone --local` による完全なgitリポジトリを持ちます
- **永続的なワークスペース** — `exit` してもファイルは保持され、`realm <name>` で再開可能。`realm <name> -d` でクリーンアップ
- **任意のイメージ・ユーザー** — rootおよび非rootコンテナイメージで動作

| 観点 | 保護 |
|--------|------------|
| ホスト作業ツリー | 変更されない — ワークスペースは独立したクローン |
| ワークスペース | `~/.realm/workspaces/<name>/` からバインドマウント、停止・起動をまたいで永続化 |
| セッションクリーンアップ | `realm <name> -d` でコンテナ、ワークスペース、セッションデータを削除 |

## SSHエージェント転送

**課題**: Dockerコンテナは通常、ホストのSSH鍵にアクセスできません。macOSではさらに困難です — DockerがVM内で動作するため、UnixソケットがVM境界を越えられません。

**realmの解決策**: ホストのSSHエージェントをコンテナに自動転送します。`git push`、`git pull`、`ssh` がすべて既存の鍵で動作します — 鍵のコピーは不要です。

**プラットフォーム別の仕組み**:

- **macOS**（Docker Desktop / OrbStack）: VM経由のソケット `/run/host-services/ssh-auth.sock` をマウント
- **Linux**: `$SSH_AUTH_SOCK` を直接コンテナにマウント

Realmはクローンしたリポジトリの `origin` リモートを実際のURL（ローカルクローンパスではなく）に自動修正するため、`git push origin` がそのまま動作します。

```bash
realm my-feature --image ubuntu:latest -- bash

# コンテナ内で
ssh-add -l          # 鍵の一覧が表示されるはずです
git push origin main

# SSH転送を無効にする場合
realm my-feature --no-ssh -- bash
```

## Claude Code連携

Realmは[Claude Code](https://docs.anthropic.com/en/docs/claude-code)の理想的なパートナーです。Realmセッション内でClaude Codeを実行すれば、リスクのある変更、ブランチの実験、テストの実行 — すべてホストから完全に隔離された環境で行えます。

```bash
realm ai-experiment --image node:20 -- claude
```

エージェントが行うすべての操作はコンテナ内に留まります。完了したらセッションを削除すれば消えます。

## ライセンス

MIT
