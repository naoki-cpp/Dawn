---
id      : ADR-0037
title   : Docked Ship Ownership — owned ship / active ship / docked station context を分離する
status  : accepted
date    : 2026-07-05
deciders: [human, ai-agent]
related : ADR-0034（Packaged Ship / Station / Assemble・Disassemble）, docs/architecture/ownership.md（§8 に Assemble 設計史・shipless gap を統合済み）, docs/process/roadmap.md §12（Phase 9B）
---

# ADR-0037 — Docked Ship Ownership

## 背景

9B の `BuildPackagedShip` は現行モデルに素直に乗るが、`Assemble` は
`PackagedShip -> live Ship entity` を作るため、現在の `PlayerId -> ShipId`
前提と衝突する。`dawn-sector` の実態は「プレイヤーが複数 ship を所有できない」
のではなく、「**1 player -> 1 active ship** しか語彙化されていない」状態である。

## 決定

Ship ownership を次の3つに分ける。

- **owned ship**: `PlayerId` が所有権を持つ ship
- **active ship**: いま flight command / session routing の対象になっている ship
- **docked station context**: プレイヤーが現在入港している station

`Assemble` は **新しい owned ship を docked station context に追加する操作** とし、
**active ship を自動では切り替えない**。どの ship で undock / 操作再開するかは
別アクションとして表現する。

## 却下した案

- **Assemble が active ship を即時置換する**: 到着 ship の扱いが曖昧になり、
  暗黙の auto-disassemble / hidden ship を生みやすいので却下。
- **Assemble では live ship を作らず中間形態を増やす**: `PackagedShip` と `Ship`
  の2形態という ADR-0034 の語彙を濁すため却下。

## 帰結

- `BuildPackagedShip` は現行のまま先行できる。
- `can_use_station(player_id, station_id)` は active ship lookup ではなく、
  player-level の docked station context に立脚させる。
- `Disassemble` は「現在の active docked ship のみ対象」として先に入れやすい。
- `Assemble` 着手前に、station 内での owned ship roster と active ship 切替の
  最小モデルを実装する必要がある。

## 実装状況（2026-07-07 追記）

上記の帰結は実装済み。`ShipRegistry.owners`（既存、複数所有に対応済みだった）と
`active_ship`（`by_player` から改名）を分離し、`SelectActiveShipCommand`（station-local
切替のみ）を新設。Undock を含む操縦系コマンドは `ship_id` を持たず、常に caller の
active ship に解決される（`is_active_ship` チェック）。station 管理系
（Fit/Unfit/Dock/BuildPackagedShip/DisassembleShip）は `ship_id` を維持し
`owns_ship` のまま（active でない所有船も対象にできる）。詳細は
`docs/architecture/ownership.md` §7。これで `Assemble`（roadmap.md §12 9B-5）の
前提条件は満たされた——本体はまだ未着手。
- 9B の blocker は Station そのものではなく、**ownership 語彙の浅さ**である。
