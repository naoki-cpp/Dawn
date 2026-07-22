---
scope    : Roadmap compatibility entry and section index
audience : AI Agent / Human Developer
update   : Keep links and compatibility headings aligned with docs/process/roadmap/
related  : ./roadmap/README.md, ./roadmap/pending.md, ./roadmap/completed.md, ./roadmap/deferred.md, ./roadmap-history.md
---

# Roadmap

ロードマップの正典は [roadmap/README.md](./roadmap/README.md) と、その配下の分割ファイルである。
architecture review と同じく、現在地・未完了・完了済み・条件待ちを分けている。

## 入口

- [現在地と読み方](./roadmap/README.md)
- [未完了タスク](./roadmap/pending.md)
- [完了済み（短縮）](./roadmap/completed.md)
- [条件待ちバックログ](./roadmap/deferred.md)
- [詳細な過去ログ](./roadmap-history.md)

## 1. 読み方

次の1件は [roadmap/README.md](./roadmap/README.md) に、未完了タスクは
[roadmap/pending.md](./roadmap/pending.md) にある。既存の§参照を壊さないため、
以下の番号付き見出しは互換性用の索引として残す。

## 2. 現在地

**Phase 9 基盤実装完了・9E 経済ループ検証中**。次の1件は **9E-1 — 経済ループのプレイテスト**。
詳細は [roadmap/README.md](./roadmap/README.md) を参照する。

## 3. TODO（未完了のみ）

正典は [roadmap/pending.md](./roadmap/pending.md) である。

## 4. 完了済み（短縮）

正典は [roadmap/completed.md](./roadmap/completed.md) である。

## 9. 継続的に開発するシステム

Combat System と Economy System の継続的な作業は [roadmap/pending.md](./roadmap/pending.md) を参照する。

## 10. Phase 8 — スケール基盤 / 持続性

完了基準は満たしている。条件付きの項目は [roadmap/deferred.md](./roadmap/deferred.md) にある。

## 11. 負荷対応バックログ

正典は [roadmap/deferred.md](./roadmap/deferred.md) §3.5 である。

## 12. Phase 9 — Resource + Economy Context

ADR-0034の実装順を示す互換性用索引。

- 9A: Item / Scrap Metal — completed
- 9B: Station / Packaged Ship / Assemble / Disassemble — completed
- 9C: Player-built infrastructure — TODO（[pending.md](./roadmap/pending.md)）
- 9D-1〜9D-3: Market crate、order book、Currency ledger — completed
- 9D-4: Sector bridge commands — completed
- 9D-5: Station限定Market UIとwire/runtime bridge — completed
- 9E: Economy loop validation — 9E-1 in progress, 9E-2 deferred

9D-1〜9D-5の完了記録は [completed.md](./roadmap/completed.md) を参照する。
9Cと9Eの残作業は [pending.md](./roadmap/pending.md) を参照する。
9A-5と9E-2の保留理由は [deferred.md](./roadmap/deferred.md) を参照する。

関連ADR: ADR-0034、ADR-0037、ADR-0038。

## 13. Phase 10 — Client 本格化

GDExtension導入と主要なwire移行は完了した。残作業は
[pending.md](./roadmap/pending.md) §3.3 のみを正典とする。関連ADR: ADR-0039〜0043。

## 14. Phase 11 — グラフィックの深化

残作業は [pending.md](./roadmap/pending.md) §3.4 のみを正典とする。

## 15. 廃止・変更された計画

廃止・変更の判断記録は [roadmap-history.md](./roadmap-history.md) と各ADRを参照する。
