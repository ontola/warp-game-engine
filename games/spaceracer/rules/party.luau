-- GameNight declarations and validation are game policy, not engine code.
-- When an edit applies: at the next start of the game (the settings list is read
-- once); each setting's label says when its value applies (live or next race).
local specs={
 {key="laps",label="Laps (next race)",kind="number",default=3,min=1,max=5},
 {key="difficulty",label="Difficulty (next race)",kind="choice",default="normal",options={"easy","normal","hard"}},
 {key="biome",label="World (next race)",kind="choice",default="city",options={"random","city","forest","cell","desert","ocean"}},
 {key="seed",label="Track seed, 0 = random (next race)",kind="number",default=0,min=0,max=99999},
 {key="ai_players",label="AI racers: auto is none with several players (next race)",kind="choice",default="auto",options={"auto","0","1","2","3","4","5"}},
 {key="quality",label="Graphics (live)",kind="choice",default="high",options={"performance","balanced","high"}},
 {key="boost_cost",label="Boost energy cost (live)",kind="number",default=22,min=5,max=50},
 {key="boost_duration",label="Boost duration % (next boost)",kind="number",default=100,min=50,max=200},
 {key="energy_refill",label="Energy regeneration % (live)",kind="number",default=100,min=0,max=300},
 {key="respawn_seconds",label="Crash recovery seconds (live)",kind="number",default=2,min=1,max=5},
 {key="weapon_pickups",label="Weapon pickups (next race)",kind="toggle",default=true},
}
local function validate(c)
 for _,s in ipairs(specs) do if s.key==c.key then
  if s.kind=="number" then return type(c.value)=="number" and c.value%1==0 and c.value>=s.min and c.value<=s.max end
  if s.kind=="toggle" then return type(c.value)=="boolean" end
  for _,v in ipairs(s.options) do if v==c.value then return true end end
 end end
 return false
end
local function live(c)
 return {cost=c.boost_cost,duration=1.25*c.boost_duration/100,refill=c.energy_refill/100,recovery=c.respawn_seconds,
  lod=c.quality=="performance" and 2.5 or (c.quality=="balanced" and 1.5 or 1),
  blur=c.quality~="performance",near_shadows=c.quality~="performance"}
end
return {specs=specs,validate=validate,live=live}
