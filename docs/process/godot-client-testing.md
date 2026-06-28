# Godot クライアントのテスト手順（GdUnit4）

> AI_DEVELOPMENT_GUIDE.md §8「Godot クライアントのテスト方針」の詳細手順の正典。
> ガイド本体には方針の要約とこのファイルへのリンクのみを残す
> （ADR-0030 と同じ理由 — セットアップ手順は client/ を触るときだけ必要）。

## セットアップ

`client/addons/` は `.gitignore` 対象（各開発者が Godot エディタの AssetLib から
個別にインストールする想定）なので、**初回はエディタの AssetLib タブで
「GdUnit4」を検索してインストールし、`project.godot` の Plugins でこのアドオンを
有効化**すること（`enabled=PackedStringArray("res://addons/gdUnit4/plugin.cfg")`
は既にコミット済み。アドオン本体だけが各マシンでの個別インストール対象）。
テストは `client/test/` 以下に `<対象ファイル>_test.gd` として置く（例: `client/test/main_test.gd`）。

**Godot バイナリの取得**: リポジトリには Godot 本体を含めない（uv/pyenv 的に、
`.godot-version` でバージョンを pin し、各マシンが個別に取得する）。

```bash
scripts/setup-godot.sh             # .godot-version の指定版を .tools/godot/ に取得・SHA512検証
# Windows PowerShell:
scripts/setup-godot.ps1
scripts/setup-godot.sh --run-tests
scripts/setup-godot.ps1 -RunTests
```

## CLI 実行

取得した Godot バイナリで GdUnit4 を走らせる（作業ディレクトリは `client/`）:

```bash
cd client
GODOT_BIN="$(../scripts/setup-godot.sh --print)"
bash addons/gdUnit4/runtest.sh --godot_binary "$GODOT_BIN" -a test
```

On Windows, prefer the setup script so it also creates the Godot user log
directory and applies the pinned-version GdUnit4 compatibility patches:

```powershell
scripts/setup-godot.ps1 -RunTests
```

> **既知の互換性問題（GdUnit4 v6.1.3 × Godot 4.6系）**: GdUnit4 v6.1.3
> （AssetLib 配布版）は Godot 4.6 の破壊的変更（`FileAccess.get_as_text()` の
> `skip_cr` 引数削除、`debug/gdscript/warnings/exclude_addons` 設定の廃止。
> upstream issue GD-1004、master では修正済みだが本タグには未反映）に未対応で、
> そのままでは CLI 実行が失敗する。`client/addons/` は `.gitignore` 対象（各マシン
> ローカルインストール）なので、AssetLib でインストールした直後に以下の2点を
> **ローカルで手動パッチする**こと（再インストール時は再適用が必要）:
>   - `addons/gdUnit4/src/core/GdUnitFileAccess.gd:199`:
>     `file.get_as_text(true)` → `file.get_as_text()`
>   - `addons/gdUnit4/plugin.gd:17`:
>     `ProjectSettings.get_setting("debug/gdscript/warnings/exclude_addons")` に
>     第2引数 `false`（デフォルト値）を追加
> 次に GdUnit4 が 4.6 対応版をリリースしたら、このパッチは不要になる。

## テスト可能 / 対象外の判断基準

クライアント側はサーバー側（Rustクレート）と違い**全コードをテストできるわけではない**。

```
テスト可能（シーンツリー無依存の純粋関数・ロジック）:
  - 座標変換、レイ/距離計算、配列・辞書を入出力とする計算
  - 例: _server_to_godot_pos() / _ray_point_distance() / _spectral_color() /
        _compute_warp_snap_pos_core()（client/test/main_test.gd 参照）
  - スクリプトを .new() でシーンツリーに追加せずインスタンス化すれば _ready() は
    呼ばれないため、@onready 変数を使わない関数なら安全にテストできる

テスト不能・対象外（Godot エディタでの目視確認に委ねる）:
  - HUD構築・更新、入力ハンドリング、マーカー（ノード）生成、ピッキングのループ自体
  - @onready のシーンツリー直パス参照に依存する処理
  - WebSocket 通信（connection.gd の実接続部分）
  → これらは docs/architecture/architecture-review-client.md の C-1/C-3 で「Godot エディタでの
    動作確認が必要」と明記した領域と一致する
```

**新しい純粋関数を `main.gd` 等に追加・抽出するときは、テストも同じ変更に含めること。**
逆に、シーンツリー依存のロジックを変更したときは、テストを書けない代わりに
「Godot エディタで何を確認したか」を PR 説明に明記する（実機検証ができないAIセッションの
場合は、その旨と推奨される手動確認手順を明記する）。
