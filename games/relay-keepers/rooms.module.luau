-- Distances and foot heights in tiles. Platforms are {left,right,top,material}.
-- p and f are past/future spans, glass is a resetting breakaway span.
-- Goals can have two separate endpoints to require both players at height.
local rooms = {
 {name="Shoulders",start={{2,0},{3.5,0}},goal={18,0},
  land={{0,20,0},{5,9,2.65}},key={7.3,2.65},gate=13},
 {name="Stepwell",start={{1,0},{2.5,0}},goal={18,0},
  land={{0,20,0},{4,8,1.25},{7,11,3.9}},key={9,3.9},gate=14},
 {name="Twin Peaks",start={{1,0},{2.5,0}},goal={16,2.65},secondGoal={12,1.15},
  land={{0,14,0},{4,6,1.25},{8,10,1.25},{11,13,1.15},{15,18,2.65}}},
 {name="Counterweight",start={{2,0},{3.5,0}},goal={18,0},
  land={{0,20,0},{3,7,1.1},{14,17,1}},pad={5,1.1},gate=10,latch={15,1}},
 {name="High Contact",start={{1,0},{2.5,0}},goal={18,0},
  land={{0,20,0},{4,8,2.65},{14,17,1.3}},pad={6,2.65},gate=11,latch={15,1.3}},
 {name="Freight",start={{1,0},{2.5,0}},goal={18,0},
  land={{0,20,0},{10,14,2.65}},crate={4,0},pad={7,0},gate=9,key={12,2.65},lock=16},
 {name="Overtime",start={{1,0},{2.5,0}},goal={18,0},
  land={{0,6,0},{7,10,0},{11,20,0},{2,5,2.65}},
  timer=2,pad={3.5,2.65},gate=15,latch={17,0}},
 {name="Hourglass",start={{1,0},{2.5,0}},goal={18,0},
  land={{0,6,0},{6,11,0,"f"},{11,20,0}},clock={4,0},latch={14,0}},
 {name="Between Tides",start={{1,0},{2.5,0}},goal={18,0},
  land={{0,5,0},{5,9.5,0,"f"},{9.5,11,0},{11,15.5,0,"p"},{15.5,20,0}},
  clock={3.7,0},latch={17,0}},
 {name="Afterimage",start={{1,0},{2.5,0}},goal={18,0},
  land={{0,6,0},{3,6,2.65},{6,10.5,0,"f"},{10.5,12,0.8},{12,16.5,1,"p"},{16.5,20,0}},
  clock={4.4,2.65},latch={17.5,0}},
 {name="First Flight",start={{1,0},{2.5,0}},goal={18,0},
  land={{0,20,0},{4,8,2.65},{10,13,4.7}},wings={6,2.65},key={12,4.7},gate=15},
 {name="Updraft",start={{1,0},{2.5,0}},goal={18,0},
  land={{0,7.5,0},{9.7,14,2.65},{15,20,0}},spring={6,0},pad={12,2.65},gate=15.5,latch={17,0}},
 {name="Fault Line",start={{1,0},{2.5,0}},goal={18.5,0},
  land={{0,6,0},{6,9,0,"glass"},{10,13,0},{13,15,0,"glass"},{16,20,0},{2,5,2.65}},
  timer=1.75,pad={3.5,2.65},gate=16.7,latch={17.7,0}},
 {name="Night Shift",start={{1,0},{2.5,0}},goal={19,0},
  land={{0,6,0},{3,6,2.65},{6,10,0,"f"},{10,12,1},{12,16,1,"p"},
        {16,20,0},{15,17,2.5},{17,20,4.7}},clock={0.35,0},wings={4.5,2.65},
  latch={16.6,0},key={18,4.7},gate=17.4,gateHeight=2.6},
}

local districts={
    require("drydock"), require("switchyard"), require("aviary"),
    require("almanac"), require("foundry"), require("summit"),
    require("narrows"), require("windinghouse"), require("springline"), require("skyport"), require("waystations"),
    require("saltmarsh"), require("glasshouse"), require("starworks"), require("aurora"),
    require("counterpoise"), require("lanternmile"), require("emberkilns"), require("beaconhead"),
    require("tidepools"), require("crabwalk"), require("foldworks"),
    require("echoes"),
}
-- The campaign climbs fast: a short opening, then only the hardest rooms of
-- each district. Rooms built on the old past/future clock switch are retired;
-- the echo hourglass (district 23, played right after the aviary) is the
-- time mechanic now. `index` is a room's place in the full authored list,
-- which the solution routes in scripts/campaign.py follow.
local OPENING={Shoulders=true,["High Contact"]=true,Freight=true,["First Flight"]=true,["Fault Line"]=true}
local ORDER={1,2,3,23,5,6,7,8,9,10,11,12,13,14,16,17,18,19,20,21,22}
local KEEP={[23]=99}
local campaign={}
for index,room in ipairs(rooms) do
    room.index=index
    if OPENING[room.name] then table.insert(campaign,room) end
end
local index=#rooms
local kept={}
for district,chapter in ipairs(districts) do
    kept[district]={}
    for _,room in ipairs(chapter) do
        index+=1
        room.index,room.district=index,district
        if not room.clock then table.insert(kept[district],room) end
    end
end
for _,district in ORDER do
    local list=kept[district]
    for n=math.max(1,#list-(KEEP[district] or 4)+1),#list do table.insert(campaign,list[n]) end
end
return campaign
