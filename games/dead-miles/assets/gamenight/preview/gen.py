import json,sys
# gen.py name setup_cmds_json warm_ticks frames step
name=sys.argv[1]; setup=json.loads(sys.argv[2]); warm=int(sys.argv[3]); n=int(sys.argv[4]); step=int(sys.argv[5]) if len(sys.argv)>5 else 2
steps=[{"ticks":30},{"press":["A"]},{"ticks":30},{"cmd":"lua debug.hud(false)","ticks":1},{"cmd":"lua debug.knob('autopilot',1)","ticks":1}]
for c in setup:
    if isinstance(c,int): steps.append({"ticks":c})
    else: steps.append({"cmd":"lua "+c,"ticks":1})
steps.append({"ticks":warm})
for i in range(n):
    steps.append({"ticks":step}); steps.append({"capture":f"{name}/f{i:04d}.png"})
json.dump({"size":[960,540],"steps":steps},open(name+".json","w"))
