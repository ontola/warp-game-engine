-- Screen-painted coastlines. Positions, silhouettes and palettes are authored;
-- only the tide, signal lights, birds and weather move.
local M = {}
local cache = {}
local floor,ceil,sin,cos=math.floor,math.ceil,math.sin,math.cos

-- Sky, horizon, sun, distant mountain, middle ridge, architecture, lit face,
-- water, and reflection. Colours stay softer than the traversable deck.
local palettes={
    harbor={"#71b7c4","#f3d8ad","#fff0ba","#bfd0ba","#9ab8a7","#739b94","#b0c1a4","#78aaa7","#a2c6b5"},
    marsh={"#91c5b1","#ead6a0","#fff0b0","#ccd3a2","#b2be84","#869971","#ced095","#8eb2a1","#c3cda4"},
    glass={"#7cbca5","#d8e2af","#fff2c6","#b3d2ae","#91bba2","#729288","#afd4b1","#82afa1","#acd5bb"},
    stars={"#434c78","#ecb390","#ffd7a0","#ba9ca9","#988fa9","#77778f","#b4a6ab","#888ca6","#b6a9ac"},
    ice={"#213855","#789ba0","#f2dfb4","#769398","#587b87","#4a6877","#95b9b5","#517e8a","#84aaaa"},
    trestle={"#887ca1","#ebc3ad","#ffe3aa","#b9a8ad","#a098a2","#766f83","#c1a7a0","#8c93a2","#b6adb4"},
    lantern={"#564570","#ec9c87","#ffe3ab","#b38ca0","#977f94","#736077","#d9a081","#947b8e","#c29b9f"},
    water={"#6bb7c7","#d1e8ca","#fff5c6","#accec1","#8eb9ac","#659b98","#b5d1b8","#73adae","#a5d6c9"},
    crab={"#db8d77","#f9d297","#fff4bc","#d3b495","#bf9c82","#997f70","#d5ae83","#a1a998","#d1c6a4"},
    portal={"#3a446b","#b0a6bb","#ffdeba","#929bb0","#7e88a3","#5c6586","#ada5bd","#798ba2","#afafc4"},
    kiln={"#ad6e77","#ffcc96","#ffe5a0","#d4a088","#b98978","#926a67","#d29b7d","#ba8c83","#dcac92"},
    beacon={"#758dba","#ffcda5","#ffefc5","#b5b6b6","#939fa7","#6e8390","#afbab2","#789baa","#adc4bf"},
    midnight={"#243652","#6b8090","#eae4c3","#667795","#526882","#43596c","#87999b","#4a7082","#7895a1"},
}
local districtStyle={
    [0]={"harbor","beacon"}, {"harbor","yard"}, {"kiln","mill"},
    {"beacon","sails"}, {"stars","observatory"}, {"kiln","kilns"},
    {"ice","ice"}, {"harbor","sails"}, {"kiln","yard"},
    {"glass","glass"}, {"stars","observatory"}, {"beacon","beacon"},
    {"marsh","mill"}, {"glass","glass"}, {"stars","observatory"},
    {"ice","ice"}, {"trestle","trestle"}, {"lantern","lantern"},
    {"kiln","kilns"}, {"beacon","beacon"},
    {"water","water"}, {"crab","crab"}, {"portal","portal"},
}

local function blend(a,b,k)
    local function channel(i)
        local v=tonumber(a:sub(i,i+1),16)
        return floor(v+(tonumber(b:sub(i,i+1),16)-v)*k+0.5)
    end
    return string.format("#%02x%02x%02x",channel(2),channel(4),channel(6))
end

-- Scan-convert angled shapes onto a four-pixel grid, coalescing equal runs.
local function polygon(rect,points,color,pixel)
    pixel=pixel or 4
    local low,high=640,0
    for i=2,#points,2 do low=math.min(low,points[i]) high=math.max(high,points[i]) end
    local lastX,lastW,lastY,lastH=nil,0,0,0
    for y=floor(low/pixel)*pixel,ceil(high/pixel)*pixel-pixel,pixel do
        local hits={}
        local scan=y+pixel/2
        local j=#points-1
        for i=1,#points-1,2 do
            local ax,ay,bx,by=points[j],points[j+1],points[i],points[i+1]
            if (ay<=scan and by>scan) or (by<=scan and ay>scan) then
                table.insert(hits,ax+(scan-ay)*(bx-ax)/(by-ay))
            end
            j=i
        end
        table.sort(hits)
        for i=1,#hits-1,2 do
            local x=floor(hits[i]/pixel)*pixel
            local w=ceil(hits[i+1]/pixel)*pixel-x
            if #hits==2 and lastX==x and lastW==w and lastY+lastH==y then
                lastH+=pixel
            else
                if lastX then rect(lastX,lastY,lastW,lastH,color) end
                lastX,lastW,lastY,lastH=x,w,y,pixel
            end
        end
    end
    if lastX then rect(lastX,lastY,lastW,lastH,color) end
end

local function oval(rect,x,y,rx,ry,color,step)
    for dy=-ry,ry-1,step do
        local u=(dy+step/2)/ry
        local half=floor(rx*math.sqrt(math.max(0,1-u*u))/step)*step
        if half>0 then rect(x-half,y+dy,2*half,step,color) end
    end
end

local function hut(rect,x,y,w,h,c,lit)
    rect(x,y-h,w,h,c)
    polygon(rect,{x-5,y-h,x+w*0.48,y-h-17,x+w+5,y-h},c)
    rect(x+5,y-h+4,w*0.3,h-4,lit)
    rect(x+w*0.56,y-h+7,5,8,lit)
end

local function pine(rect,x,y,h,c)
    rect(x-2,y-h*0.42,4,h*0.42,c)
    for i=1,3 do
        local w=h*(0.09+0.035*i)
        local base=y-h*(0.67-(i-1)*0.22)
        polygon(rect,{x,base-h*0.35,x+w,base,x-w,base},c)
    end
end

local function printLandmark(kind,rect,p)
    local mass,light=p[6],p[7]
    if kind=="beacon" then
        polygon(rect,{712,431,774,403,812,401,842,372,886,365,921,324,992,320,1018,352,1058,363,1132,401,1200,415,1200,458,712,458},mass)
        polygon(rect,{875,384,923,328,987,324,957,350,940,352,917,380},light)
        polygon(rect,{926,322,936,251,953,251,964,324},light)
        polygon(rect,{950,253,956,253,965,324,956,324},mass)
        polygon(rect,{931,284,958,284,961,300,930,300},mass)
        rect(931,245,29,7,mass) rect(935,227,22,17,mass)
        rect(938,231,7,10,p[3]) rect(948,231,5,10,p[3])
        polygon(rect,{930,227,946,214,962,227},mass)
        hut(rect,976,328,40,14,mass,light)
        hut(rect,796,406,48,15,mass,light)
    elseif kind=="mill" then
        polygon(rect,{0,416,92,401,162,410,215,398,322,403,419,424,538,434,630,451,0,451},mass)
        polygon(rect,{153,403,169,307,198,307,215,405},light)
        polygon(rect,{184,308,198,308,215,405,193,405},mass)
        polygon(rect,{160,310,170,293,189,289,205,310},mass)
        rect(174,366,11,31,mass)
        hut(rect,235,405,51,18,mass,light)
        for _,x in {602,635,664,1004,1032,1065} do
            rect(x,406,3,27,mass)
            polygon(rect,{x+1,426,x-12,393,x+3,417},mass)
            rect(x+2,397,4,13,light)
        end
    elseif kind=="glass" then
        for _,h in {{80,427,150,48},{230,427,199,78},{429,427,109,36}} do
            local x,y,w,t=unpack(h)
            polygon(rect,{x,y-8,x+8,y-t*0.62,x+w*0.5,y-t,x+w-8,y-t*0.62,x+w,y-8},light)
            for i=0,4 do
                local rx=x+w*i/4
                local roof=y-t*(1-0.8*math.abs(i-2)/2)
                rect(rx,roof,4,y-roof,mass)
            end
            rect(x,y-t*0.25,w,4,mass)
            polygon(rect,{x,y-t*0.42,x+w*0.5,y-t-4,x+w,y-t*0.42,x+w,y-t*0.42+5,x+w*0.5,y-t+1,x,y-t*0.42+5},mass)
            rect(x,y-8,w,12,mass)
        end
        for _,v in {{768,408,62},{794,410,44},{1048,420,80}} do
            local x,y,h=unpack(v)
            rect(x-3,y-h,6,h,mass)
            for _,dx in {-31,-18,19,34} do
                polygon(rect,{x,y-h+4,x+dx*0.55,y-h-12,x+dx,y-h+10,x+dx*0.48,y-h-3},mass)
            end
        end
    elseif kind=="observatory" then
        polygon(rect,{0,432,78,404,167,398,222,372,294,373,351,392,460,410,517,436,0,444},mass)
        rect(171,347,97,41,mass)
        oval(rect,220,346,50,46,light,4)
        rect(168,346,104,12,mass)
        rect(216,300,8,46,mass)
        rect(185,364,6,9,light) rect(203,364,6,9,light)
        polygon(rect,{813,420,826,366,837,366,850,420},mass)
        polygon(rect,{781,377,777,367,871,314,878,326},mass)
        polygon(rect,{860,319,856,308,874,298,885,317,869,329},light)
        hut(rect,878,424,77,21,mass,light)
    elseif kind=="kilns" then
        for _,v in {{135,416,61},{234,429,89},{334,422,48}} do
            local x,y,h=unpack(v)
            polygon(rect,{x-33,y,x-27,y-34,x-12,y-52,x-8,y-h,x+8,y-h,x+12,y-52,x+28,y-34,x+35,y},mass)
            rect(x-10,y-h-3,21,5,light)
            polygon(rect,{x-13,y,x-11,y-16,x-6,y-25,x+6,y-25,x+12,y-16,x+13,y},p[3])
            rect(x-4,y-18,8,18,light)
        end
        polygon(rect,{577,439,685,423,772,391,832,393,886,373,1005,394,1100,422,1200,420,1200,453,577,453},mass)
    elseif kind=="water" then
        for _,v in {{-33,438,194,104},{218,430,105,57},{852,434,180,82},{1061,431,118,118}} do
            local x,y,w,h=unpack(v)
            polygon(rect,{x,y,x+w*.12,y-h*.36,x+w*.21,y-h*.38,x+w*.27,y-h*.86,x+w*.40,y-h,x+w*.72,y-h,x+w*.83,y-h*.61,x+w*.91,y-h*.23,x+w,y},mass)
            polygon(rect,{x+w*.21,y-h*.38,x+w*.27,y-h*.84,x+w*.41,y-h,x+w*.68,y-h,x+w*.54,y-h*.80,x+w*.51,y-h*.68,x+w*.37,y-h*.63,x+w*.34,y-h*.42},light)
            rect(x+w*.19,y-9,w*.65,4,p[9])
        end
        for _,v in {{0,442,108},{298,432,55},{888,443,87},{1080,437,102}} do rect(v[1],v[2],v[3],3,p[9]) end
    elseif kind=="crab" then
        for _,v in {{93,424,92},{320,432,61},{870,421,125}} do
            local x,y,w=unpack(v)
            polygon(rect,{x-w*.63,y-12,x+w*.51,y-12,x+w*.42,y-3,x+w*.27,y+4,x-w*.40,y+4},mass)
            rect(x-w*.21,y-35,w*.39,21,light)
            rect(x-w*.25,y-36,w*.45,5,mass)
            rect(x-w*.07,y-29,w*.1,8,mass)
            rect(x+w*.25,y-74,4,59,mass)
            for n=0,8 do rect(x+w*.24-n*5,y-74+n*5,6,3,mass) end
            rect(x+w*.25,y-74,w*.17,5,light)
        end
        for _,x in {550,585,616,1045,1082,1129} do
            rect(x,408,3,22,mass) rect(x-3,414,9,8,p[3]) rect(x-5,423,12,4,mass)
        end
    elseif kind=="portal" then
        for _,v in {{90,420,66},{220,425,100},{810,423,116},{1057,424,77}} do
            local x,y,h=unpack(v)
            polygon(rect,{x-22,y,x-12,y-h*.85,x-5,y-h,x+12,y-h*.93,x+20,y},mass)
            polygon(rect,{x-12,y-h*.8,x-4,y-h,x+1,y-h*.85,x-4,y-7,x-9,y-4},light)
            rect(x-16,y-h*.61,29,5,p[5])
            local fy=y-h-30
            polygon(rect,{x-11,fy,x-3,fy-16,x+8,fy-14,x+10,fy-1,x+1,fy+6},light)
        end
        for n=0,19 do
            local a=n/19*math.pi
            rect(655+math.cos(a)*116,213-math.sin(a)*64,11,4,p[7])
        end
    elseif kind=="trestle" then
        for _,x in {128,368,608,848,1088} do
            polygon(rect,{x-18,427,x-9,294,x-5,289,x+4,289,x+9,294,x+18,427},mass)
            rect(x-6,308,6,81,light)
            for j=0,11 do
                local q=j/12
                local dy=77*4*q*(1-q)
                rect(x+j*20,299+dy,20,4,mass)
                if j%2==1 then rect(x+j*20+9,301+dy,3,math.max(2,83-dy),mass) end
            end
        end
        rect(0,386,1200,7,mass)
        rect(0,386,1200,2,light)
    elseif kind=="lantern" then
        polygon(rect,{0,438,54,422,132,425,228,412,336,422,476,406,598,421,688,434,748,447,0,447},mass)
        for _,v in {{30,428,52},{157,421,59},{246,424,37},{376,421,58},{510,420,48},{990,424,55},{1072,421,44}} do
            local x,y,w=unpack(v)
            rect(x,y-35,w,34,mass)
            polygon(rect,{x-10,y-32,x+6,y-42,x+w*0.45,y-54,x+w,y-41,x+w+12,y-32},mass)
            rect(x+7,y-22,w-14,4,light)
            for n=12,w-8,14 do rect(x+n,y-21,3,9,p[3]) end
        end
        for _,v in {{50,195,2},{324,226,3},{594,185,2},{864,221,3}} do
            local x,y,phase=unpack(v)
            rect(x,y-3,4,229-(y-195),mass)
            for j=0,13 do
                local q=j/14
                local dest=if phase==2 then 31 else -41
                local ly=y+45*4*q*(1-q)+dest*q
                rect(x+j*19.4,ly,20,3,light)
                if j%3==2 then
                    local h=if j%2==0 then 11 else 17
                    rect(x+j*19.4+8,ly+2,2,7,mass)
                    rect(x+j*19.4+4,ly+9,10,h,p[3])
                    rect(x+j*19.4+1,ly+12,16,h-6,"#f6bb8d")
                    rect(x+j*19.4+5,ly+h+8,8,3,mass)
                end
            end
        end
    elseif kind=="ice" then
        for _,v in {{34,421,65},{60,428,99},{108,421,50},{771,425,59},{823,421,90},{865,431,67},{1130,426,81},{1170,422,52}} do
            pine(rect,v[1],v[2],v[3],mass)
        end
    elseif kind=="sails" then
        for _,v in {{248,428,80},{873,428,51},{948,426,76}} do
            local x,y,h=unpack(v)
            rect(x,y-h,3,h,mass)
            polygon(rect,{x-4,y-h+8,x-4,y-13,x-h*0.58,y-12},light)
            polygon(rect,{x+5,y-h+20,x+h*0.36,y-17,x+5,y-16},mass)
            polygon(rect,{x-h*0.62,y-6,x+h*0.48,y-6,x+h*0.34,y+2,x-h*0.46,y+2},mass)
        end
    else
        for _,v in {{162,425,92},{308,425,141},{875,426,87}} do
            local x,y,h=unpack(v)
            rect(x-4,y-h,10,h,mass)
            polygon(rect,{x-25,y-h+8,x+70,y-h-28,x+75,y-h-18,x-22,y-h+15},mass)
            rect(x+67,y-h-17,3,49,mass) rect(x+60,y-h+28,17,8,light)
            hut(rect,x-35,y,67,22,mass,light)
        end
    end
end

local function plate(p,kind,moon)
    local shapes={}
    local function rect(x,y,w,h,c)
        table.insert(shapes,{x,y,w,h,c})
    end
    rect(0,0,1200,640,p[1])
    -- Wide printed bands; a continuous horizon behind the moving machinery.
    for i=0,15 do rect(0,40+i*24,1200,24,blend(p[1],p[2],i/15)) end
    local sx=if moon then 178 elseif kind=="beacon" then 648 elseif kind=="observatory" then 832 else 940
    local sy=if kind=="beacon" then 150 else 146
    oval(rect,sx,sy,59,59,p[3],4)
    if moon then oval(rect,sx+21,sy-12,49,48,blend(p[1],p[2],0.22),4) end
    for _,v in {{51,164,109},{350,93,59},{543,175,91},{910,189,120},{997,110,77}} do
        local x,y,w=unpack(v)
        local c=blend(p[2],p[3],0.28)
        rect(x,y,w,4,c) rect(x+w*0.30,y-4,w*0.57,4,c)
    end
    if kind=="ice" or kind=="observatory" or kind=="portal" or moon then
        for _,v in {{74,118},{142,57},{205,91},{270,64},{375,139},{418,88},{686,112},{731,75},{796,131},{1130,77},{1174,131}} do
            rect(v[1],v[2],3,3,p[3])
        end
    end
    if kind=="ice" then
        for i=0,68 do
            local x=i*18
            local y=94+30*sin(i*0.10)+13*sin(i*0.24)
            local h=19+17*(1+sin(i*0.16))
            rect(x,y,18,h,blend(p[1],"#7ccbb5",0.56+0.13*sin(i*0.14)))
            rect(x,y+h-4,18,5,"#8cc5b3")
        end
    end
    if kind=="mill" then
        polygon(rect,{0,348,109,339,228,350,385,319,486,326,579,307,679,314,772,341,902,331,1038,312,1200,322,1200,440,0,440},p[4])
    elseif kind=="glass" or kind=="lantern" then
        polygon(rect,{0,331,83,322,166,291,215,285,276,291,373,328,463,311,529,284,604,282,675,298,782,317,896,302,975,272,1048,266,1120,285,1200,297,1200,440,0,440},p[4])
    elseif kind=="kilns" then
        polygon(rect,{0,310,130,303,169,282,232,282,251,254,321,254,380,292,428,302,523,302,560,317,634,318,728,264,790,258,844,270,904,307,1007,296,1085,254,1153,254,1200,276,1200,440,0,440},p[4])
    else
        polygon(rect,{0,325,61,302,136,309,224,250,301,292,398,249,454,266,550,219,619,272,693,236,763,278,844,257,949,286,1059,236,1114,266,1200,229,1200,440,0,440},p[4])
    end
    if kind=="ice" or kind=="beacon" then
        for _,m in {{224,250,39},{550,219,54},{1059,236,35}} do
            local x,y,w=unpack(m)
            polygon(rect,{x-w,y+w*0.6,x,y,x+w*0.75,y+w*0.55,x+5,y+15,x-3,y+11,x-13,y+22,x-17,y+16},blend(p[4],p[2],0.62))
        end
    end
    polygon(rect,{0,353,76,330,168,345,252,323,340,357,438,336,511,354,582,337,648,351,726,322,784,327,861,294,951,314,1020,300,1093,339,1200,314,1200,449,0,449},p[5])
    rect(0,410,1200,230,p[8])
    rect(0,414,1200,2,p[9])
    for _,v in {{42,425,93},{370,430,115},{585,419,164},{981,430,128},{228,443,54},{693,446,105},{853,436,57}} do
        rect(v[1],v[2],v[3],3,p[9])
    end
    printLandmark(kind,rect,p)
    -- Darkening at the bottom gives falling and grounded players a clear edge.
    local deep=blend(p[8],p[6],0.47)
    polygon(rect,{0,581,172,573,260,578,443,565,527,566,654,575,811,560,912,563,1086,551,1200,556,1200,640,0,640},deep,6)
    local tide=blend(p[8],p[9],0.50)
    for _,v in {{8,593,86},{82,587,33},{341,604,75},{400,599,22},{637,580,35},{1030,583,94},{1073,579,31},{915,624,59}} do
        rect(v[1],v[2],v[3],2,tide)
    end
    return shapes
end

function M.style(l)
    local style=districtStyle[l.district or 0] or districtStyle[0]
    return palettes[style[1]],style[2]
end

function M.draw(rect,l,s)
    local p,kind=M.style(l)
    local moon=l.clock and not s.past
    if moon then p=palettes.midnight end
    local cacheKey=tostring(l.district or 0)..(if moon then "-" else "+")
    if not cache[cacheKey] then cache[cacheKey]=plate(p,kind,moon) end
    for _,v in cache[cacheKey] do rect(v[1],v[2],v[3],v[4],v[5]) end
    local t=s.time or 0
    if kind=="mill" then
        for i=0,3 do
            local a=t*0.35+i*math.pi/2
            local dx,dy=cos(a),sin(a)
            polygon(rect,{184+4*dx,310+4*dy,184+49*dx,310+49*dy,184+49*dx-7*dy,310+49*dy+7*dx,184+16*dx-7*dy,310+16*dy+7*dx},p[6])
        end
        rect(180,306,8,8,p[7])
    elseif kind=="kilns" then
        for _,stack in {{135,355},{234,340},{334,374}} do
            for i=0,3 do
                local k=(t*0.16+i/4)%1
                rect(stack[1]+k*32-6,stack[2]-k*59-7,12+10*k,4+10*k,blend(p[4],p[2],k*0.8))
            end
        end
    elseif kind=="beacon" then
        local sweep=math.cos(t*0.54)
        local sx,sy=946,235
        for i=1,12 do
            rect(sx+sweep*i*10-3,sy-2-i*0.23,6+math.abs(sweep)*7,3+i*0.57,blend(p[2],p[3],i/16))
        end
    end
    for i=0,13 do
        local x=(i*137+t*(4+i%3))%1300-55
        rect(x,449+(i*23)%151,16+(i%4)*12,2,p[9])
    end
    if kind~="ice" and kind~="observatory" and kind~="portal" then
        for i=0,2 do
            local x=(t*7+330+i*39)%1320-30
            local y=175+i%2*11+sin(t*0.9+i)*2
            local rise=if sin(t*2.7+i)>0 then -3 else 2
            rect(x,y,4,2,p[6])
            rect(x-5,y+rise,5,2,p[6])
            rect(x+4,y+rise,5,2,p[6])
        end
    end
end
return M
