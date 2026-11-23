import contextlib
import gzip
import io
import json
from pathlib import Path
import shutil
import tempfile
import unittest

from cache_web_shelf import CONFIG_TAG, prepare


class WebShelfCacheTests(unittest.TestCase):
    def test_existing_install_migrates_and_the_next_release_keeps_asset_keys(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            previous = root/'previous'
            files = {
                'version.json': '{"version":"old"}',
                'index.html': '<html data-version="old"><head><meta charset="utf-8"></head><script>async function wasm() {}</script></html>',
                'pkg/browser.js': 'export default 1',
                'pkg/browser_bg.wasm': 'a' * 12000,
                'stories/editor.js': "new Worker(new URL('./worker.js',import.meta.url),{}); const m={version:document.documentElement.dataset.version}",
                'stories/worker.js': 'self.onmessage=()=>{}',
                'games/index.json': '["city"]',
                'games/city/files.json': '["main.luau"]',
                'games/city/main.luau': 'local x=4\n' * 1200,
            }
            for name, text in files.items():
                path = previous/name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text)

            def release(old, version, edit=False):
                stage = root/version
                shutil.copytree(old, stage)
                (stage/'version.json').write_text(json.dumps({'version': version}))
                prior = json.loads((old/'version.json').read_text())['version']
                page = stage/'index.html'
                page.write_text(page.read_text().replace('data-version="'+prior+'"','data-version="'+version+'"'))
                if edit:
                    (stage/'games/city/files.json').write_text('["main.luau","new.glb"]')
                    (stage/'games/city/new.glb').write_bytes(b'new asset' * 1500)
                with contextlib.redirect_stdout(io.StringIO()):
                    report = prepare(stage, old)
                cfg = json.loads(page.read_text().split(CONFIG_TAG,1)[1].split('</script>',1)[0])
                return stage, cfg, report

            one, first, summary = release(previous,'one',True)
            self.assertEqual(first['keys']['pkg/browser_bg.wasm'],'old')
            self.assertEqual(first['keys']['stories/worker.js'],'')
            self.assertEqual(first['keys']['games/city/main.luau'],'old')
            self.assertEqual(len(first['keys']['games/city/files.json']),64)
            self.assertEqual(len(first['keys']['games/city/new.glb']),64)
            self.assertEqual(summary['changed_game_files'],['games/city/new.glb'])
            for name, (key, size, wire, _) in first['compressed'].items():
                packed = (one/('pkg/compressed/'+key+'.gz')).read_bytes()
                self.assertEqual(len(packed),wire)
                self.assertEqual(len(gzip.decompress(packed)),size)
                self.assertEqual(gzip.decompress(packed),(one/name).read_bytes())
            _, second, _ = release(one,'two')
            self.assertEqual(first, second)
            draft = one/'games/city/main.luau'
            original = draft.read_bytes()
            # Changed bytes behind a previously stable path must receive a different key.
            stage = root/'three'
            shutil.copytree(one,stage)
            (stage/'games/city/main.luau').write_bytes(original+b'-- new')
            (stage/'version.json').write_text('{"version":"three"}')
            page=stage/'index.html'
            page.write_text(page.read_text().replace('data-version="one"','data-version="three"'))
            with contextlib.redirect_stdout(io.StringIO()): prepare(stage,one)
            keys=json.loads(page.read_text().split(CONFIG_TAG,1)[1].split('</script>',1)[0])['keys']
            self.assertNotEqual(keys['games/city/main.luau'],first['keys']['games/city/main.luau'])
            self.assertEqual(keys['games/city/new.glb'],first['keys']['games/city/new.glb'])


if __name__ == '__main__':
    unittest.main()
