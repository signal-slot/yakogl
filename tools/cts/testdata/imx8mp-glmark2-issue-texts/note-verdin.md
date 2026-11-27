<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# i.MX8MP (Verdin iMX8M Plus, Vivante GC7000UL): the 33 glmark2-es2 pictures, looked at by eye

Synthetic fixture for `tools/cts/imx8mp-glmark2-issue-texts-selftest.sh`: the verdict
table of a complete note, filled with fake values (commit `0123abcd`, records
`20261125-09NN-gmNN-x-0123abcd`, route `dump`); all verdicts `right`. Not a real record.

## Which board

All 33 rows were measured on the Verdin iMX8M Plus (i.MX8MP, GC7000UL);
the records carry `board=verdin`.

## Verdicts

| scene | commit | record | route | verdict |
|---|---|---|---|---|
| [build] use-vbo=false | 0123abcd | records/imx8mp/20261125-0901-gm01-x-0123abcd/ | dump | right |
| [build] use-vbo=true | 0123abcd | records/imx8mp/20261125-0902-gm02-x-0123abcd/ | dump | right |
| [texture] texture-filter=nearest | 0123abcd | records/imx8mp/20261125-0903-gm03-x-0123abcd/ | dump | right |
| [texture] texture-filter=linear | 0123abcd | records/imx8mp/20261125-0904-gm04-x-0123abcd/ | dump | right |
| [texture] texture-filter=mipmap | 0123abcd | records/imx8mp/20261125-0905-gm05-x-0123abcd/ | dump | right |
| [shading] shading=gouraud | 0123abcd | records/imx8mp/20261125-0906-gm06-x-0123abcd/ | dump | right |
| [shading] shading=blinn-phong-inf | 0123abcd | records/imx8mp/20261125-0907-gm07-x-0123abcd/ | dump | right |
| [shading] shading=phong | 0123abcd | records/imx8mp/20261125-0908-gm08-x-0123abcd/ | dump | right |
| [shading] shading=cel | 0123abcd | records/imx8mp/20261125-0909-gm09-x-0123abcd/ | dump | right |
| [bump] bump-render=high-poly | 0123abcd | records/imx8mp/20261125-0910-gm10-x-0123abcd/ | dump | right |
| [bump] bump-render=normals | 0123abcd | records/imx8mp/20261125-0911-gm11-x-0123abcd/ | dump | right |
| [bump] bump-render=height | 0123abcd | records/imx8mp/20261125-0912-gm12-x-0123abcd/ | dump | right |
| [effect2d] kernel=0,1,0;1,-4,1;0,1,0; | 0123abcd | records/imx8mp/20261125-0913-gm13-x-0123abcd/ | dump | right |
| [effect2d] kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1; | 0123abcd | records/imx8mp/20261125-0914-gm14-x-0123abcd/ | dump | right |
| [pulsar] light=false:quads=5:texture=false | 0123abcd | records/imx8mp/20261125-0915-gm15-x-0123abcd/ | dump | right |
| [desktop] blur-radius=5:effect=blur:passes=1:separable=true:windows=4 | 0123abcd | records/imx8mp/20261125-0916-gm16-x-0123abcd/ | dump | right |
| [desktop] effect=shadow:windows=4 | 0123abcd | records/imx8mp/20261125-0917-gm17-x-0123abcd/ | dump | right |
| [buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=map | 0123abcd | records/imx8mp/20261125-0918-gm18-x-0123abcd/ | dump | right |
| [buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=subdata | 0123abcd | records/imx8mp/20261125-0919-gm19-x-0123abcd/ | dump | right |
| [buffer] columns=200:interleave=true:update-dispersion=0.9:update-fraction=0.5:update-method=map | 0123abcd | records/imx8mp/20261125-0920-gm20-x-0123abcd/ | dump | right |
| [ideas] speed=duration | 0123abcd | records/imx8mp/20261125-0921-gm21-x-0123abcd/ | dump | right |
| [jellyfish] <default> | 0123abcd | records/imx8mp/20261125-0922-gm22-x-0123abcd/ | dump | right |
| [terrain] <default> | 0123abcd | records/imx8mp/20261125-0923-gm23-x-0123abcd/ | dump | right |
| [shadow] <default> | 0123abcd | records/imx8mp/20261125-0924-gm24-x-0123abcd/ | dump | right |
| [refract] <default> | 0123abcd | records/imx8mp/20261125-0925-gm25-x-0123abcd/ | dump | right |
| [conditionals] fragment-steps=0:vertex-steps=0 | 0123abcd | records/imx8mp/20261125-0926-gm26-x-0123abcd/ | dump | right |
| [conditionals] fragment-steps=5:vertex-steps=0 | 0123abcd | records/imx8mp/20261125-0927-gm27-x-0123abcd/ | dump | right |
| [conditionals] fragment-steps=0:vertex-steps=5 | 0123abcd | records/imx8mp/20261125-0928-gm28-x-0123abcd/ | dump | right |
| [function] fragment-complexity=low:fragment-steps=5 | 0123abcd | records/imx8mp/20261125-0929-gm29-x-0123abcd/ | dump | right |
| [function] fragment-complexity=medium:fragment-steps=5 | 0123abcd | records/imx8mp/20261125-0930-gm30-x-0123abcd/ | dump | right |
| [loop] fragment-loop=false:fragment-steps=5:vertex-steps=5 | 0123abcd | records/imx8mp/20261125-0931-gm31-x-0123abcd/ | dump | right |
| [loop] fragment-steps=5:fragment-uniform=false:vertex-steps=5 | 0123abcd | records/imx8mp/20261125-0932-gm32-x-0123abcd/ | dump | right |
| [loop] fragment-steps=5:fragment-uniform=true:vertex-steps=5 | 0123abcd | records/imx8mp/20261125-0933-gm33-x-0123abcd/ | dump | right |
