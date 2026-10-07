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
fw = [e for e in tree(vulkancts['sha']) if e['path'] == 'framework'][0]
vk = [e for e in tree(fw['sha']) if e['path'] == 'vulkan'][0]
cm = [e for e in tree(vk['sha']) if e['path'] == 'CMakeLists.txt'][0]
text = blob(cm['sha'])
lines = text.splitlines()
for i, line in enumerate(lines):
    if 'X11' in line or 'XCB' in line or 'vulkan-docs' in line or 'INL_GEN' in line:
        lo = max(0, i - 12)
        hi = min(len(lines), i + 6)
        print('--- context around line', i)
        print('\n'.join(lines[lo:hi]))
