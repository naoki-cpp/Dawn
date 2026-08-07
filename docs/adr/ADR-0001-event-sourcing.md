---
id      : ADR-0001
title   : Event Sourcing の採用
status  : accepted
date    : 2026-06-04
deciders: [human, ai-agent]
---

# ADR-0001 — Event Sourcing の採用

> **ADR-0049 amendment (2026-08-07):** `DomainEvent` は引き続き append-only の
> durable public/business fact と監査履歴だが、Sector の exact operational recovery
> における唯一の state authority ではない。権威状態の復旧は versioned checkpoint +
> committed authoritative state-delta tail で行う。DomainEvent と reliable outbox
> intent は同じ durable transition envelope に原子的に commit される。以下の
> 「Event を唯一の真実」「同じ Event だけで常に同じ State を完全復元」という旧記述は、
> public fact / audit / projection の文脈に限定して読むこと。復旧・commit ordering は
> ADR-0049 と `docs/architecture/recovery-contract.md` が normative である。

## コンテキスト

分散ノード間で数万エンティティの状態を一貫して管理する方法を決定する必要があった。  
以下の要件が設計を制約した。

- 複数ノードが同一エンティティを変更した場合の競合解決
- バグ発生時に「どのような操作の結果としてこの状態になったか」を追跡できること
- 将来の分散化において、ノードが部分的に障害を起こしても状態を再構築できること
- AI エージェントが継続開発する際に、過去の変更を安全に再演算できること

---

## 検討した選択肢

### A: State 同期（現在状態をそのまま伝播）

データベースに現在の State を保存し、変更時に上書きする。  
ノード間では「現在の状態」をブロードキャストして同期する。

**メリット:**
- 実装がシンプル
- クエリが容易（現在状態を直接読むだけ）

**デメリット:**
- 競合発生時に「どちらが正しいか」を判断する因果情報が失われる
- バグ修正後に「過去の状態はどうだったか」を再現できない
- ノード間で State が diverge した場合のマージ手段がない

### B: Event Sourcing（Event を durable fact とする）

「何が起きたか」を append-only の `DomainEvent` として記録し、CQRS と組み合わせる。
ADR-0049 以降、これらの event は exact ECS recovery reducer ではなく、public/business
fact・監査・projection の authority である。exact operational state は同じ transition に
commit された authoritative state delta から復旧する。

**メリット:**
- public/business change に因果情報（Tick・NodeId）が付与される
- バグ調査時に durable fact の履歴を追跡できる
- public projection を既存 Event から再生成できる
- exact recovery schema を public event catalog から分離できる（ADR-0049）

**デメリット:**
- Event スキーマの後方互換性管理が必要
- exact state recovery には別の versioned state-delta/checkpoint 契約が必要
- 「現在状態」への点クエリが Read モデルを必要とする

### C: Operational Transformation (OT)

操作の変換によって競合を解決する。Google Docs タイプの協調編集で実績あり。

**デメリット:**
- ゲームシミュレーションのユースケースへの適合性が低い
- 実装の複雑性が高く、AI による継続開発に向かない

---

## 決定

**Event Sourcing + CQRS を public/business fact と audit/projection の基盤として採用する。**

Sector exact recovery については ADR-0049 の **versioned authoritative state-delta journal +
checkpoint** が追加の normative authority である。

---

## 根拠

分散システムにおける重要な問題の一つは「何が起きたかの合意」である。State 同期だけでは
現在値しか持たず、競合解決や監査に必要な因果情報が失われる。public/business fact を
append-only に記録することで、その履歴を保持できる。

一方、位置・capacitor・lock countdown・module cycle・queue 等の高頻度 authoritative
mutation は public event catalog では完全に表現されず、eventless Tick も存在する。
ADR-0049 はこのギャップを exact state-delta journal で埋める。したがって
`DomainEvent` replay は public projection/audit に有用だが、任意の committed Tick の
bit/exact state を単独で再構築する契約ではない。

AI エージェントが機能を拡張する際、新しい public Projection（Read モデル）を追加して
既存 Event からビューを生成できる価値は維持される。同時に、recovery representation は
public schema から独立して versioning できる。

---

## 影響

### 採用によって得られるもの

- public/business fact の append-only 監査履歴
- event を用いた時間旅行的な因果調査と public projection 再生成
- public event schema と exact state-recovery schema の分離
- ADR-0049 の state delta と同一 atomic envelope に event を含めることで、commit 後に
  public fact だけ失う crash window を排除

### 採用によって生じるトレードオフ

- Event スキーマの後方互換性を管理する必要がある
- exact recovery には checkpoint/state-delta format の versioning も必要
- 「現在状態を読む」という単純な操作に Read モデルのオーバーヘッドが生じる

### この決定が強制する設計上の制約

```text
- committed DomainEvent を update / delete / in-place rewrite してはならない
  （INV-001 / FBD-001）。
- authoritative state mutation は ADR-0049 の RecoveryDelta で durable に表現する。
  DomainEvent が存在しない Tick も durable transition を持つ。
- operational recovery は compatible checkpoint + committed authoritative state-delta tail。
  public-event-only replay や historical Tick 再実行は exact recovery authority ではない。
- DomainEvent と reliable outbox intent は transition delta と同じ atomic commit boundary を持つ。
- Event スキーマの後方互換性は event-catalog.md のルールで管理する。
```

---

## 今後の再評価トリガー

- Recovery/state-delta log の write volume や replay cost が checkpoint/compaction でも
  管理できないほど増えた場合
  → ADR-0049/#284 の recovery model と RTO budget を再評価する。
- Event スキーマの後方互換性管理コストが開発速度を著しく下げた場合
  → public fact / projection 契約自体を別 ADR で再評価する。
