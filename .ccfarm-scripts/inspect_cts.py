import urllib.request, json, base64, sys

API = 'https://api.github.com/repos/KhronosGroup/VK-GL-CTS/git'

def get(url):
    req = urllib.request.Request(url, headers={'User-Agent': 'yakogl-stage1'})
    return json.load(urllib.request.urlopen(req, timeout=30))

def tree(sha):
    return get(API + '/trees/' + sha)['tree']

def blob(sha):
    return base64.b64decode(get(API + '/blobs/' + sha)['content']).decode()

root = tree('93bca01861b0e3ef3c387027a9791e6d065f900c')
ext = [e for e in root if e['path'] == 'external'][0]
targets = {'vulkan-validationlayers', 'vulkancts', 'amber', 'vulkansc-pcutil', 'spirv-tools', 'glslang', 'jsoncpp', 'zlib', 'libpng', 'video_generator', 'vulkan-video-samples', 'openglcts'}
for e in tree(ext['sha']):
    if e['type'] == 'tree' and e['path'] in targets:
        print('== external/' + e['path'])
        for c in tree(e['sha']):
            print('  ', c['type'], c['path'])
            if c['path'] == 'CMakeLists.txt':
                text = blob(c['sha'])
                for kw in ('vulkan-docs', 'src', 'EXISTS'):
                    for line in text.splitlines():
                        if kw in line:
                            print('     |', line.strip()[:160])
