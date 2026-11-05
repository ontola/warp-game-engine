"""Isolated installed-daemon smoke test. Synthetic profiles; never joins a live party.
Usage: python test-gamenight.py --daemon PATH --game PATH --out NEW_DIRECTORY
Creates only owned headless processes, uses the daemon's real launch/token path.
Physical controller sampling, rumble and desktop window focus remain manual tests.
"""
import argparse, json, os, socket, subprocess, time
from pathlib import Path

parser=argparse.ArgumentParser()
parser.add_argument("--daemon", required=True)
parser.add_argument("--game", required=True)
parser.add_argument("--out", required=True)
args=parser.parse_args()
out=Path(args.out).resolve(); out.mkdir(parents=True, exist_ok=False)
game=Path(args.game).resolve(); daemon=Path(args.daemon).resolve()
with socket.socket() as s:
    s.bind(("127.0.0.1",0)); port=s.getsockname()[1]
lib=[{"id":"spaceracer","title":"SpaceRacer integration test","min_players":1,"max_players":4,
      "launch":{"command":str(game),"cwd":str(game.parent),"args":["--integration-probe",str(out),"--biome","forest"]}}]
(out/"library.json").write_text(json.dumps(lib))
env={k:v for k,v in os.environ.items() if not k.startswith("GAMENIGHT_")}
env.update(GAMENIGHT_ADDR=f"127.0.0.1:{port}",GAMENIGHT_LIBRARY=str(out/"library.json"),GAMENIGHT_NO_PREWARM="1",GAMENIGHT_NO_LOBBY_WATCH="1",GAMENIGHT_STORE=str(out/"store"),GAMENIGHT_PLAYER_MEMORY=str(out/"memory.json"),GAMENIGHT_TEST_NO_CONTROLLERS="1",GAMENIGHT_NO_MUSIC="1")
log=(out/"daemon.log").open("w")
proc=subprocess.Popen([str(daemon)],env=env,stdout=log,stderr=log,creationflags=getattr(subprocess,"CREATE_NO_WINDOW",0))
checks=[]
def wait(fn,label,seconds=75):
    until=time.monotonic()+seconds
    while time.monotonic()<until:
        try:
            val=fn()
            if val: return val
        except (OSError,ValueError,KeyError): pass
        if proc.poll() is not None: raise RuntimeError("Owned daemon exited")
        time.sleep(.1)
    raise TimeoutError(label)
def request(msg=None):
    with socket.create_connection(("127.0.0.1",port),3) as s:
        s.settimeout(5)
        stream=s.makefile("rb")
        s.sendall(b'{"type":"hello","role":"overlay"}\n')
        welcome=json.loads(stream.readline())
        if msg is None: return welcome["party"]
        s.sendall((json.dumps(msg)+"\n").encode())
        response=json.loads(stream.readline())
        if response.get("type")=="error": raise RuntimeError(response.get("message"))
        return response

def state(): return json.loads((out/"state.json").read_text())
def command(text,capture=None):
    value={"command":text}
    if capture: value["capture"]=capture
    tmp=out/"request.tmp"; tmp.write_text(json.dumps(value)); tmp.replace(out/"request.json")
    wait(lambda: not (out/"request.json").exists(),text)
    if capture: wait(lambda: (out/capture).is_file(),capture)
def check(label): checks.append(label); print("PASS",label,flush=True)
def setting(key,value):
    request({"type":"set_setting","game":"spaceracer","key":key,"value":value})
    wait(lambda: state()["settings"][key]==value,key)
def avatar(hat):
    px=[""]*(48*48)
    for y in range(48):
        for x in range(48):
            if (y in range(24,28) and (x in range(19,22) or x in range(29,32))) or (y in range(33,35) and x in range(23,31)): px[y*48+x]="#152337"
            if hat and ((y in range(5,16) and abs(x-24)<(y-3)/2) or (y in range(15,18) and x in range(13,36))): px[y*48+x]="#db44ef"
    return json.dumps({"v":1,"w":48,"h":48,"px":px},separators=(",",":"))
try:
    wait(lambda:request(),"daemon listening")
    for name,color,hat in [("Mint Pilot","#37dab6",True),("Coral Pilot","#ee655b",False)]:
        request({"type":"join_party","name":name,"color":color,"avatar":avatar(hat)})
    players=wait(lambda: request().get("players"),"players")
    ids=[next(p["id"] for p in players if p["name"]==name) for name in ["Mint Pilot","Coral Pilot"]]
    for i,p in enumerate(ids):
        request({"type":"bind_controller","player_id":p,"controller":f"test-opaque-{8-i*5}"})
        request({"type":"set_player_skin_color","player_id":p,"skin_color":["#d9a675","#fee0bc"][i]})
    request({"type":"play_next","game":"spaceracer"})
    wait(lambda: state().get("host_phase")=="running","game launched and ready",120)
    assert state()["players"]==2 and state()["pilot_names"][:2]==["Mint Pilot","Coral Pilot"],state()
    assert not state()["error"] and not state()["script_error"]
    check("daemon launches portable build; ready after GPU upload; two named views")
    party=request()
    (out/"host-summary.json").write_text(json.dumps({k:party.get(k) for k in ["settings","actions","active_session"]},indent=2))
    for key,value in [("boost_cost",31),("boost_duration",150),("energy_refill",200),("respawn_seconds",4),("quality","balanced"),("laps",2),("difficulty","hard"),("biome","forest"),("seed",80385),("weapon_pickups",False)]: setting(key,value)
    assert state()["active_laps"]==3 and state()["pickup_count"]>0
    assert state()["pilots"][0]["boost_cost"]==31
    assert state()["pilots"][0]["boost_duration"]==1.875
    assert state()["live_settings"]=={"refill":2,"recovery":4}
    check("all ten settings accepted; live boost applies; structural changes deferred")
    try: setting("laps",0)
    except RuntimeError: check("invalid settings rejected by real host")
    else: raise AssertionError("Invalid laps accepted")
    for action,values,cost in [("set",{"boost_cost":17,"energy_refill":50},17),("undo",{},31)]:
        party=request(); spec=next(v for v in party["settings"] if v["game"]=="spaceracer")
        request({"type":"control_settings","game":"spaceracer","session":party["active_session"]["id"],"expected_revision":spec["revision"],"player_id":ids[0],"action":action,"values":values})
        wait(lambda:state()["pilots"][0]["boost_cost"]==cost,"batch "+action)
    check("atomic settings batch and Undo restore actual pilot parameters")
    request({"type":"open_overlay"})
    wait(lambda:state()["host_phase"]=="paused","overlay pause")
    time.sleep(.2); frozen=state()["clock"]; rev=state()["face_revisions"][0]
    command("portrait on","profiles-before.png")
    request({"type":"set_player_avatar","player_id":ids[0],"avatar":avatar(False)})
    request({"type":"set_player_color","player_id":ids[0],"color":"#6688ee"})
    request({"type":"rename_player","player_id":ids[0],"name":"Blue Pilot"})
    wait(lambda:state()["face_revisions"][0]>rev and state()["pilot_names"][0]=="Blue Pilot","live profile")
    command("portrait on","profiles-after.png")
    assert state()["clock"]==frozen
    check("overlay freezes simulation; live avatar, paint and name update without restart")
    command("portrait off","two-player.png")
    request({"type":"close_overlay"})
    wait(lambda:state()["clock"]>frozen+.1,"overlay resume")
    request({"type":"pause"}); wait(lambda:state()["host_phase"]=="paused","explicit pause")
    request({"type":"open_overlay"}); request({"type":"close_overlay"})
    assert state()["host_phase"]=="paused"
    request({"type":"resume"}); wait(lambda:state()["host_phase"]=="running","explicit resume")
    check("overlay close resumes; explicit pause remains paused until resume")
    seats=state()["host_seats"][:2]
    request({"type":"open_overlay"})
    request({"type":"swap_seats","a":seats[0],"b":seats[1]})
    # Fixed race rosters: closing the overlay after a swap asks the host to
    # Prepare a fresh session. It must not silently transfer an ongoing car.
    request({"type":"close_overlay"})
    wait(lambda:state()["pilot_names"][:2]==["Coral Pilot","Blue Pilot"] and state()["host_phase"]=="running","new roster prepare")
    assert state()["bots"][:2]==[False,False]
    assert state()["seed"]==80385 and state()["active_laps"]==2 and state()["pickup_count"]==0
    check("host re-prepares swapped seats on overlay close with saved track settings")
    seed=state()["seed"]
    request({"type":"trigger_action","game":"spaceracer","key":"new_race","player_id":ids[0]})
    wait(lambda:state()["seed"]==seed and state()["active_laps"]==2 and state()["pickup_count"]==0,"next race settings")
    assert state()["biome"]=="forest" and state()["difficulty"]=="hard"
    check("real host new-race action keeps a fixed seed and applies deferred settings")
    request({"type":"open_overlay"})
    for name in ["Gold Pilot","Violet Pilot"]: request({"type":"join_party","name":name})
    request({"type":"close_overlay"})
    wait(lambda:state()["players"]==4 and state()["host_phase"]=="running","four players")
    request({"type":"open_overlay"}); wait(lambda:state()["host_phase"]=="paused","four-player pause")
    command("portrait off","four-player.png")
    assert state()["pilot_names"][:4]==["Coral Pilot","Blue Pilot","Gold Pilot","Violet Pilot"]
    check("joining through the overlay re-prepares four named viewports")
    report={"checks":checks,"daemon":str(daemon),"game":str(game),"physical_controllers_tested":False,"desktop_focus_tested":False}
    (out/"report.json").write_text(json.dumps(report,indent=2))
finally:
    (out/"stop").touch()
    time.sleep(2)
    proc.terminate()
    try: proc.wait(timeout=10)
    except subprocess.TimeoutExpired: proc.kill();proc.wait()
    log.close()
