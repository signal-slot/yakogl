| scene | FrameTime ms | FPS |
|---|---:|---:|
| [build] use-vbo=false | 5.00 | 200 |
| [build] use-vbo=true | 5.50 | 181 |
| [texture] texture-filter=nearest | 6.00 | 166 |
| [texture] texture-filter=linear | 6.50 | 153 |
| [texture] texture-filter=mipmap | 7.00 | 142 |
| [shading] shading=gouraud | 7.50 | 133 |
| [shading] shading=blinn-phong-inf | 8.00 | 125 |
| [shading] shading=phong | 8.50 | 117 |
| [shading] shading=cel | 9.00 | 111 |
| [bump] bump-render=high-poly | 9.50 | 105 |
| [bump] bump-render=normals | 10.00 | 100 |
| [bump] bump-render=height | 10.50 | 95 |
| [effect2d] kernel=0,1,0;1,-4,1;0,1,0; | 11.00 | 90 |
| [effect2d] kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1; | 11.50 | 86 |
| [pulsar] light=false:quads=5:texture=false | 12.00 | 83 |
| [desktop] blur-radius=5:effect=blur:passes=1:separable=true:windows=4 | 12.50 | 80 |
| [desktop] effect=shadow:windows=4 | 13.00 | 76 |
| [buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=map | 13.50 | 74 |
| [buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=subdata | 14.00 | 71 |
| [buffer] columns=200:interleave=true:update-dispersion=0.9:update-fraction=0.5:update-method=map | 14.50 | 68 |
| [ideas] speed=duration | 15.00 | 66 |
| [jellyfish] <default> | 15.50 | 64 |
| [terrain] <default> | 120.50 | 8 |
| [shadow] <default> | 40.25 | 24 |
| [refract] <default> | 60.10 | 16 |
| [conditionals] fragment-steps=0:vertex-steps=0 | 16.00 | 62 |
| [conditionals] fragment-steps=5:vertex-steps=0 | 16.50 | 60 |
| [conditionals] fragment-steps=0:vertex-steps=5 | 17.00 | 58 |
| [function] fragment-complexity=low:fragment-steps=5 | 17.50 | 57 |
| [function] fragment-complexity=medium:fragment-steps=5 | 18.00 | 55 |
| [loop] fragment-loop=false:fragment-steps=5:vertex-steps=5 | 18.50 | 54 |
| [loop] fragment-steps=5:fragment-uniform=false:vertex-steps=5 | 19.00 | 52 |
| [loop] fragment-steps=5:fragment-uniform=true:vertex-steps=5 | 19.50 | 51 |

glmark2 Score: 123
