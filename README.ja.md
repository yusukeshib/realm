# realm

[English](README.md)

[![Crates.io](https://img.shields.io/crates/v/realm-cli)](https://crates.io/crates/realm-cli)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![CI](https://github.com/yusukeshib/realm/actions/workflows/ci.yml/badge.svg)](https://github.com/yusukeshib/realm/actions/workflows/ci.yml)

gitリポジトリ用のサンドボックスDocker環境 — AIコーディングエージェントのための安全な実験場。

![demo](./demo.gif)

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
realm                                               セッションマネージャー（TUI）
realm <name> [options] [-- cmd...]                  セッションの作成または再開
realm <name> -d [-- cmd...]                         バックグラウンドで実行（デタッチ）
realm <name>                                        実行中のセッションにアタッチ
realm upgrade                                       最新版にアップグレード
```

### セッションマネージャー

引数なしで `realm` を実行すると、対話型TUIが開きます：

```
 NAME            STATUS   PROJECT                   IMAGE            CREATED
> New realm...
  my-feature     running  /Users/you/projects/app   alpine:latest    2026-02-07 12:00:00 UTC
  test                    /Users/you/projects/other  ubuntu:latest   2026-02-07 12:30:00 UTC

 [Enter] Resume  [d] Delete  [q] Quit
```

- **Enter** でセッションを再開、または「New realm...」で新規作成
- **d** でハイライト中のセッションを削除（確認あり）
- **q** / **Esc** で終了

### セッションの作成または再開

```bash
# デフォルト: alpine:latest イメージ、sh シェル、カレントディレクトリ
realm my-feature

# カスタムイメージでbashを使用（作成時のみ有効）
realm my-feature --image ubuntu:latest -- bash

# 追加のDockerフラグ（環境変数、ボリューム、ネットワークなど）
realm my-feature --docker-args "-e KEY=VALUE -v /host:/container --network host"

# セッションが存在すれば元の設定で再開
# 存在しなければ新規作成
realm my-feature
```

セッションが存在しない場合は自動的に作成されます。既存のセッションを再開する場合、`--image` などの作成時オプションは無視されます。`--docker-args` や `--no-ssh` などのランタイムオプションは毎回適用されます。

### デタッチモード

```bash
# バックグラウンドで実行
realm my-feature -d -- claude -p "do something"

# 実行中のセッションにアタッチ
realm my-feature

# 停止せずにデタッチ: Ctrl+P, Ctrl+Q
```

## オプション

| オプション | 説明 |
|--------|-------------|
| `-d` | バックグラウンドでコンテナを実行（デタッチ） |
| `--image <image>` | 使用するDockerイメージ（デフォルト: `alpine:latest`）- 作成時のみ有効 |
| `--docker-args <args>` | 追加のDockerフラグ（例: `-e KEY=VALUE`、`-v /host:/container`）。`$REALM_DOCKER_ARGS` を上書き |
| `--no-ssh` | SSHエージェント転送を無効化（デフォルトは有効） |

## 環境変数

CLIフラグを完全に省略するためのデフォルト設定です。`.zshrc` や `.bashrc` に設定すれば、`realm <name>` を実行するだけで自動的に適用されます。

| 変数 | 説明 |
|----------|-------------|
| `REALM_DEFAULT_IMAGE` | 新規セッションのデフォルトDockerイメージ（デフォルト: `alpine:latest`） |
| `REALM_DOCKER_ARGS` | デフォルトの追加Dockerフラグ。`--docker-args` が指定されていない場合に使用 |

```bash
# 全セッションにデフォルトのDockerフラグを設定
export REALM_DOCKER_ARGS="--network host -v /data:/data:ro"
realm my-session

# 特定のセッションで --docker-args で上書き
realm my-session --docker-args "-e DEBUG=1"
```

## 仕組み

初回実行時、`git clone --local` でリポジトリの独立したコピーをワークスペースディレクトリに作成します。コンテナは完全に自己完結したgitリポジトリを取得します — 特別なマウントやentrypointスクリプトは不要です。ホストの作業ディレクトリは一切変更されません。

- **独立したクローン** — 各セッションは `git clone --local` による完全なgitリポジトリを持ちます
- **永続的なワークスペース** — `exit` してもファイルは保持され、`realm <name>` で再開可能。セッションマネージャーから削除でクリーンアップ
- **任意のイメージ・ユーザー** — rootおよび非rootコンテナイメージで動作

| 観点 | 保護 |
|--------|------------|
| ホスト作業ツリー | 変更されない — ワークスペースは独立したクローン |
| ワークスペース | `~/.realm/workspaces/<name>/` からバインドマウント、停止・起動をまたいで永続化 |
| セッションクリーンアップ | セッションマネージャーから削除でコンテナ、ワークスペース、セッションデータを削除 |

## 設計上の判断

### なぜ `git clone --local`？

gitの隔離戦略はいくつか存在しますが、それぞれに問題があります：

| 戦略 | 問題点 |
|------|--------|
| **ホストリポジトリをバインドマウント** | 隔離なし — エージェントが実際のファイルを直接変更してしまう |
| **git worktree** | `.git` ディレクトリをホストと共有するため、checkout・reset・rebaseがホストのブランチやrefに影響する |
| **bare-gitマウント** | 状態を共有するため、コンテナ内でのブランチ作成・削除がホストに影響する |
| **ブランチのみの隔離** | エージェントが他のブランチをチェックアウトしたり、共有refに対して破壊的なgitコマンドを実行することを防げない |
| **完全コピー（`cp -r`）** | 完全に隔離されるが、大きなリポジトリでは遅い |
| **Dockerサンドボックス（`--sandbox`）** | 不透明 — イメージの制御不可、永続性なし、SSH転送なし、カスタムDocker引数なし（[詳細は下記](#なぜ素のdocker--sandbox-ではなく)） |

`git clone --local` が最適な理由：

- **完全に独立** — クローンは独自の `.git` を持ち、コンテナ内の操作がホストリポジトリに影響することはない
- **高速** — 同一ファイルシステム上ではコピーではなくハードリンクを使用
- **完全** — 全履歴、全ブランチを含む標準的なgitリポジトリ
- **シンプル** — ラッパースクリプトや特別なentrypointは不要

### なぜ素のDocker（`--sandbox` ではなく）？

Claude Codeの組み込み `--sandbox` モードはDockerをラップしますが、利便性と引き換えに柔軟性を犠牲にしています：

- **不透明** — ベースイメージ、インストール済みパッケージ、コンテナの設定を制御できない
- **柔軟性がない** — 独自のツールチェーン、ランタイム、開発環境を持ち込めない
- **永続性がない** — 終了して再開ができない。毎回ゼロからスタート
- **SSH転送なし** — SSH鍵によるgit push/pullがそのままでは動作しない
- **カスタムDocker引数なし** — ネットワーク、追加ボリューム、環境変数の設定ができない
- **単一エージェント** — Claude Code専用。realmは任意のエージェントや手動操作で使用可能

素のDockerを使うことで完全な制御を維持しつつ、realmが隔離とライフサイクルを管理します。

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

デタッチモードでバックグラウンド実行：

```bash
realm ai-experiment -d --image node:20 -- claude -p "refactor the auth module"
```

エージェントが行うすべての操作はコンテナ内に留まります。完了したらセッションを削除すれば消えます。

## ライセンス

MIT
