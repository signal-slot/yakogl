import urllib.request, json, base64

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
vulkancts = [e for e in tree(ext['sha']) if e['path'] == 'vulkancts'][0]
for e in tree(vulkancts['sha']):
    print('vulkancts:', e['type'], e['path'])
fw = [e for e in tree(vulkancts['sha']) if e['path'] == 'framework'][0]
vk = [e for e in tree(fw['sha']) if e['path'] == 'vulkan'][0]
print('== vulkancts/framework/vulkan')
for c in tree(vk['sha']):
    print('  ', c['type'], c['path'])
    if c['path'] == 'CMakeLists.txt':
        text = blob(c['sha'])
        for kw in ('vulkan-docs', 'Vulkan-Headers', 'find_package', 'video', 'graphicsfuzz'):
            for line in text.splitlines():
                if kw in line:
                    print('     |', line.strip()[:170])
mods = [e for e in tree(vulkancts['sha']) if e['path'] == 'modules'][0]
vkm = [e for e in tree(mods['sha']) if e['path'] == 'vulkan'][0]
print('== vulkancts/modules/vulkan (top-level entries)')
for c in tree(vkm['sha']):
    print('  ', c['type'], c['path'])
    if c['path'] == 'CMakeLists.txt':
        text = blob(c['sha'])
        for kw in ('video_generator', 'vulkan-video-samples', 'graphicsfuzz', 'add_subdirectory'):
            for line in text.splitlines():
                if kw in line:
                    print('     |', line.strip()[:170])
