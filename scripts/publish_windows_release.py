"""Prepare signed release metadata or upload a reviewed release using existing Git credentials.

Never prints credentials or the updater private key. Run --prepare before committing/tagging;
run --publish only after pushing the version commit/tag. Releases start as drafts.
"""
import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import subprocess
import urllib.error
import urllib.parse
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
REPO = 'dongyan3721/YourAllIn1Helper'


def release_config():
    config = json.loads((ROOT / 'src-tauri/tauri.conf.json').read_text(encoding='utf-8'))
    windows = json.loads((ROOT / 'src-tauri/tauri.windows.conf.json').read_text(encoding='utf-8'))
    config['plugins'] = windows['plugins']
    return config


def credential():
    if os.environ.get('GITHUB_TOKEN'):
        return os.environ['GITHUB_TOKEN']
    result = subprocess.run(['git', 'credential', 'fill'], cwd=ROOT,
                            input='protocol=https\nhost=github.com\n\n',
                            text=True, capture_output=True, check=True)
    values = dict(line.split('=', 1) for line in result.stdout.splitlines() if '=' in line)
    return values['password']


def api(method, path, payload=None, *, binary=None):
    url = path if path.startswith('https://') else f'https://api.github.com/repos/{REPO}{path}'
    headers = {'Authorization': f'Bearer {credential()}', 'User-Agent': 'YourAllIn1Helper-release',
               'Accept': 'application/vnd.github+json', 'X-GitHub-Api-Version': '2022-11-28'}
    data = None
    if payload is not None:
        data = json.dumps(payload).encode()
        headers['Content-Type'] = 'application/json'
    if binary is not None:
        data = binary
        headers['Content-Type'] = 'application/octet-stream'
    with urllib.request.urlopen(urllib.request.Request(url, data=data, headers=headers, method=method), timeout=180) as response:
        body = response.read()
        return json.loads(body) if body else None


def prepare(msi, notes):
    config = release_config()
    version = config['version']
    tag = 'v' + version
    signature = Path(str(msi) + '.sig')
    if not msi.is_file() or not signature.is_file():
        raise SystemExit('MSI and its updater .sig must both exist')
    output = ROOT / 'dist' / tag
    output.mkdir(parents=True, exist_ok=True)
    import shutil
    name = f'All-in-One-Toolbox_{version}_windows_x64.msi'
    for src, dest in [(msi, name), (signature, name + '.sig')]:
        shutil.copy2(src, output / dest)
    manifest = {'version': version, 'notes': notes.read_text(encoding='utf-8'),
                'pub_date': dt.datetime.now(dt.timezone.utc).isoformat().replace('+00:00', 'Z'),
                'platforms': {'windows-x86_64': {
                    'url': f'https://github.com/{REPO}/releases/download/{tag}/{name}',
                    'signature': signature.read_text().strip()}}}
    serialized = json.dumps(manifest, ensure_ascii=False, indent=2) + '\n'
    (output / 'latest.json').write_text(serialized, encoding='utf-8')
    feed = ROOT / 'updates/windows-beta.json'
    feed.parent.mkdir(exist_ok=True)
    feed.write_text(serialized, encoding='utf-8')
    (output / 'updater.pub').write_text(config['plugins']['updater']['pubkey'] + '\n', encoding='utf-8')
    shutil.copy2(notes, output / 'RELEASE_NOTES.md')
    sums = '\n'.join(f'{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}'
                     for path in sorted(output.iterdir()) if path.name != 'SHA256SUMS.txt') + '\n'
    (output / 'SHA256SUMS.txt').write_text(sums, encoding='utf-8')
    print(output)


def publish(finalize):
    config = release_config()
    tag = 'v' + config['version']
    output = ROOT / 'dist' / tag
    body = (output / 'RELEASE_NOTES.md').read_text(encoding='utf-8')
    local_sha = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    remote = api('GET', '/git/ref/tags/' + tag)
    if remote['object']['type'] == 'tag':
        remote = {'object': api('GET', '/git/tags/' + remote['object']['sha'])['object']}
    if remote['object']['sha'] != local_sha:
        raise SystemExit('The pushed release tag must point at the current version commit')
    try:
        release = api('GET', '/releases/tags/' + tag)
    except urllib.error.HTTPError as error:
        if error.code != 404:
            raise
        release = api('POST', '/releases', {'tag_name': tag, 'target_commitish': local_sha,
                      'name': config['version'].replace('-beta', '.beta'), 'body': body,
                      'draft': True, 'prerelease': True})
    if not release['draft']:
        raise SystemExit('Refusing to replace assets on an already published release')
    existing = {asset['name'] for asset in release['assets']}
    upload_url = release['upload_url'].split('{')[0]
    for file in sorted(output.iterdir()):
        if file.name not in existing:
            asset = api('POST', upload_url + '?' + urllib.parse.urlencode({'name': file.name}), binary=file.read_bytes())
            if asset['size'] != file.stat().st_size:
                raise SystemExit('Uploaded asset size mismatch: ' + file.name)
            print('Uploaded:', file.name)
    if finalize:
        release = api('PATCH', f'/releases/{release["id"]}', {'draft': False, 'prerelease': True, 'make_latest': 'false'})
    print(release['html_url'])


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--prepare', type=Path)
    parser.add_argument('--notes', type=Path)
    parser.add_argument('--publish', action='store_true')
    parser.add_argument('--finalize', action='store_true')
    args = parser.parse_args()
    if args.prepare:
        if not args.notes:
            parser.error('--prepare requires --notes')
        prepare(args.prepare.resolve(), args.notes.resolve())
    if args.publish:
        publish(args.finalize)
