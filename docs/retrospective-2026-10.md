<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# 開発レトロスペクティブ 2026-10

対象は 2026-10-04 から 2026-10-09 の履歴である。
この clone の最古のコミット（`afb1633`、2026-10-04）がこの範囲の始点になる。
それ以前の 9 月分は、履歴に実体がないため扱わない。

## 0. 調査の範囲と数値の出どころ

- `ccfarm/integration` の先端は `b6d8b4e`（2026-10-09 03:21）。
  `main` は先端 `16a53f8`（2026-10-06）で、`main` から見て integration は 281 コミット先行する。
  `main` への昇格はまだ行われていない。
- integration 上のコミット数は日別に次のとおり。
  10-04 が 18、10-05 が 72、10-06 が 28、10-07 が 119、10-08 が 139、10-09 が 20（約 400）。
- `ccfarm: merge` は 87 件、`resolve conflicts` は 22 件、`revert merge` は 2 件、手動 `Revert` は 1 件。
- この clone の `main` スナップショットには docs/plan-status.md と docs/board-sessions/ が無い。
  両者は integration 側にある。
  plan-status.md は `git show c5c4baf:docs/plan-status.md` で読んだ。
  取り込みの扱いは付録 A に書く。
- 「原因」と書いた箇所は、コミットメッセージと plan-status.md の記述からの推定である。
  エージェントのトランスクリプトまでは読んでいない。

## 1. 失敗とやり直し

| # | 何が起きたか | 証拠 | 推定原因 | 状態 |
|---|---|---|---|---|
| 1 | es3-caps と es2 report guard が、確認レビューの前にマージされた。その後 veto されて revert された。 | `b6004e1` → `3f78982`、`75dedad` → `a35095b`。ブランチは `ccfarm/01a11864-6943-…`、`ccfarm/01a11867-620d-…` | 相談（consultation）を経ずにマージされた。レビューの順序が守られていなかった。 | 未解決。plan-status で stopped。相談を通した作り直しが要る。 |
| 2 | ほかのブランチのマージが手動で revert された。 | `bca6c4d`（`ccfarm/01a110af-f843-…`） | 詳細はコミットから読めない。 | 解消済み。再投入の有無は未確認。 |
| 3 | AM62P plan #1 の板記録 `bf5c575` がボードのログインとシリアルブリッジ名を含んでいた。マージ済みの docs/board-sessions/am62p-plan1.md は「何も走らなかった」試行を記録している。 | plan-status 項目 `am62p-plan1-doc` | 板記録に伏せ字化の手順がなかった。 | 未解決。redact して再投入する項目が ready のまま。 |
| 4 | 板セッションが「何も走らず」に終わった。 | Pi 4: `pi4-es2-r1a`（`c4a4d7f`）、`pi4-es2-r1b`（`6e4c837`）、plan #1 の `dbd1e88`、`baseline-pi4`（`b32b3f4`）。AM62P: `baseline-am62p`（`27d33aa`、`c658d8a`）。ほかに `50a0b52`、`fdb7b5c`、`c9ab376`、`fb263cf` | ワーカーに Zephyr ワークスペース、SDK、dEQP stage 1、板の環境変数が無かった。コマンドが承認されず `west` が拒否された例もある。 | 一部解決。`76b3639`（setup-check-env）が入った。Pi 4 は `pi4-es2-r1a2` で再実行待ち。 |
| 5 | AM62P の板セッションで UART が無音だった。 | `am62p-es2-r1`（結果 `46e60a14`）、`3cfaff9`（am62p-gles3-percase）。image 8 は 428 秒で 0 バイト。 | 板側かブリッジ側の問題。ワーカー再起動で中断した回もある。 | 未解決。ES 3.0 の板セッションは停止中。 |
| 6 | ライセンス/SPDX 監査、伏せ字監査、website 更新の 3 項目が一度もマージされなかった。 | `ec9f59c`（決定 `34d7dc0`）、`3a4e55f`（`4b37a25`）、`058e6fc`/`b3cc30e`（`e4fe79a`） | 却下の理由が記録にない。 | 未解決。3 項目とも redo として待機。 |
| 7 | 同じ変更が work/ 系のブランチと別のコミットとして 2 度入っている。 | `c1f0e2a`/`e9d37dd`（imx8mp display 69.3 MHz）。`0190f68`/`d9d8dac`（ES 2.0 の DRAW_BUFFERS）。`c4f4ef5`/`c073059`（DEPTH_COMPONENT24）。`3edbf04`/`7cf0e80`（pvr switches）。`refs/ccfarm/origin/heads/land/*` と `work/*` が対になっている。 | work ブランチと land ブランチ、または main と integration の間で cherry-pick された。 | 現象として確認した。無害だが、履歴検索のノイズになる。 |
| 8 | 着地した変更が GC7000 を退行させた。dEQP の fbo depth_component24 と fragdata indexing が落ち、gl_test case 2 も間欠的に落ちた。 | `112dd4f`、`186cbb5`、`16a53f8`。修正は `c4f4ef5`、`d9d8dac`、`bf8e51d` | 「製品から切り替えを消す」型の整理（`3edbf04`、`7cf0e80`）と ES 2.0 の厳格化が、板で確認される前に着地した。 | 部分的に解決。case 2 は `bf8e51d` で「GPU ストリームには無い」と判明したが、間欠失敗は残る。 |
| 9 | AM62P の dEQP-GLES2 で、invariance 9 件が新規に退行した。 | `shaders.invariance.{lowp,mediump,highp}.common_subexpression_{1,2,3}`。診断項目 es2-invariance-diag は「done: no changes」で、レポートがマージされなかった。 | 診断項目が成果物を残さずに完了した。完了条件が曖昧だった。 | 未解決。es2-invariance が診断を自分で持つ形に変えた。 |
| 10 | ブランチが何度も衝突解決を要した。 | `resolve conflicts` が 22 件。連鎖も多い。`1bbf955`（4 段）、`2530ea6`（3 段）、`8240c3c`。対象は npot、glmark2 host harness、benchmark inventory、Pi 4 のベースライン。 | 複数プランが同じファイルを触った（docs/benchmarks.md、docs/es2-cts-failures.md、tools/cts/*）。 | 継続中。plan-status の「touch overlap」ブロックが 14 件ある。 |
| 11 | 「redo:」を付けた再投入が発生した。 | `5a71a1d`、`de7c911`（"redo: Benchmark needs inventory per board" と "redo: AM62P board session"）。どちらも `resolve conflicts` を 3 から 4 回伴った。 | 元の試行が使えなかった。事前条件を確認する手順が弱い。 | 解消済み。 |
| 12 | ゲートの clippy 行が、ホストによって赤になる。 | `141dcfa`（2026-10-06 のツールチェイン用）。`cf38c17`（stuck-results）。`1dc382e`（デフォルトツールチェインで 96 ok／3 host-tool、nightly で clippy 13 行追加）。Cargo.toml は `rust-version = 1.88` のみで、ツールチェインを固定していない。 | ツールチェインが固定されていない。 | 未解決。項目 `clippy-gate-fix` は ready だが、touch overlap で待機。 |
| 13 | ゲートの V3D ライブラリテスト手順がワーカーの時間制限を超えた。 | es2-offline-table-diag で `gpu-v3d` ビルドが kill された（exit 137）。項目 `gate-timing` は ready。 | 手順ごとの所要時間が計測されていない。 | 未解決。`docs/investigations/check-gate-timing.md` は未作成。 |
| 14 | 見積もりを超えた項目がある。 | plan-status.md 上は `pub-roadmap`（最初の試行は bundling error で失敗し、再試行中）。「見積もり超過」と明記した記述は plan-status.md 内には見つからなかった。 | 超過の記録欄が存在しない。 | 不明。次の改善案 P7 を参照。 |

## 2. 繰り返し現れるパターン

数えた件数は、上の表の行と plan-status.md の記述にもとづく概数である。

| パターン | 件数 | 該当 |
|---|---|---|
| A. 複数プランが同じファイルを触り、項目が互いに待つ。 | 14 件（touch overlap）と 22 件（resolve conflicts） | 表 #10、#11 |
| B. ワーカーに環境（Zephyr ワークスペース、SDK、dEQP、板の変数、承認）が無く、板セッションが空振りする。 | 8 件以上 | 表 #4、#5 |
| C. 板記録に機微情報が入り、伏せ字化のやり直しが要る。 | 2 件（`bf5c575`、監査項目）。`pub-redaction-audit` は未マージのまま。 | 表 #3、#6 |
| D. レビュー/相談の手順を飛ばし、マージ後に revert された。 | 3 件 | 表 #1、#2 |
| E. ホスト側ゲートが重い、またはホスト依存で不安定になる。 | 3 件（clippy、V3D テスト手順、case 10/2 のフレーク） | 表 #8、#12、#13 |
| F. 完了条件が曖昧で、診断が成果を残さない。 | 2 件以上 | 表 #9、#14 |
| G. 板での確認の前に着地し、後から退行が見つかる。 | 2 件 | 表 #8 |
| H. 同じ変更の二重着地。 | 4 組 | 表 #7 |

パターン A と B が最も多く、作業の待ち時間と空振りの大半を説明する。

## 3. 改善提案

効果と手間で順位を付けた。
「決定」は所有者の判断が要るもの、「実施可」は担当者がそのまま実施できるものを指す。

| 順位 | 提案 | 対応パターン | 効果 | 手間 | 区分 |
|---|---|---|---|---|---|
| 1 | 板セッション項目の冒頭に環境チェックを置く。`76b3639` の setup-check-env を呼び、足りなければ最初のターンで終了し、理由を記録に書く。pi4-es2-r1a2 は既にこの形である。 | B | 大 | 小 | 実施可 |
| 2 | 板記録のテンプレートと、コミット前の機微情報チェックを足す。docs/HANDOFF.md の秘匿規則に従う。tools/check.sh に、ログイン名、IP、UART パスを grep する行を追加する。 | C | 中 | 小 | 実施可 |
| 3 | ツールチェインを固定する。`rust-toolchain.toml` を置くか、ゲートに clippy のバージョン表示を入れる。 | E | 大 | 小 | 決定（固定するバージョン） |
| 4 | `gate-timing` を先に実行し、V3D ライブラリテスト手順を分割するか、`--quick` から外す。既存の項目である。 | E | 大 | 中 | 実施可 |
| 5 | 「マージ前に相談を通す」ことをマージ側で機械的に確認する。確認レビュー済みの印を持たない結果を、自動マージしない。 | D | 大 | 中 | 決定（ccfarm の設定） |
| 6 | 診断項目の完了条件に「docs/investigations/ にレポートをマージすること」を入れる。レポートが無ければ done にしない。 | F | 中 | 小 | 実施可 |
| 7 | plan-status.md に「見積もり」と「実績」の列を足す。超過した項目は理由を 1 行で書く。 | F | 小 | 小 | 実施可 |
| 8 | 共有ファイルを分ける。docs/benchmarks.md と docs/es2-cts-failures.md をボード別に分割する。板記録は既に docs/board-sessions/ にある。表の行は別ファイルに置く。 | A | 大 | 中 | 決定（レイアウトの変更） |
| 9 | 昇格の遅れを減らす。integration が main より 281 コミット先行しており、昇格の頻度を決める。 | G、H | 中 | 小 | 決定（所有者の判断） |
| 10 | 退行しうる整理は板確認を待ってから着地させる。例は `3edbf04` 系で、確認が済むまで `land/*` に置かない。 | G | 中 | 中 | 実施可（運用ルール） |

## 4. プロジェクトの現状（docs/plan-status.md 基準、履歴で更新）

基準は `c5c4baf` 版の plan-status.md（2026-10-09）である。
integration の先端は `b6d8b4e` で、plan-status の記載時点より進んでいる。
数値は 2026-10-09 時点の確認値と、記録上の値を区別して書く。

### ステージ 1: dEQP-GLES2（3 ボードで失敗 0）

- 完了: i.MX8MP は 0 fail（"d8583f20"、10-01）。AM62P は 17,141 pass／0 fail（"d60b9d76"、10-02）。
  Pi 4 のキューブ 25 件は host で修正済み（`c3e4146` ほか）。
- 進行中: AM62P の invariance 9 件の回帰。`gl_test` の case 10 と 2 のフレーク（#9/#25）。
  i.MX8MP の MSAA #8 は未解決。
- 滞留: Pi 4 は 10-01 以後、dEQP の完走記録が無い（表 #4）。
  AM62P の image 8 から 17 と ms4 は未走行。

### ステージ 2: ベンチマーク（glmark2、Slint、cluster）

- 完了: i.MX8MP のベースライン（GM 42、`2606b25`）。glmark2 の host harness。
- 進行中: npot（`2673197`）、carve-out（`ca4074a`）、refract（`541c9d3`）。いずれも host は済み、板確認待ち。
  terrain に新しい pool の隙間がある（`459343d`）。
  PowerVR 側は precompiled シェーダ層が AM62P で 48/48（`129290c`、`17605c3`）。
- 滞留: AM62P と Pi 4 のベースラインは計測できていない。

### ステージ 3: 公開

- 完了: CHANGELOG（`e6b84f7`）、CONTRIBUTING/SECURITY の草稿（`b4ec427`）、リリースビルドガイド。
- 進行中: `pub-roadmap`（再試行中）、`pub-contrib-docs`。
- 滞留: ライセンス監査、伏せ字監査、website の 3 項目は redo 待ち。`main` への昇格と公開は所有者の決定。

### ステージ 4: ES 3.0

- 完了: 設計文書と host 側の作業。AM62P の dEQP-GLES3 全体は 31,278 pass（73.1%）、"396d667a"。
- 進行中: 失敗マップ（shaders、texture 等）、整数カラーターゲット、centroid、転送フィードバック設計。
- 滞留: `es3-caps` と es2 report guard は revert 済みで、相談を通した作り直しが要る。
  板の GLES3 セッションは、各ボードのステージ 1 が終わるまで保留。AM62P は UART 無音で停止。

### ステージ 5: ボード追加

- 未着手。Armadillo-X2 の下地（`driver/boards/atmark/armadillo_x2`、docs/direct-boot.md）だけがある。

### 数値の更新

| 項目 | plan-status の値 | 履歴から確認できる値 |
|---|---|---|
| integration の先端 | `2256c26f` | `b6d8b4e`（10-09 03:21） |
| main との差 | 記載なし | integration が 281 コミット先行 |
| revert したマージ | 2 件（`3f78982`、`a35095b`） | 変わらず。ほかに手動 revert `bca6c4d` が 1 件 |
| touch overlap のブロック | 記載なし | 14 件 |
| 衝突解決コミット | 記載なし | 22 件 |

## 付録 A: 手順上の注意

- docs/plan-status.md は、この作業の起点（`main` の `16a53f8`）には無い。
  integration 側の版を参照して現状を書いた。
  「See also」行は、plan-status.md が `main` に入った後で足す。
  足す行は次のとおり。

  `See also: [Retrospective 2026-10](retrospective-2026-10.md).`
- マークダウンの lint は、tools/check.sh には無い。
  SPDX ヘッダだけを、他の docs に合わせて付けた。
