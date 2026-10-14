<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# CTS の状況

2026-10-15 更新（dEQP-GLES2 の 10/12〜10/14 の記録。下の「10/12〜10/14 の dEQP-GLES2」の節）。ゴールは「Zephyr 上で、内蔵シェーダコンパイラを使って、Khronos の CTS（dEQP）を通す」。
このファイルはいまの状態だけを書く。細かい数字・ケース名・コミットは [RESULTS.md](RESULTS.md)、開いている作業とその順番は [docs/HANDOFF.md](../../docs/HANDOFF.md)、9/25〜10/7 の日記は [log/status-2026-09-25-to-10-07.md](log/status-2026-09-25-to-10-07.md)（それより前は git の履歴）。

## ひとことで

**dEQP-GLES2: 10/12 に 3 ボードとも 1 つのコミットの全ツリーで失敗 0（AM62P 63efed8e、i.MX8MP c345ab74、Pi 4 d56e79e5）。その後 8bbf9ba2 で `GL_BGRA_EXT` のレンダーバッファ 46 件が退行し、i.MX8MP（cd738b1a、835cf9cc）と Pi 4（20a48791 の行 0〜14）で失敗 46。未修正。AM62P は 8bbf9ba2 を含むコミットをまだ走らせていない。**
**dEQP-GLES3 は AM62P で必須 42,810 件の全 28 画像を 10/11 に走らせた（main f2a13877）: 32,669 合格（76.3%）、失敗 5,549、未対応 2,318、結果なし 2,089（GPU のハングが 2 ケース）。** ここが次の大きな山。Pi 4 と i.MX8MP の GLES3 はまだ走らせていない。性能はその後。

## 10/12〜10/14 の dEQP-GLES2

失敗ごとの状態と担当は docs/es2-cts-failures.md の「Per failure」と「After round 1」、まだ測っていないものは同じファイルの「Not yet measured in round 1」。数字はどれもコミット済みの記録から取った（必須 17,165 件）。

| | コミット（日付） | 結果 | 記録 |
|---|---|---|---|
| AM62P | 63efed8e（10/12）、全 17 画像 + ms4 | **失敗 0**。1x は 17,125 合格 / 未対応 34 / 品質警告 5 / 互換警告 1、ms4 で multisample 16 件を置き換えて 17,141 / 0 / 18 / 5 / 1。invariance 9 件（画像 1: 1,512 / 1,512）、texture.vertex.2d 45 件（画像 9: 452 / 452）、ms4 の 2 件は全ツリーで合格を確認 | docs/board-sessions/am62p-es2-final-r2.md |
| AM62P | 40dd2107（10/12）、画像 8〜17、ms4、gl_test | 画像 8〜17 は 7,023 / 失敗 0 / 34 / 5 / 2、ms4 16 / 0、gl_test 50/50、171 番単独も合格 | docs/board-sessions/am62p-es2-r1c.md |
| AM62P | cd738b1a（10/14） | 何も走っていない（イメージは作った。コンソールが無音、またはワーカーからボードに届かない） | docs/board-sessions/am62p-es2-final.md |
| i.MX8MP | c345ab74（10/12）、全ツリー | **失敗 0**。`TALLY (29 entries): begun 17371 passed 17318 failed 0 notsup 34 other 19 noresult 0` | docs/board-sessions/imx8mp-es2-final-r2.md |
| i.MX8MP | cd738b1a（10/14、2 回）と 835cf9cc（10/14）、全ツリー | **失敗 46**。17,272 合格 / 失敗 46 / 未対応 34 / その他 19。46 件はすべて e21（fbo）の `GL_BGRA_EXT` レンダーバッファ | docs/board-sessions/imx8mp-es2-final.md、RESULTS.md の 835cf9cc の節 |
| Pi 4 | a25712f2（10/12、統合の先端）、全ツリー | 17,118 / **失敗 9** / 34 / 3 / 1。9 件は T3 6 件と T4 3 件のキューブ（9b24ea2 を含まないコミット） | docs/board-sessions/pi4-es2-tip-r2.md |
| Pi 4 | 40dd2107（10/12〜13）と d56e79e5（10/12）、全ツリー | **失敗 0**。どちらも 17,127 合格 / 未対応 34 / 品質警告 3 / 互換警告 1。fe5e8e73 のキューブ 25 件は全部合格。1 つのコミットの全ツリーで初めての失敗 0 | docs/board-sessions/pi4-es2-r1-tip.md、docs/board-sessions/pi4-es2-final-r2.md |
| Pi 4 | 20a48791（10/14）、行 0〜14 | 11,877 / **失敗 46** / 25 / 3 / 2（11,953 件）。FB1 3 件、FB2 43 件、すべて `GL_BGRA_EXT`。行 15〜28 は未実行（pi4-es2-final-b） | docs/board-sessions/pi4-es2-final.md |

- 46 件の原因（両方の記録）: 8bbf9ba2（10/12）の report-es2 のレンダーバッファ許可リストが `GL_BGRA8_EXT` だけを挙げ、dEQP-GLES2 が使う `GL_BGRA_EXT` を挙げていない。63efed8e、c345ab74、d56e79e5 はこのコミットを含まないので、失敗 0 の 3 つの全ツリーはこれを測っていない。担当の項目は記録にまだない（HANDOFF の Open work に「no issue yet」）。
- まだ測っていないもの: 8bbf9ba2 を含むコミットでの AM62P の全ツリー（am62p-es2-final-a、-b）、Pi 4 の行 15〜28（pi4-es2-final-b）、修正後の i.MX8MP の e21 と Pi 4 の FB1・FB2。es2-docs-final がまとめる。

## 10/11 のボード実測（main f2a13877 と 1cf946a9、nuc、tools/board.sh。記録は RESULTS.md の 2026-10-11 の各節）

main は f2a13877（2026-10-11）で統合の残り、Pi 4 のシリアルブリッジ用ランナー、GC7000 の RGBA16F、PowerVR の深度リゾルブ修正、AM62P の GLES3 の 28 画像の表を取り込み、1cf946a9 で V3D の非シームレスキューブの level 修正、集計の Lost / done_lossy、`check.sh --changed` / `--local`、land.sh の範囲限定ゲート、`.github/workflows/gate.yml`（自前の 2 台のランナーが main への push ごとと夜間に全行を走らせる。push 前のローカルのゲートは変更の範囲だけで約 10 分）を取り込んだ。

- **Pi 4: dEQP-GLES2 の失敗は 0。** 必須 17,165 件のうち **17,126 合格 / 失敗 0 / 未対応 34 / 警告 5（品質警告 3、互換警告 2）**（10/1 の fe5e8e73 は 17,075 / 25 / 62 / 3）。3 つの記録を合わせた数字で、1 つのコミットの全画像ではない: a27d69c0 の全ツリー（17,083 合格 / 失敗 9 / 未対応 34 / 品質警告 3 / 互換警告 2 / 結果なし 34、build/pi4-tally-a27d69c0.txt）、失敗 9 件（キューブの T3 の mipmap 6 件と T4 の size 3 件）が 1cf946a9 で全部合格（キューブのフラグメントの検索が変わった 6 行 RT、S5f、T2、T3、T4、TF で必須 934 件が 934 合格 / 失敗 0 / 取りこぼし 0、build/pi4-rows-tally-1cf946a9.txt）、結果なしの 34 件（F1 の blend、capture の欠落）は F1 の再走行で全部合格。gl_test は 12/12（1cf946a9、records/pi4/gt-1cf946a9）。未対応 34 の中身は multisample 16、negative_api.texture 9、clip_control 8、multisampled_render_to_texture.readpixels 1。27 件の fbo と fragdata は合格になった。
- **キャプチャの欠落（Pi 4、AM62P とも）。** usb1 の TCP シリアルブリッジは、接続が詰まると 1 まとまりの行を落とす（Pi 4 の F1 は一度目に 34 件（#379〜#412）、再走行で 260 件（#870〜#1129）、AM62P の GLES3 画像 25 は 51 件）。イメージは走らせているので、集計は NoResult ではなく Lost、ランナーは done ではなく done_lossy と読む。直すのはブリッジのファームウェア。
- **AM62P: glmark2（GM 画像、f2a13877）は 33 シーン / 失敗 0 / スコア 465、「did not resolve its depth」の行は 0**（以前は shadow と refract のフレームごとに 1 行）。深度リゾルブの修正（pvr-depth-resolve）のボードでの確認。リセット 0（records/am62p/gm-f2a13877）。
- **AM62P: dEQP-GLES3 の全ツリー（28 画像、f2a13877）。** 必須 42,810 件のうち **32,669 合格 / 失敗 5,549 / 未対応 2,318 / 品質警告 185 / 結果なし 2,089**（10/10 の c4ccc966 は 30,431 / 7,011 / 2,274 / 5 / 3,088）。失敗のグループ別: shaders 412（1,603 から）、texture 1,752（未対応 1,646）、fragment_ops 1,109、fbo 872、transform_feedback 640（未対応 191）、fragment_out 595、attribute_location 48、vertex_arrays 41、state_query 26、rasterization 21、draw 14、implementation_limits 11、instanced 6、lifetime 1、uniform_api 1。buffer 0 と read_pixels 0（300 と 24 から）。結果なしの内訳: 画像 5 の末尾 558（lifetime のあと、transform feedback の取り込み未実装で dEQP が打ち切り）、画像 7 の末尾 1,397（builtin_functions.precision.clamp の描画が GENERAL アリーナ 1 MiB に入らず ResourceError）、画像 20 と 21 の末尾 83、**Lost 51**（画像 25、transform_feedback.basic_types。ボードは走らせたので失敗にも NoResult にも数えない。イメージのカウンタでは失敗 35 / 未対応 16）。
  - **GPU がハングする 2 ケース: `texture.filtering.3d.formats.r11f_g11f_b10f_linear_mipmap_linear`（画像 20）と `texture.filtering.3d.sizes.3x7x5_linear_mipmap_linear`（画像 21）。** どちらも 13 ケースのあと 14 ケース目で止まり、ボードは RTI のリセットでしか戻らない（ランナーの判定は 602 s / 612 s で no_reset。RUN_BOUND=720 RETURN_BOUND=240 ならそれより長く待つ）。画像 11 は 4 回のリセットつきで最後まで走る（done_with_resets、2 回とも）。
  - 次の足場: 保留中の着地（work/land9: transform feedback の取り込み、glBlitFramebuffer の色、バッファのマップと PBO、PowerVR の sRGB）はホストだけで、ここに載せた数字は何も含まない。
- **i.MX8MP: gl_test の 44 番（es3hdr、#6）は f2a13877 でも cd738b1a でも失敗。** check 3 が 0x40042814 で、期待は 0x40043904。(5,0) の色が 8 画素幅のリニアな RGBA16F ターゲットの (7,0) に書かれている（64 ビット画素のピクセルエンジン側の置き方で、サンプラではない）。調べる gl_test 44 の GC7000 側の探りは保留中の着地にあり、ボードではまだ走らせていない（records/imx8mp/gc44-f2a13877）。
- **i.MX8MP: 共通セット（cd738b1a、2026-10-14、U-Boot 起動、records/imx8mp/common-cd738b1a）。** glmark2 は 43（33 シーン、失敗 0）、SO / SV / CL のプローブは合格、GT は 49 件中 48 件が合格（16 番は飛ばし、44 番が失敗、#6）。全行でリセット 0。

- **i.MX8MP: 共通セットを a63fcc5b（10/14）で実測。** glmark2 は **スコア 43（33 シーン実行 / 失敗 0 / 未対応 0）**、探りの SO・SV・CL はいずれも合格。gl_test は **49 件中 48 件合格**で、44 番（es3hdr、#6）が引き続き失敗、16 番（4x MSAA、#8）は未対応のままスキップ。新しい失敗ケースはない。記録は RESULTS.md の「i.MX8MP: common set on main a63fcc5b」の節。

## 10/10 のボード実測（main 32778b57、nuc、tools/board.sh、uboot-mmc。記録は RESULTS.md の 2026-10-10 の各節）

10/7 の変更（計画 #1、log/status-2026-09-25-to-10-07.md）を載せた main のイメージを、初めてボードで走らせた。AM62P と i.MX8MP だけ。Pi 4 は上の 10/11 の節。

- **AM62P: 失敗 0 ではない。45 + 2 が残り、どちらも原因は分かっている（前者）か診断中（後者）。** 全ツリー（17 枚、32778b57）は必須 17,165 件のうち 17,070 合格 / 失敗 54 / 未対応 34 / 品質警告 5 / 互換警告 2（flush_finish.flush と flush_wait）。失敗 54 は 10/2 の d60b9d76（失敗 0）からの退行で、すべて合格から悪くなった:
  - `shaders.invariance.{lowp,mediump,highp}.common_subexpression_{1,2,3}` 9 件（画像 1）。e17fb6a7 で直し、**画像 1 だけを e17fb6a7 で走らせて 1,379 合格 / 失敗 0、9 件とも合格を確認**。
  - `texture.vertex.2d.{filtering 36, wrap 9}` 45 件（画像 9、「Image comparison failed」）。原因は GENERAL アリーナを 1 MiB に縮めたため（dcaafa41）、dEQP の 1,225,804 バイトのクライアント配列の描画が断られていたこと。インデックスを分割して描く修正（a6fc6f14、gl_test 197）をホストで入れた。**画像 9 でのボード確認は未了。**
  - 4x 画像（ms4）の multisample 16 件は 14 合格 / 失敗 2（constancy_alpha_to_coverage と constancy_both_inverted、2 回走らせて同じ）。10/2 は 16/16 だった。alpha to coverage はフラグメントシェーダの sample_kill 版を通る（src/gl/draw.rs の needs_sample_kill）が、原因は未発見。診断中で issue はまだない。
  - 未対応 34 は clip_control 8、negative_api.texture.compressedtexsubimage2d_* 9、multisampled_render_to_texture.readpixels 1、multisample 16（1x の設定。上の ms4 で測る）。fragdata.invalid_assign_to_1 は合格になった（4e9397c）。
  - 合わせると（ms4 の結果で multisample 16 を置き換えて）17,084 合格 / 失敗 56 / 未対応 18 / 警告 7。vertex 修正と a2c の診断が済むまで失敗 0 には戻らない。
  - gl_test の既定セット: 48/48（タイムアウト 0、リセット 0、precompiled hits 0 / compiler runs 197）。
- **i.MX8MP: 失敗 46（すべて fbo の GL_BGRA_EXT レンダーバッファ）。** 全ツリー（29 セレクタ、835cf9cc、2026-10-14）は 17,371 件が走り、17,272 合格 / 失敗 46 / 未対応 34 / その他 19（品質警告）/ 結果なし 0。32778b57 の失敗 0 と比べて変わったのは e21（fbo）だけで、711 件中 665 合格 / 失敗 46。原因は 8bbf9ba2 の report-es2 のレンダーバッファ許可リストが GL_BGRA8_EXT だけを挙げ、dEQP-GLES2 が使う GL_BGRA_EXT を挙げていないこと（cd738b1a の記録と同じ 46 件、docs/board-sessions/imx8mp-es2-final.md）。未修正。e14 の 14 件、e27 の 5 件が警告、e22 の 9 件と e28 の 25 件が未対応で、これは前回と同じ。
  - gc_gl_test: 3 回走らせて 45/48（最初の失敗は 10 番、不確定の #9）、46/48 と 46/48（最初の失敗は 16 番、#8: 4x の半分を名前つきで飛ばし、セットの集計が飛ばしを失敗に数える。2 つ目の失敗はこの記録に名前がない）。8abeaabb の基準（46/48、16 番が最初）と同じ。
- 注: AM62P の dEQP-GLES3 を 32778b57 で試みたが結果は出なかった。イメージが ES 2.0 を報告していたため。`tools/cts/am62p-deqp-images.sh` が `-DCONFIG_YAKOGL_REPORT_ES2=n` を渡すように直した（測定ではない）。

## 開いている不具合（10/11 時点。順番と詳細は docs/HANDOFF.md「Open work」）

- **dEQP-GLES2、3 ボード共通（10/15 時点）**: `GL_BGRA_EXT` のレンダーバッファ 46 件（8bbf9ba2 の退行、未修正）。i.MX8MP と Pi 4 で失敗、AM62P は未測定。
- **AM62P、dEQP-GLES2**: `texture.vertex.2d` の 45 件（画像 9）と ms4 の `multisample.constancy_alpha_to_coverage`、`constancy_both_inverted` の 2 件は 63efed8e の全ツリーで合格（10/12、am62p-es2-final-r2.md）。以前は「ホストで修正済み、ボード確認待ち」。4x のフレームが 2 つのキックに分かれるとサンプルを失う欠陥（pvr-msaa-split）は残る。
- **AM62P、dEQP-GLES3**: 画像 20 と 21 の 3D テクスチャの 2 ケースで GPU がハングする。画像 7 は GENERAL アリーナ 1 MiB に入らない描画で打ち切り（ResourceError）。画像 5 は transform feedback の取り込みが未実装で打ち切り（保留中の着地はホストだけ）。
- **i.MX8MP**: gl_test 44 番（es3hdr、RGBA16F の描画先、#6）は f2a13877 でも cd738b1a でも失敗。gl_test 10 番と 2 番（#9、#25）はときどき失敗し、原因は未特定（tools/cts/log/investigations/gc-case10-flake.md）。16 番の 4x MSAA の経路（#8）は未解決。glmark2 のスコアが 37〜46 に留まるのは、ボード層が毎フレームパネルの VSYNC を待つため、という予測がある（glmark2 は待たない設定を求めている。コードを読んだ予測で、ボードでは未確認、修正 F1 も未適用。tools/cts/log/investigations/imx8mp-frame-floor.md）。
- **Pi 4**: dEQP-GLES2 は d56e79e5 の全ツリーで失敗 0（10/12）、20a48791 の行 0〜14 で `GL_BGRA_EXT` の 46 件が失敗（10/14）。GLES3 は未実行。
- **キャプチャの欠落（Pi 4、AM62P とも）**: シリアルブリッジが行を落とす。集計は Lost、ランナーは done_lossy と読む。直すのはブリッジのファームウェア。

## 注意

- 上の数字は Zephyr 上、内蔵コンパイラ。dEQP-GLES2 は ES 2.0 報告、dEQP-GLES3 は ES 3.0 報告のイメージ。
- 「未対応」（ES 2.0 にない機能、マルチサンプル構成なし）は失敗に数えていない。Lost（ボードは走らせたが結果の行が届かなかったケース）も失敗にも結果なしにも数えない。
