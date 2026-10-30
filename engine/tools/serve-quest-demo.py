#!/usr/bin/env python3
"""Serve ONLY the generated demo; optional temporary HTTPS link for Quest."""
import argparse
from functools import partial
from http.server import ThreadingHTTPServer, SimpleHTTPRequestHandler
from pathlib import Path
import shutil
import subprocess
import threading

parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--port',type=int,default=8787)
parser.add_argument('--public',action='store_true',help='Open a temporary public HTTPS tunnel through cloudflared (serves only build/quest-demo)')
args=parser.parse_args()
root=Path(__file__).resolve().parents[2]/'build/quest-demo'
if not (root/'index.html').is_file(): parser.error('Build first: python3 engine/tools/build-quest-demo.py')
class Handler(SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header('X-Content-Type-Options','nosniff')
        self.send_header('Referrer-Policy','no-referrer')
        self.send_header('Permissions-Policy','xr-spatial-tracking=(self)')
        self.send_header('Cache-Control','no-cache')
        super().end_headers()
server=ThreadingHTTPServer(('127.0.0.1',args.port),partial(Handler,directory=str(root)))
print(f'Demo: http://localhost:{args.port}',flush=True)
tunnel=None
try:
    if args.public:
        if not shutil.which('cloudflared'): parser.error('Install cloudflared or upload build/quest-demo to your own HTTPS static host')
        threading.Thread(target=server.serve_forever,daemon=True).start()
        tunnel=subprocess.Popen(['cloudflared','tunnel','--url',f'http://127.0.0.1:{args.port}','--no-autoupdate'])
        tunnel.wait()
    else: server.serve_forever()
except KeyboardInterrupt: pass
finally:
    if tunnel is not None and tunnel.poll() is None: tunnel.terminate();tunnel.wait()
    server.server_close()
