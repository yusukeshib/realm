# box

[English](README.md)

[![Crates.io](https://img.shields.io/crates/v/box-cli)](https://crates.io/crates/box-cli)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![CI](https://github.com/yusukeshib/realm/actions/workflows/ci.yml/badge.svg)](https://github.com/yusukeshib/realm/actions/workflows/ci.yml)

AIコーディングエージェントのための安全で使い捨て可能な開発環境 — DockerとGitで動作。

![demo](./demo.gif)

## なぜ box？

AIコーディングエージェント（Claude Code、Cursor、Copilot）は強力ですが、実際の作業ツリーで自由に動かすのはリスクがあります。Boxは**安全で隔離されたサンドボックス**を提供し、エージェントが影響を気にせず自由に実験できる環境を作ります。

- **コードの安全性** — 独立したクローンを使用し、ホストのファイルは一切変更されません
- **AIエージェントが自由に実験可能** — コミット、ブランチ作成、書き換え、破壊 — 作業ツリーには影響なし
- **永続的なセッション** — 終了しても再開時にそのまま続行、ファイルは保持されます
- **名前付きセッション** — 複数の実験を並行して実行可能
- **自分のツールチェーンを使用** — 任意のDockerイメージで動作

## 必要なもの

- [Docker](https://www.docker.com/)（macOSでは[OrbStack](https://orbstack.dev/)も可）
- [Git](https://git-scm.com/)

## インストール

### クイックインストール

```bash
curl -fsSL https://raw.githubusercontent.com/yusukeshib/realm/main/install.sh | bash
```

### crates.ioから

```bash
cargo install box-cli
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
box my-feature --image ubuntu:latest -- bash
# gitアクセス可能な隔離コンテナの中にいます
```

Boxはgitリポジトリ内で実行する必要があります — 現在のリポジトリをコンテナ内にクローンします。

フラグ不要のワークフローについては、下記の[カスタムイメージのセットアップ](#カスタムイメージのセットアップ)を参照してください。

## カスタムイメージのセットアップ

Boxの推奨ワークフロー：イメージを一度ビルドし、環境変数を設定するだけ。以降はフラグなしで使えます。

**1. ツールチェーンを含むDockerfileを作成**

ワークフローに必要なツール（言語、ランタイム、CLIツールなど）をすべて含めます。

**2. イメージをビルド**

```bash
docker build -t mydev .
```

**3. 環境変数を設定**

`.zshrc` または `.bashrc` に以下を追加：

```bash
export BOX_DEFAULT_IMAGE=mydev              # カスタムイメージ
export BOX_DOCKER_ARGS="--network host"     # 常に使いたいDockerフラグ
export BOX_DEFAULT_CMD="bash"               # 新規セッションのデフォルトコマンド
```

**4. 完了 — あとは box を使うだけ**

環境変数を設定すれば、すべてのセッションがフラグなしでカスタムイメージを使用します：

```bash
# これだけです。以降は:
box feature-1
box bugfix-auth
box experiment-v2
# それぞれが完全なツールチェーンを持つ隔離サンドボックスになります。
```

## 使い方

```bash
box                                               セッションマネージャー（TUI）
box <name> [options] [-- cmd...]                  セッションの作成または再開
box <name> -d [-- cmd...]                         バックグラウンドで実行（デタッチ）
box <name>                                        実行中のセッションにアタッチ
box config zsh|bash                                シェル補完を出力
box upgrade                                       最新版にアップグレード
```

### セッションマネージャー

引数なしで `box` を実行すると、対話型TUIが開きます：

```
 NAME            STATUS   PROJECT                   IMAGE            CREATED
  New box...
> my-feature     running  /Users/you/projects/app   alpine:latest    2026-02-07 12:00:00 UTC
  test                    /Users/you/projects/other  ubuntu:latest   2026-02-07 12:30:00 UTC

 [Enter] Resume  [d] Delete  [q] Quit
```

- **Enter** でセッションを再開、または「New box...」で新規作成
- **d** でハイライト中のセッションを削除（確認あり）
- **q** / **Esc** で終了

### セッションの作成または再開

```bash
# デフォルト: alpine:latest イメージ、sh シェル、カレントディレクトリ
box my-feature

# カスタムイメージでbashを使用（作成時のみ有効）
box my-feature --image ubuntu:latest -- bash

# 追加のDockerフラグ（環境変数、ボリューム、ネットワークなど）
box my-feature --docker-args "-e KEY=VALUE -v /host:/container --network host"

# セッションが存在すれば元の設定で再開
# 存在しなければ新規作成
box my-feature
```

セッションが存在しない場合は自動的に作成されます。既存のセッションを再開する場合、`--image` などの作成時オプションは無視されます。`--docker-args` や `--no-ssh` などのランタイムオプションは毎回適用されます。

### デタッチモード

```bash
# バックグラウンドで実行
box my-feature -d -- claude -p "do something"

# 実行中のセッションにアタッチ
box my-feature

# 停止せずにデタッチ: Ctrl+P, Ctrl+Q
```

## オプション

| オプション | 説明 |
|--------|-------------|
| `-d` | バックグラウンドでコンテナを実行（デタッチ） |
| `--image <image>` | 使用するDockerイメージ（デフォルト: `alpine:latest`）- 作成時のみ有効 |
| `--docker-args <args>` | 追加のDockerフラグ（例: `-e KEY=VALUE`、`-v /host:/container`）。`$BOX_DOCKER_ARGS` を上書き |
| `--no-ssh` | SSHエージェント転送を無効化（デフォルトは有効） |

## 環境変数

CLIフラグを完全に省略するためのデフォルト設定です。`.zshrc` や `.bashrc` に設定すれば、`box <name>` を実行するだけで自動的に適用されます。

| 変数 | 説明 |
|----------|-------------|
| `BOX_DEFAULT_IMAGE` | 新規セッションのデフォルトDockerイメージ（デフォルト: `alpine:latest`） |
| `BOX_DOCKER_ARGS` | デフォルトの追加Dockerフラグ。`--docker-args` が指定されていない場合に使用 |
| `BOX_DEFAULT_CMD` | 新規セッションのデフォルトコマンド。`-- cmd` が指定されていない場合に使用 |

```bash
# 全セッションにデフォルトのDockerフラグを設定
export BOX_DOCKER_ARGS="--network host -v /data:/data:ro"
box my-session

# 特定のセッションで --docker-args で上書き
box my-session --docker-args "-e DEBUG=1"
```

## シェル補完

シェル設定ファイルに以下のいずれかを追加すると、セッション名やサブコマンドのタブ補完が有効になります：

```bash
# Zsh (~/.zshrc)
eval "$(box config zsh)"

# Bash (~/.bashrc)
eval "$(box config bash)"
```

シェルを再読み込みすると、`box [tab]` で利用可能なセッションとサブコマンドが表示されます。

## 仕組み

初回実行時、`git clone --local` でリポジトリの独立したコピーをワークスペースディレクトリに作成します。コンテナは完全に自己完結したgitリポジトリを取得します — 特別なマウントやentrypointスクリプトは不要です。ホストの作業ディレクトリは一切変更されません。

- **独立したクローン** — 各セッションは `git clone --local` による完全なgitリポジトリを持ちます
- **永続的なワークスペース** — `exit` してもファイルは保持され、`box <name>` で再開可能。セッションマネージャーから削除でクリーンアップ
- **任意のイメージ・ユーザー** — rootおよび非rootコンテナイメージで動作

| 観点 | 保護 |
|--------|------------|
| ホスト作業ツリー | 変更されない — ワークスペースは独立したクローン |
| ワークスペース | `~/.box/workspaces/<name>/` からバインドマウント、停止・起動をまたいで永続化 |
| セッションクリーンアップ | セッションマネージャーから削除でコンテナ、ワークスペース、セッションデータを削除 |

## 設計上の判断

<details>
<summary><strong>なぜ <code>git clone --local</code>？</strong></summary>

gitの隔離戦略はいくつか存在しますが、それぞれに問題があります：

| 戦略 | 問題点 |
|------|--------|
| **ホストリポジトリをバインドマウント** | 隔離なし — エージェントが実際のファイルを直接変更してしまう |
| **git worktree** | `.git` ディレクトリをホストと共有するため、checkout・reset・rebaseがホストのブランチやrefに影響する |
| **bare-gitマウント** | 状態を共有するため、コンテナ内でのブランチ作成・削除がホストに影響する |
| **ブランチのみの隔離** | エージェントが他のブランチをチェックアウトしたり、共有refに対して破壊的なgitコマンドを実行することを防げない |
| **完全コピー（`cp -r`）** | 完全に隔離されるが、大きなリポジトリでは遅い |

`git clone --local` が最適な理由：

- **完全に独立** — クローンは独自の `.git` を持ち、コンテナ内の操作がホストリポジトリに影響することはない
- **高速** — 同一ファイルシステム上ではコピーではなくハードリンクを使用
- **完全** — 全履歴、全ブランチを含む標準的なgitリポジトリ
- **シンプル** — ラッパースクリプトや特別なentrypointは不要

</details>

<details>
<summary><strong>なぜ素のDocker？</strong></summary>

一部のツール（例: Claude Codeの `--sandbox`）はDockerサンドボックスを組み込みで提供しています。Boxは別のアプローチ — 素のDockerを直接使用 — を採ることで、以下を実現しています：

- **自分のツールチェーンを使用** — 必要な言語、ランタイム、ツールを含む任意のDockerイメージを使用可能
- **永続的なセッション** — 終了しても再開可能、ファイルと状態が保持される
- **SSHエージェント転送** — ホストのSSH鍵で `git push` / `git pull` がそのまま動作
- **完全なDocker制御** — カスタムネットワーク、ボリューム、環境変数、その他の `docker run` フラグが使用可能
- **任意のエージェントで動作** — 特定のツールに縛られず、Claude Code、Cursor、Copilot、手動操作で使用可能

素のDockerを使うことで完全な制御を維持しつつ、Boxが隔離とライフサイクルを管理します。

</details>

## SSHエージェント転送

**課題**: Dockerコンテナは通常、ホストのSSH鍵にアクセスできません。macOSではさらに困難です — DockerがVM内で動作するため、UnixソケットがVM境界を越えられません。

**boxの解決策**: ホストのSSHエージェントをコンテナに自動転送します。`git push`、`git pull`、`ssh` がすべて既存の鍵で動作します — 鍵のコピーは不要です。

**プラットフォーム別の仕組み**:

- **macOS**（Docker Desktop / OrbStack）: VM経由のソケット `/run/host-services/ssh-auth.sock` をマウント
- **Linux**: `$SSH_AUTH_SOCK` を直接コンテナにマウント

Boxはクローンしたリポジトリの `origin` リモートを実際のURL（ローカルクローンパスではなく）に自動修正するため、`git push origin` がそのまま動作します。

```bash
box my-feature --image ubuntu:latest -- bash

# コンテナ内で
ssh-add -l          # 鍵の一覧が表示されるはずです
git push origin main

# SSH転送を無効にする場合
box my-feature --no-ssh -- bash
```

## Claude Code連携

Boxは[Claude Code](https://docs.anthropic.com/en/docs/claude-code)の理想的なパートナーです。Boxセッション内でClaude Codeを実行すれば、リスクのある変更、ブランチの実験、テストの実行 — すべてホストから完全に隔離された環境で行えます。

```bash
box ai-experiment --image node:20 -- claude
```

デタッチモードでバックグラウンド実行：

```bash
box ai-experiment -d --image node:20 -- claude -p "refactor the auth module"
```

エージェントが行うすべての操作はコンテナ内に留まります。完了したらセッションを削除すれば消えます。

## ライセンス

MIT
