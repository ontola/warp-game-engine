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
}
for district,chapter in districts do
    for _,room in chapter do
        room.district=district
        table.insert(rooms,room)
    end
end
return rooms
