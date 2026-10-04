---
title: Zephyr から PowerVR GPU (BXS-4-64) を素手で叩く #1（DDK ファームウェアの移植とパイプライン完動まで）
tags: PowerVR, Zephyr, GPU, AM62P, 組み込み
private: false
---

## このシリーズについて

Toradex Verdin AM62P（TI AM62P、PowerVR BXS-4-64 GPU 内蔵）で、A53 コアを Linux ではなく Zephyr で動かしたまま実 GPU でレンダリングするための、自作ドライバの開発記録である。

ソフトウェアレンダリングは使わない。
最終目標は Slint UI を実 GPU で描くことだが、その手前の「三角形1枚を実 GPU でタイリングしてラスタライズする」までが本シリーズの主題になる。

作業量が大きいのでシリーズにする。
第1回は、ファームウェア（以下 FW）が起動すらしなかった状態から、GEOM から 3D までのレンダリングパイプラインがハードウェア障害ゼロで完走するところまでの記録である。
まだ三角形は出ていない。
geom（ジオメトリ処理）がプリミティブを 1 つもタイリングしないところで止まっており、その切り分けの現在地までを書く。

この記事は、読んだ人が同じ手順を踏めることを目標にする。
使ったツールの中身、コンパイルコマンド、実機からの採取方法、動作中 DDK との突き合わせ方を、省略せずに載せる。

## 対象環境とバージョン

再現の前提になる構成を先に固定しておく。

- ボード：Toradex Verdin AM62P（TI AM62P、eMMC）
- OS（デバッグ母艦）：Torizon OS、kernel 6.6.138
- GPU：PowerVR BXS-4-64、BVNC 36.53.104.796、RISC-V FW コア、XE アーキ
- DDK：TI/Img 版 25.2（`rgx.fw.36.53.104.796`、`libpvr_dri_support.so.25.2`）
- 自作ドライバ：`powervr-bxs-am62p`（signal-slot org の private リポジトリ）

自作ドライバのリポジトリは次のレイアウトになっている。
以降の手順はこのパスを指す。

- `src/`：ドライバ本体（Zephyr 用のハンドライト、Linux KO としてもビルドする）
- `tools/pvrlab/`：Linux カーネルモジュールとしてビルドして実機に載せるデバッグハーネス
- `tools/offsetof/`：DDK ヘッダを実 config でコンパイルして構造体レイアウトを吐くダンパ
- `tools/sfdecode/`：FW トレースを人間可読にするデコーダ
- `docs/`：この記事を含むドキュメント

## なぜ DDK ファームウェアのパスなのか

PowerVR には2系統の FW がある。

- **オープンソース FW**：mainline の `drm/imagination` が使う
- **DDK FW**：TI/Img のベンダ `pvrsrvkm` が使うクローズドな FW

この silicon でタイリングが実証されている構成は DDK FW だけだった。
ベンダの `rgx_triangle_test` が同じボード上で 59 FPS で回る（後述する）。
一方、オープン FW とハンドライトの組み合わせでは geom が 0 プリミティブに終わり、オープン FW と mainline KM の組み合わせは probe で SoC が panic する。
そこで、DDK FW を自作ドライバから正しい FWIF ABI で叩く方針にした。

DDK のカーネル側ソースは公開されている（`ti-img-rogue-driver`）。
FW バイナリと userspace はクローズドだが、FWIF ABI（ホストと FW が共有する構造体定義）はカーネル側ヘッダに全部ある。
出発点は、公開ソースを読んで FWIF ABI を忠実に再構築すれば FW は正しく解釈するはずだ、というものだった。

## デバッグの型：推論せず実測とソースで詰める

このプロジェクトで効いたのは、次の型を最初に決めたことだった。

ABI は DDK ソースを `offsetof` でコンパイルして確定する。
FW の内部進行は FW トレースをデコードして読む。
到達点は動作中の DDK と実機レジスタで確認する。
そして公開情報からの推論より実測を優先する。

以降、この4つをそれぞれ再現できる形で示す。

## FWIF ABI を offsetof で確定する

FWIF 構造体のレイアウトは、DDK ヘッダを実際にコンパイルして `offsetof` と `sizeof` を出力すれば確定する。
問題は、どの config でコンパイルするかである。

DDK のソースツリーには複数のビルド config がある。
FW が実際に読むレイアウトは、その FW を焼いた config でコンパイルしたときのものだけが正しい。
`copy-to-kernel-tc` config でコンパイルすると `RGXFWIF_KCCB_CMD` は 80 バイト、`RGXFWIF_FWCOMMONCONTEXT` は 336 バイトになる。
ところが、このボードに焼かれている FW の config（`binary_am62p_linux_nulldrmws_release/config_kernel.h`）でコンパイルすると、それぞれ 160 バイト、456 バイトだった。
KM 側 config の値を信じて実装すると、FW から見て全フィールドがずれる。

ダンパは短い C で書く。
測りたい構造体の各フィールドに `offsetof` を並べるだけである。

```c
/* tools/offsetof/dumpcc.c */
#include <stdio.h>
#include <stddef.h>
#include "rgx_fwif_km.h"
#define O(s,f) printf("  %-40s off=%4zu\n", #f, offsetof(s,f))

int main(void){
  printf("sizeof(RGXFWIF_KCCB_CMD)=%zu\n", sizeof(RGXFWIF_KCCB_CMD));
  O(RGXFWIF_KCCB_CMD, eCmdType);
  O(RGXFWIF_KCCB_CMD, ui32KCCBFlags);
  O(RGXFWIF_KCCB_CMD, uCmdData);

  printf("sizeof(RGXFWIF_FWCOMMONCONTEXT)=%zu\n", sizeof(RGXFWIF_FWCOMMONCONTEXT));
  O(RGXFWIF_FWCOMMONCONTEXT, psContextState);
  O(RGXFWIF_FWCOMMONCONTEXT, ui32CompatFlags);
  O(RGXFWIF_FWCOMMONCONTEXT, ui32MaxDeadlineMS);
  O(RGXFWIF_FWCOMMONCONTEXT, ui32PID);
  /* 以下、確認したいフィールドを並べる */
  return 0;
}
```

コンパイルは、FW の config を `-include` で先頭に差し込み、DDK のインクルードパスを通す。
`-D` は config が要求する FW ヒープ関連のマクロで、これが無いとヘッダが展開できない。

```sh
DDK=/path/to/ti-img-rogue-driver
CFG=$DDK/binary_am62p_linux_nulldrmws_release

gcc -I$CFG \
    -I$DDK/include -I$DDK/include/rogue -I$DDK/include/public \
    -I$DDK/hwdefs/rogue/km -I$DDK/hwdefs/rogue \
    -I$DDK/services/include -I$DDK/services/include/rogue \
    -I$DDK/services/shared/include -I$DDK/generated/rogue_km \
    -include config_kernel.h \
    -DRGX_FW_HEAP_OSID_ASSIGNMENT=0 \
    -DRGX_FW_HEAP_USES_FIRMWARE_OSID=0 \
    -DPVRSRV_HWPERF_COUNTERS_PERBLK=12 \
    tools/offsetof/dumpcc.c -o /tmp/dumpcc && /tmp/dumpcc
```

出力の先頭はこうなる。

```
sizeof(RGXFWIF_KCCB_CMD)=160
...
sizeof(RGXFWIF_FWCOMMONCONTEXT)=456
```

この 160 と 456 が、FW が実際に読むストライドとサイズである。
自作側の構造体は、この出力に合わせて `#ifdef PVR_DDK_FW` でレイアウトを切り替える。

## FW トレースを人間可読にする

FW は内部の進行を circular buffer にログする。
各エントリは SFID（32bit のメッセージ ID）と 64bit タイムスタンプと引数の並びである。
SFID はグループ番号とメッセージ番号と引数個数から次式でエンコードされる。

```
sfid = num | ((grp & 0x10) << 7) | ((grp & 0xF) << 12) | (params << 16) | 0x70000000
```

デコーダは、DDK の `rgx_fwif_sf.h` にある SFID 定義表を読んで、この式で ID を再構成し、ID からフォーマット文字列への辞書を作る。
そのうえで実機から吸い出した生 dword 列を頭から走査し、`0x700xxxxx` に一致する語を SFID とみなして、続く 2 語をタイムスタンプ、その後を引数として取り出す。

```python
# tools/sfdecode/sfdecode.py（要点）
import re, sys
hdr = open(SRC + "/include/rgx_fwif_sf.h").read()

groups = {}
for i, m in enumerate(re.findall(r'X\(RGXFW_GROUP_\w+,\s*(\w+)\)', hdr)):
    groups["RGXFW_GROUP_" + m] = i

body = hdr.split("RGXFW_LOG_SFIDLIST")[1].replace("\\\n", " ")
sfs = {}
for m in re.finditer(r'X\(\s*(\d+)\s*,\s*(RGXFW_GROUP_\w+)\s*,\s*\w+\s*,\s*"((?:[^"\\]|\\.)*)"\s*,\s*(\d+)\s*\)', body):
    a, grp, s, e = int(m[1]), m[2], m[3], int(m[4])
    g = groups[grp]
    sfid = (a | ((g & 0x10) << 7) | ((g & 0xF) << 12) | (e << 16) | 0x70000000) & 0xffffffff
    sfs[sfid] = (s, e)

data = [int(t, 16) for line in sys.stdin for t in re.findall(r'[0-9a-fA-F]{8}', line)]
i = 0
while i < len(data):
    d = data[i]
    if (d & 0xFFF00000) == 0x70000000 and d in sfs:
        fmt, argn = sfs[d]
        args = data[i+3:i+3+argn]
        print(fmt % tuple(args) if argn else fmt)
        i += 3 + argn
    else:
        i += 1
```

生トレースは自作ドライバが dmesg に吐く。
`TracePointer`（現在の書き込み位置）と `WrapCount` と `SizeInDwords` に続けて、`t[]` として dword を並べる。

```
pvrgpu: DDK FW trace: TracePointer=9886 WrapCount=1047 SizeInDwords=12000
pvrgpu:  t[0]: 00000000 7001105c 00000000 01f67ca9 00000001 7001700d
...
```

この `t[...]` 行を切り出してデコーダに渡すと、FW の進行が読める。

```sh
grep 'pvrgpu:  t\[' ours_dmesg.txt | python3 tools/sfdecode/sfdecode.py
```

デコードすると、DDK の実機トレースと同じ書式（`T0-RTD>` や `T0-PM>` で始まる行）で読めるようになる。
これで FW が今どの段にいるかが分かり、以降の切り分けが実測ベースになる。

## ビルドとデプロイと採取のループ

自作ドライバは Zephyr 用だが、切り分けは Linux カーネルモジュールとしてビルドし、Torizon 上で `insmod` して行う。
ドライバのコア `.c` をそのまま Linux KO に組み込む薄いハーネスが `tools/pvrlab/` である。

`Kbuild` はドライバの `.o` を並べ、Zephyr との差分を shim で吸収し、DDK パスを2つのフラグで選ぶだけになっている。

```make
obj-m += pvrgpu.o
pvrgpu-y := pvr_mod.o pvr_fw_boot.o pvr_pds_render_gen.o pvr_vdm_ppp_gen.o \
            pvr_pds_pixel_event_gen.o pvr_isp_ctrl_stream_gen.o pvr_fw_trace_decode.o
ccflags-y := -I$(src)/shim -I$(SRC_DIR) -DPVR_LINUX_MODULE \
             -include $(src)/shim/pvr_compat.h -Wno-declaration-after-statement
```

ビルドは、母艦のクロスコンパイラと、ボードのカーネルに一致するツリーに対して行う。
DDK FW パスを選ぶフラグは `-DPVR_DDK_FW` と `-DPVR_BIG_GLOBAL_FL` の2つである。

```sh
make -C $KDIR M=$PWD ARCH=arm64 CROSS_COMPILE=$CROSS \
     EXTRA_CFLAGS="-DPVR_DDK_FW -DPVR_BIG_GLOBAL_FL" modules
```

デプロイと採取は1コマンドにまとめる。
リング buffer を消してから積み替え、FW を起動させ、dmesg を回収する。
接続先はハードコードせず環境変数にする（ボードの IP や sudo パスワードをスクリプトに埋めない）。

```sh
scp pvrgpu.ko "$BOARD_HOST":/tmp/
ssh "$BOARD_HOST" 'sudo sh -c "dmesg -C; rmmod pvrgpu 2>/dev/null; \
                   insmod /tmp/pvrgpu.ko do_boot=1; sleep 3; dmesg"' > ours_dmesg.txt
```

このループを回すと、1 回のリロードで FW 起動から geom kick、シグネチャダンプ、FW トレースまでが `ours_dmesg.txt` に入る。

## 動作中の DDK を ground truth にする

このボードには TI 純正の DDK が入っている。
自作ドライバの出力を、動いている DDK の出力と突き合わせられる。

- KM：`/lib/modules/$(uname -r)/updates/pvrsrvkm.ko`
- FW：`/lib/firmware/rgx.fw.36.53.104.796`、`/lib/firmware/rgx.sh.36.53.104.796`
- テスト：`/usr/bin/rgx_triangle_test`、`/usr/bin/rgx_compute_test` ほか

`rgx_triangle_test` を走らせると、DRM/KMS に 1024x600 で 59 FPS の三角形が出る。
これが「この silicon でタイリングが成立している」証拠になる。

```
Selected display mode: 1024x600@59
FPS: 59.0
```

このとき FW が吐くトレースを回収すると、geom が成功している 1 フレームの中身がそのまま読める。
自作側で同じ手順（`tools/sfdecode/`）でデコードしておけば、行単位で比較できる。

DDK の geom（TA）サイクルは次の順で進む。
各行の値の意味を押さえておくと、自作側のどこが違うかが分かる。

```
Load Freelist ... for PMDM0: FL Total Pages 26 (max=32,grow size=0),
                  FL-addr = 0x000000fa00008010, stacktop = 0x19, Alloc Page Count = 0
CONTEXT_PB_BASE set to 0x2, FL different between TA/3D: local: 1, global: 0
Is TA: 1, finished: 0 on HW 0 (HWRTData = 0xc0104500, MemCtx = 0xc0104000)
ALIST0 SP = 0, MLIST0 SP = 0 (VCE ..., TE ..., ALIST ...)
Kick TA: FWCtx 0xc0104040 @ 4368, RTD 0xc0104500, First kick:1, Last kick:1
Is TA: 1, finished: 1
ALIST0 SP = 3, MLIST0 SP = 9 (VCE 0xb94f3001, TE 0xb94f1001, ALIST 0xb95c5001)
Store Freelist ... stacktop = 0x08, Alloc Page Count = 8, Alloc MMU Page Count = 9
```

読み方は次のとおり。

- **Load Freelist**：geom が使う freelist を PM（Parameter Manager）に載せる。`stacktop` は空きページのスタック先頭で、ここでは 0x19（25）。
- **CONTEXT_PB_BASE set to 0x2**：geom 用にパラメータバッファのベースが設定された。3D 側は 0x0 になる。
- **Is TA: 1, finished: 0 から 1**：geom が開始し、完走した。
- **ALIST0 / MLIST0 SP が 0 から 3 / 9**：VCE と TE がプリミティブを emit した結果、リストのスタックポインタが進んだ。
- **Store Freelist の Alloc Page Count = 8**：geom が freelist から 8 ページを確保した。

geom がタイリングした、という判定はこの `Alloc Page Count > 0` と `MLIST0 SP > 0` で下せる。

## 当初の想定と実際の差

ここは「バグ修正」ではない。
まだ何も表示できていない、書きかけのドライバである。
公開ソースから ABI を組んでみて、実機やソースと突き合わせて当初の想定と実際の差を埋めていく開発作業である。
そこで、当初どう実装していたか（想定）と、実測やソースで分かった実際とを並べる。
確定手段はいずれも、DDK ソースの `offsetof`、実機実測、動作中 DDK のキャプチャのどれかである。

| 対象 | 当初の実装（想定） | 実測やソースで分かった実際と、そのとき起きた挙動 | 確定手段 |
|---|---|---|---|
| `RGXFWIF_CCB_CTL`（KCCB/FWCCB） | 128B のキャッシュライン padding 付きで、WriteOffset/ReadOffset/WrapMask を padding 込みに配置すると想定 | compact で WriteOffset@0, ReadOffset@4, WrapMask@8。padding も cmd_size も無い。想定レイアウトだと WrapMask を @8 の 0 と読み、read=(0+1)&0=0 で slot 0 を無限再実行していた | DDK source |
| KCCB/FWCCB コマンドの stride | 88 バイトと想定 | 160 バイト（union 最大メンバ `FREELISTS_RECONSTRUCTION_DATA`=152）。88 で書くと slot 1 以降が garbage になり、FW_FAULT から GPU_RESTART | binary_am62p の offsetof、実機 fault の offset 0xa0=160 |
| `RGXFWIF_CCB_CMD_HEADER` のサイズ | 40 バイト（WORKLOAD_ESTIMATION 込みと想定） | 16 バイト（この build に WORKLOAD_ESTIMATION 無し）。40 だと全 CCCB コマンドの本体位置が 24 バイトずれる | offsetof |
| `RGXFWIF_RUNTIME_CFG` の CoreClock/SOCClock | CoreClockSpeed/SOCClockSpeed を別 offset に置いていた | @12/@16。誤 offset だと FW が "Core clock 0 Hz" と読み、deadline scheduling が働かず DM が未スケジュール。800MHz（`clk_summary` 実測）で確定 | offsetof と実測 |
| `RGXFWIF_HWRTDATA` のレイアウト | OSS 版（PM/TE/VCE cat-base 配列が [4]）と想定 | 非 OSS 版（配列 [1]、freelists[3]、eState@76、sizeof=256）。OSS 版だと eState 位置がずれ、FW が非 kickable と誤判定 | offsetof |
| CCCB の write_offset | ホストが write_offset を直書きしてから KICK すればよいと想定 | FW は「ctl の現在値と KICK の CWoffUpdate の差分」で新規 work を検出する。直書きすると差分ゼロで work 無しと判定され、KICK は consume されるのに DM がスケジュールされない | 実測 |
| geom コマンド本体（`RGXFWIF_CMDTA`）の sGeomRegs | 公開 mainline の 64 バイト `geom_regs` レイアウトと想定 | 64 バイトのコマンドで sGeomRegs=32 バイトの compact。mainline 版に合わせると ppp_ctrl/te_psg が誤位置に載り、タイリングベースが 0 になって BIF page fault(VA=0) | 動作中 DDK の live capture |
| `RGXFWIF_FREELIST` の `ui64FreeListID` | u32 と想定 | u64@40。u32 だと offset 40 以降が全てずれる | offsetof |

このほか、align-checks 配列、osinit の各 FW アドレス、HWPerf バッファ、RISC-V FWCORE remap のバイト一致なども、同様に実測とソースへ合わせ込んでいる。

2つ、方法論として効いた点を残しておく。

FW を焼いた config で測ること。
実機の fault アドレス（`Req Status` の offset 0xa0=160）から stride は 160 だと気付き、config を替えて全 ABI を測り直した。
KM 側 config の 80/336 を信じていた間は、直しても直しても別の場所がずれ続けた。

live capture が推論に勝つこと。
geom コマンドの構造体はクライアント側ヘッダにあり、KM ソースには含まれない。
公開 mainline から読むと sGeomRegs は 64 バイトに見えるが、動作中 DDK に `printk` を仕込んで実コマンドをダンプすると 32 バイトだった。
32 バイトに合わせた時点で、タイリングベース 0 による page fault が消えた。

## fault 注入でパイプラインの生存段を切り分ける

geom がタイリングしないとき、GPU がどの段まで実行しているかは、fault を意図的に起こして確かめられる。
ある段が参照するアドレスを未マップの番地に差し替えておき、そこで BIF page fault が起きれば、その段は動いている。

ドライバに2つのノブを用意した。
1つは頂点 DMA の転送元、もう1つは VS（頂点シェーダ）の USC 実行アドレスである。
どちらも、注入した番地ちょうどで GUILTY の BIF fault が出た。

| 注入 | 結果 | 意味 |
|---|---|---|
| 頂点 DMA 元 を未マップ(0xE800000000)へ | その番地で BIF fault（GUILTY） | 頂点 DMA（PDS DDMAD）が動作し、頂点をフェッチしている |
| VS の USC を未マップ(0xE008000000)へ | その番地で BIF fault（GUILTY） | VS が dispatch され、USC がシェーダをフェッチしている |

VDM から PDS 頂点 DMA、VS dispatch までは生きている、とここまでで確定できる。

## 現在の課題：VCE が emit せず PM_CAT_BASE が 0 のまま

自作ドライバの clean build（fault 注入なし）でも、パイプラインはハードウェア障害ゼロで走る。
BIF fault は出ない。
それでも geom はプリミティブを 1 つもタイリングしない。

geom kick 中にシグネチャレジスタを読むと、どの段まで信号が出ているかが分かる。

```
GEOMSIG: PM_CAT_VCE0=0x0 TE0=0x0 ALIST0=0x0  TE_SIG=0x0 VCE=0x0
         PPP_SIG=0x475643e8 USC_UVS0=0xd1bb9704  EVENT=0x10
```

`PPP_SIG` と `USC_UVS0` が非ゼロである。
VS が実行され、PPP（プリミティブ処理）まで信号が届いている。
一方 `VCE`、`TE_SIG`、そして PM のカタログベース `PM_CAT_VCE0/TE0/ALIST0` はすべて 0 のままである。

ハング後にもう一度読んでも、PM カタログベースは 0 で動かない。

```
HANGPROBE: PM_CAT_VCE0=0 PM_CAT_TE0=0 PM_CAT_ALIST0=0 VDM_CTX_STORE_STATUS=0x1
TA-out mlist  nz=0/1024
```

MLIST は 1 語も書かれておらず（`nz=0/1024`）、TA は完走しない。
HWR は dm=2（geom）で type=0、つまり page fault ではなく deadline による lockup である。

動作中 DDK の同じ箇所と対比すると、差がはっきりする。
DDK では geom 後に `PM_CAT` 系が実アドレス（VCE=0xb94f3001 など）になり、`Alloc Page Count` が 0 から 8 に進む。
自作側はここが 0 のまま動かない。

PM のカタログベースは freelist の状態から引かれる。
VCE がプリミティブを emit する先のページは、PM が freelist から確保して `PM_CAT_BASE_VCE0` に載せる。
そのレジスタが 0 のままということは、PM が freelist からページを取れていない、あるいは geom 用の freelist を PM に結び付けられていない。
現時点の局所化はここである。
停止段は PPP から VCE への emit で、疑いは freelist と PM の結合（およびパラメータバッファのベース）にある。
根拠は `PM_CAT_VCE0=0` という実測で、原因の断定はまだしない。

ABI、render payload、タイリングレジスタ、freelist のロードフィールドは、いずれも動作中 DDK と実機実測で ground truth に一致させ済みで、候補から外している。

## まとめと次回

第1回の到達点は、FW が起動すらしなかった状態から、DDK FW を自作ドライバで叩いて GEOM から 3D のパイプラインをハードウェア障害ゼロで完走させたところまでである。
自作 ABI の当初の想定と実際の差は、その過程で実測とソースを ground truth に一つずつ埋めた。
ただし geom はまだ 0 プリミティブで、三角形は出ていない。

再現の要点は4つに絞れる。

- FW ABI は、FW を焼いた config でヘッダをコンパイルして `offsetof` で測る。
- FW トレースは SFID をデコードして人間可読にし、進行を実測で追う。
- 動作中の DDK（`rgx_triangle_test`）を同じボードで走らせ、成功しているフレームと行単位で突き合わせる。
- どの段まで生きているかは、未マップ番地への fault 注入で切り分ける。

第2回は、この最後の壁（PM と freelist の結合、パラメータバッファのベース）を扱う。
`PM_CAT_VCE0` が実アドレスになり `Alloc Page Count > 0` になれば geom がタイリングし、三角形が点灯する。
そこから先で Slint 接続に進む。
本シリーズは作業の進捗に合わせて追記していく。
