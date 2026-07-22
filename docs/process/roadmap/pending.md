---
scope    : 未完了タスクと現在の優先順位
audience : AI Agent / Human Developer
update   : タスク完了時、または優先順位が変わったとき
related  : ./README.md, ./deferred.md, ../roadmap.md, ../roadmap-history.md
---

# Roadmap — Pending

ここには未完了の作業だけを置く。次に着手する作業は、原則として `README.md` の
「次の1件」と一致させる。完了した項目は長い説明をここへ戻さず、
[completed.md](./completed.md)へ短く追記する。

## 3. TODO（未完了のみ）

### 3.1 現在の作業

| 優先度 | ID | タスク | 状態 |
|---|---|---|---|
| NOW | 9E-1 | 経済ループの人間プレイテストと結果記録 | 手順済み・実施待ち |
| NEXT | 10-3 | Client-Side PredictionをRust側へ実装 | 要ADR・未着手 |
| NEXT | 10-2/10-4 | 残る型共有・固定型メッセージ移行の整理 | 一部実装 |

### 3.2 Phase 9 の残作業

| ID | タスク | 状態 |
|---|---|---|
| 9C-1 | 構造物エンティティ・所有権モデルのADR | TODO |
| 9C-2 | `can_use(actor, structure) -> bool` の決定論的アクセス制御 | 9C-1後 |
| 9C-3 | 構造物のSector所有権とTransitの関係を確定 | TODO |
| 9C-4 | NPC Stationをプレイヤー建造可能へ拡張 | 9Cの設計後 |

### 3.3 Phase 10 の残作業

| ID | タスク | 状態 |
|---|---|---|
| 10-2 | `InitialState` / `PlayerLoadout` / `AoiEnter` の固定型移行を完了 | 一部実装 |
| 10-3 | Client-Side Predictionとreconciliation | 要ADR-0043・TODO |
| 10-4 | WebSocket上のpostcard移行の段階2を完了 | 段階1完了・残作業あり |
| 10-5 | Godot editorでのPrediction / dock / warpプレイテスト | 自動テスト済み・手動確認待ち |

### 3.4 Phase 11 のTODO

| # | タスク | 状態 |
|---|---|---|
| 1 | 船種ごとのglTF 3Dモデル | TODO・調達方針が必要 |
| 2 | 発射・被弾・爆発のパーティクル | TODO |
| 3 | ワープ突入・離脱エフェクト | TODO・優先度高 |
| 4 | モジュール発動フィードバックの拡充 | TODO |
| 5 | 恒星・惑星の見た目の深化 | TODO |
| 6 | Bloom・トーンマッピング等の調整 | TODO |
| 7 | 視覚エフェクトのフレームレート回帰確認 | 1〜6後 |

## 9. 継続的に開発するシステム

### Combat System

戦闘の基盤は完了したが、新モジュール・新ダメージタイプ・新戦術は継続的に追加する。
挙動変更は都度ADRを起票する。

### Economy System

Phase 9は基盤構築であり、新資源・新構造物・市場メカニクスは継続的な開発対象である。
9Cは基盤の外側にあるため、§3.2で独立して追跡する。

## 10. Phase 8 — スケール基盤 / 持続性

Phase 8の完了基準は満たした。未完了または条件付きの項目は
[deferred.md](./deferred.md) にのみ置く。

## 12. Phase 9 — Resource + Economy Context

ADR-0034の実装順。9A・9B・9Dは完了、9CはTODO、9E-1は進行中、9E-2は保留。

## 13. Phase 10 — Client 本格化

GDExtension導入と主要なwire移行は完了した。残作業は §3.3 のみを正典とする。

## 14. Phase 11 — グラフィックの深化

残作業は §3.4 のみを正典とする。サーバー権威・イベントスキーマ・ゲームルールは変更しない。
