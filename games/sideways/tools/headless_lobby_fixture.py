"""Authenticated, invisible replacement lobby for an isolated daemon test.

It receives real runtime-owned controller snapshots. It never synthesizes or
publishes controller input and must only be launched by the isolated harness.
"""
import argparse
import asyncio
import json
import os
from pathlib import Path
import websockets


async def run(out):
    address=os.environ['GAMENIGHT_ADDR']
    game=os.environ['GAMENIGHT_GAME_ID']
    token=os.environ['GAMENIGHT_TOKEN']
    async with websockets.connect('ws://'+address) as ws:
        await ws.send(json.dumps({'type':'hello','role':'lobby','game':game,'token':token}))
        welcome=json.loads(await ws.recv())
        if welcome.get('type')!='welcome': raise RuntimeError('Fixture lobby authentication rejected')
        # Protocol-only fixture is initialized. No visible frame is claimed.
        await ws.send(json.dumps({'type':'lobby_ready'}))
        (out/'lobby-ready.json').write_text(json.dumps({'game':game,'role':'lobby','protocol_only':True}),encoding='utf-8')
        with (out/'runtime-controller-frames.jsonl').open('w',encoding='utf-8') as frames:
            async for raw in ws:
                message=json.loads(raw)
                if message.get('type')=='controller_frame':
                    frames.write(json.dumps(message)+'\n'); frames.flush()
                elif message.get('type')=='error':
                    (out/'lobby-error.json').write_text(json.dumps(message),encoding='utf-8')
                    raise RuntimeError(message.get('message','Fixture lobby error'))


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--out',required=True,type=Path)
    asyncio.run(run(p.parse_args().out))
