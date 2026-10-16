<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# CTS の状況

2026-10-16 更新。いまの状態だけを書く。数字とコミットは [RESULTS.md](RESULTS.md)、開いている作業とその順番は [docs/HANDOFF.md](../../docs/HANDOFF.md)、それより前の経緯は git の履歴。
ゴールは「Zephyr 上で、内蔵シェーダコンパイラを使って、Khronos の CTS（dEQP）を通す」。

## ひとことで

- **ES 2.0（段階 1）は達成。** main e9901869a で 3 ボードとも dEQP-GLES2 の失敗 0（10/15〜16、各ボードのブートローダから起動、#870）。AM62P 17,140 合格（1x と ms4）、i.MX8MP 17,318 合格、Pi 4 17,126 合格。リセットは数えた行ですべて 0。未対応 34 件は 3 ボード共通で受け入れ済み。
- **共通セット（段階 2 の入力）は 3 ボードとも全行 done**（e9901869a）。glmark2 は AM62P 462、Pi 4 188、i.MX8MP は 43（10/14、コンソールが無いのでスコアは記録から）。i.MX8MP の glmark2 は GL エラーが 15 件以上残る（#814）。各シーンの絵はまだ確かめていない。
- **ES 3.0（段階 4）。** AM62P の dEQP-GLES3 全ツリー（ad2b5ed、10/14）は必須 42,810 件のうち 36,552 合格 / 失敗 2,880 / 未対応 2,195 / 品質警告 185 / 結果なし 998。Pi 4 は 91 画像のうち 7 画像だけ（2,798 / 744 / 265）。i.MX8MP はまだボードで走らせていない。

## 次の山

- 段階 2: glmark2 の各シーンの絵の確認と、ボードごとの最低性能（docs/notes/stage2-bench-gap.md）。AM62P の terrain、shadow、refract が遅い（1 フレーム 1.47 s、0.38 s、2.08 s）。
- 段階 4: AM62P の GLES3 の残り失敗と、画像 20・21 の GPU ハング、画像 7 の ResourceError。Pi 4 と i.MX8MP の GLES3 全ツリー。

## 注意

- 数字は Zephyr 上、内蔵コンパイラ。dEQP-GLES2 は ES 2.0 報告、dEQP-GLES3 は ES 3.0 報告のイメージ。
- 「未対応」（ES 2.0 にない機能、マルチサンプル構成なし）は失敗に数えない。
