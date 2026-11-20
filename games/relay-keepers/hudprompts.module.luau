-- Pocket-chart illustrations, printed on a two-pixel grid in room coordinates.
local M={}
local ink,paper,tide="#37565a","#f4e6c5","#84aaa5"

-- Coastline, a small compass rose, and a sounding route across the folded sheet.
local coastline={
    "........................", "........................",
    "...c..............:::~~~", "...c............::~~~~~~",
    ".ccccc...........:~~o~~~", "...c.............:~~~~~~",
    "...c..........::::~~~~~~", "...........:::~~~~o~~~~~",
    "..........:~~~~~~~~~~~:.", "....::::..:~~~~~o~~~~:..",
    "...:~~~~::~~~~~~~~~~:...", "...:~~~~~~~~o~~~~~::....",
    ".::~~~~~o~~~~~~~::......", ".:~~~~~~~~~~~:::........",
    "........................", "........................",
}
local folds={3,3,2,2,1,1,0,0, 0,0,1,1,2,2,3,3, 3,3,2,2,1,1,0,0}
local paperInks={
    {["."]="#fff0ca",["~"]="#91c0b7",[":"]="#6c9d93",o="#976544",c="#73948c"},
    {["."]="#d7c9a5",["~"]="#70a298",[":"]="#578279",o="#825438",c="#668c82"},
    {["."]="#f1e4be",["~"]="#88b8ad",[":"]="#68998f",o="#976544",c="#73948c"},
}

-- Collapse neighboring pixels into an ink run. All drawing below uses flat ink.
local function printRuns(rows)
    local runs={}
    for y,row in rows do
        local x=1
        while x<=#row do
            local c=row[x]
            local right=x+1
            while right<=#row and row[right]==c do right+=1 end
            if c then table.insert(runs,{x-1,y-1,right-x,c}) end
            x=right
        end
    end
    return runs
end

local function printChart()
    local rows={}
    -- False is unprinted paper outside the silhouette, and keeps each row dense.
    for y=1,21 do rows[y]=table.create(26,false) end
    for x,offset in folds do
        for y=0,17 do
            rows[y+offset+1][x]=ink
            rows[y+offset+1][x+2]=ink
        end
    end
    for x,offset in folds do
        local palette=paperInks[math.ceil(x/8)]
        for y=1,#coastline do
            rows[y+offset+1][x+1]=palette[coastline[y]:sub(x,x)]
        end
    end
    return printRuns(rows)
end
local chart=printChart()

local function foldedChart(box,x,y,open)
    if open<=0 then return end
    local width=2*(0.32+open*0.68)
    local right=x+52
    y-=math.floor(63*(1-open))
    for _,r in chart do
        local left=math.floor(right+(r[1]-26)*width)
        local edge=math.floor(right+(r[1]+r[3]-26)*width)
        if edge>left then box(left,y+r[2]*2,edge-left,2,r[4]) end
    end
end

-- An outline controller with a D-pad and four face buttons, engraved in two inks.
-- The active button has a paper collar so it also reads on a dark district sky.
local function gesture(box,x,y,face,reveal)
    if reveal<=0 then return end
    local alpha=string.format("%02x",math.floor(reveal*255+0.5))
    local outline,fill=ink..alpha,paper..alpha
    local function r(dx,dy,w,h,c) box(x+dx,y+dy,w,h,c) end
    r(6,2,31,16,outline)
    r(3,5,37,13,outline)
    r(1,10,12,12,outline)
    r(30,10,12,12,outline)
    r(1,20,8,4,outline)
    r(34,20,8,4,outline)
    r(6,5,31,10,fill)
    r(4,9,9,9,fill)
    r(31,9,8,9,fill)
    r(3,16,6,5,fill)
    r(35,16,5,5,fill)
    r(11,6,3,9,outline)
    r(8,9,9,3,outline)
    for i,p in {{31,6},{35,10},{31,14},{27,10}} do
        if i==face then
            r(p[1]-3,p[2]-3,6,6,outline)
            r(p[1]-2,p[2]-2,4,4,(if i==1 then "#edb85f" elseif i==3 then "#76bfa6" else "#79b3c8")..alpha)
        else
            r(p[1]-1,p[2]-1,3,3,outline)
        end
    end
end

local function note(box,x,y)
    box(x+2,y,30,20,ink)
    box(x,y+2,34,16,ink)
    box(x+5,y+18,7,6,ink)
    box(x+3,y+3,28,13,paper)
    box(x+7,y+15,4,5,paper)
    box(x+10,y+8,3,3,tide)
    box(x+16,y+8,3,3,tide)
    box(x+22,y+8,3,3,tide)
end

function M.corner(box,s)
    local reviewing=s.review~=nil
    local rest=s.promptRest or 0
    local reveal=math.clamp((rest-1)/0.4,0,1)
    if s.atlasSeen and not reviewing then reveal=0 end
    if reviewing and s.review.held then reveal=1 end
    local button=if reviewing then 4 else 1
    local shift=math.floor(8*(1-reveal)+0.5)
    gesture(box,1074+shift,31,button,reveal)
    if reviewing then
        note(box,1137,28)
    else
        local k=math.clamp((rest-0.15)/0.55,0,1)
        foldedChart(box,1129,18,k*k*(3-2*k))
    end
end

function M.atlas(box)
    gesture(box,30,595,3,1)
    for i=0,4 do box(85+i*3,598+i*2,3,20-i*4,paper) end
    gesture(box,1074,595,1,1)
    foldedChart(box,1129,586,1)
end
return M
