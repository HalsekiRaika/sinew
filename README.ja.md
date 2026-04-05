# sinew

[Claude Code](https://docs.anthropic.com/en/docs/claude-code) セッション間のピア発見とメッセージング。

sinew は複数の Claude Code インスタンスが互いを発見し、ローカルのブローカーデーモンを介してリアルタイムにメッセージを交換できるようにします。

> [louislva/claude-peers-mcp](https://github.com/louislva/claude-peers-mcp) に着想を得て、Rust で一からスクラッチ実装した単一バイナリです。

**[English / 英語](README.md)**

## 特徴

- **ピア発見** - 同一マシン、ディレクトリ、Git リポジトリ上の他の Claude Code セッションを検出
- **リアルタイムメッセージング** - チャンネル通知によるセッション間メッセージの送受信
- **ブローカー自動起動** - 必要時にブローカーデーモンが自動で立ち上がる
- **シングルバイナリ** - ランタイム依存なし。ブローカーと MCP サーバーを1つの実行ファイルに統合
- **クロスプラットフォーム** - Windows、macOS、Linux 対応

## インストール

### ソースからビルド

```bash
cargo install --git https://github.com/HalsekiRaika/sinew
```

### ビルド済みバイナリ

[GitHub Releases](https://github.com/HalsekiRaika/sinew/releases) からダウンロード、または [cargo-binstall](https://github.com/cargo-bins/cargo-binstall) を使用:

```bash
cargo binstall sinew --git https://github.com/HalsekiRaika/sinew
```

## クイックスタート

### 1. Claude Code の設定

Claude Code の MCP 設定（`~/.claude/claude_desktop_config.json` など）に sinew を追加:

```json
{
  "mcpServers": {
    "sinew": {
      "command": "sinew",
      "args": ["serve"]
    }
  }
}
```

これだけで準備完了です。最初のセッション接続時にブローカーが自動起動します。

### 2. Claude Code から使う

設定後、Claude Code で4つのツールが利用可能になります:

| ツール | 説明 |
|--------|------|
| `list_peers` | 他の Claude Code セッションを発見 |
| `send_message` | 他のセッションにメッセージを送信 |
| `check_messages` | 受信メッセージを確認 |
| `set_summary` | ピアに表示される作業サマリーを設定 |

### 3. チャンネル通知

メッセージ受信時、sinew は `notifications/claude/channel` を通じてリアルタイム通知を Claude Code にプッシュします。リサーチプレビュー期間中はフラグ付きで起動する必要があります:

```bash
claude --dangerously-load-development-channels server:sinew
```

`server:sinew` 引数でチャンネル通知を許可する MCP サーバーを指定します。このフラグなしでも、`check_messages` ツールで手動メッセージ確認は可能です。

## アーキテクチャ

```
Claude Code A                         Claude Code B
     |                                      |
  [MCP Server]                         [MCP Server]
  (sinew serve)                        (sinew serve)
     |                                      |
     +----------> [Broker Daemon] <---------+
                  (sinew broker)
                  localhost:7899
                      |
                   [SQLite]
```

sinew は2プロセス構成です:

- **Broker** - `localhost:7899` 上の HTTP サーバー + SQLite。ピアの中央レジストリ兼メッセージルーター。
- **MCP Server** - Claude Code セッションごとに1つ。HTTP でブローカーと通信。15秒ごとにハートビート送信、1秒ごとにメッセージをポーリング。

## CLI

```
sinew <COMMAND>

Commands:
  broker    Broker デーモンを起動
  serve     MCP サーバーを起動（stdio トランスポート）
  shutdown  稼働中の Broker デーモンをシャットダウン
  status    Broker と接続中のピアの状態を表示
```

### `sinew broker`

ブローカーデーモンを手動起動（通常不要 - `serve` が自動起動します）。

```bash
sinew broker --port 7899
```

### `sinew serve`

MCP サーバーを起動。Claude Code から呼び出されるコマンドです。

```bash
sinew serve --broker-url http://127.0.0.1:7899
```

### `sinew status`

ブローカーの健全性と接続ピア数を確認。

```bash
sinew status
# Broker: ok (http://127.0.0.1:7899)
# Peers:  3
```

### `sinew shutdown`

ブローカーデーモンをグレースフルに停止。

```bash
sinew shutdown
```

## 設定

### 環境変数

| 変数 | 説明 |
|------|------|
| `OPENAI_API_KEY` | 起動時の自動サマリー生成を有効化（任意、`gpt-4o-mini` を使用） |
| `RUST_LOG` | ログレベルフィルター（例: `debug`, `info`, `warn`） |

### デフォルト値

| 設定 | 値 |
|------|------|
| ブローカーポート | `7899` |
| ブローカー URL | `http://127.0.0.1:7899` |
| ハートビート間隔 | 15秒 |
| メッセージポーリング間隔 | 1秒 |
| データベース位置 | `{TEMP_DIR}/sinew-broker.db` |

## ピアスコープ

ピア一覧取得時にスコープでフィルタリングできます:

| スコープ | 返却内容 |
|----------|---------|
| `machine` | システム上の全ピア |
| `directory` | 同一作業ディレクトリのピア |
| `repo` | 同一 Git リポジトリのピア |

## ソースからのビルド

Rust 1.85 以降（edition 2024）が必要です。

```bash
git clone https://github.com/HalsekiRaika/sinew.git
cd sinew
cargo build --release
```

### テスト実行

```bash
cargo test
```

### リント

```bash
cargo clippy
cargo deny check
```

## 謝辞

本プロジェクトは [@louislva](https://github.com/louislva) 氏の [claude-peers-mcp](https://github.com/louislva/claude-peers-mcp) の設計に着想を得た、Rust によるフルスクラッチ実装です。
