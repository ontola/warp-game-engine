-- Edit this map in F2 or in the source tree. Positions are {x,z} in metres.
-- The city begins at x=-178. Negative z is north on the minimap.
return {
    kind="west-cape-layout",
    -- Each west-to-east slice is {x, north bank z, south bank z}.
    coast={
        {-1210,-18,10}, {-1203,-44,38}, {-1190,-67,62}, {-1168,-98,88},
        {-1140,-142,102}, {-1110,-183,130}, {-1074,-202,176}, {-1038,-195,224},
        {-1002,-185,255}, {-966,-202,262}, {-930,-235,253}, {-894,-279,238},
        {-858,-310,230}, {-820,-318,241}, {-782,-297,272}, {-746,-268,306},
        {-710,-250,324}, {-672,-249,327}, {-634,-269,302}, {-598,-300,263},
        {-562,-317,225}, {-526,-310,206}, {-490,-282,213}, {-454,-248,232},
        {-418,-227,251}, {-382,-224,256}, {-346,-233,242}, {-310,-238,218},
        {-276,-224,192}, {-242,-201,179}, {-209,-183,178}, {-178,-178,178},
    },
    asphalt={
        width=9, radius=23, junction={-330,0},
        -- The ring starts and ends at the junction with Exchange Avenue.
        bends={{-490,-72},{-675,-127},{-870,-145},{-1030,-78},{-1100,0},
            {-1010,110},{-826,145},{-649,110},{-475,62}},
    },
    rally={
        width=5.2, radius=19,
        -- Endpoints snap to the ring. An optional third value places a checkpoint
        -- at one of four samples around that bend (1=entry, 4=exit).
        bends={{-565,-94},{-585,-68},{-622,-69,1},{-650,-42,4},{-711,-33},
            {-738,-52,4},{-697,-80},{-676,-104},{-696,-113,1},{-753,-94},
            {-789,-65,2},{-806,-23},{-855,-8,3},{-907,24},{-935,53,1},
            {-918,68},{-868,40,3},{-832,43},{-803,65,4},{-831,82},
            {-879,106,4},{-875,125},{-821,144}},
        -- Offset from the trailhead in metres, measured along and to the right
        -- of the first segment. Yaw is relative to Warp's -z forward axis.
        carAlong=2.6, carRight=-6.92,
    },
    terrain={
        x=-1024,z=-160,spacing=32,columns=20,rows=11,maxGrade=.25,
        -- {centre x, centre z, radius x, radius z, peak amplitude}.
        hills={{-706,-64,109,62,37},{-865,66,82,77,42},
            {-561,42,67,55,15},{-950,-62,65,50,12}},
    },
    -- Forest ellipses {centre x, centre z, radius x, radius z}.
    groves={{-690,-99,120,83},{-900,108,80,65},{-516,102,48,25},
        {-748,-191,76,32},{-896,192,64,30},{-1062,42,65,31}},
    places={
        grainYard={-255,26},lookout={-1111,0},marinerWood={-674,-116},
        lookoutMeadow={-1080,11},lookoutStop={-1093,0},ferryTerminal={164.5,59},
    },
}
