-- One clustered mesh for a whole stand. The baker reads these trunk roots too,
-- so its visible stems and the game's collision circles share a coordinate frame.
local forest = {}
local roots = {
    -- Local x, z, uniform scale, heading (degrees), species.
    {-9.3,-9.0,1.02,41,"pine"}, {-3.0,-9.2,.81,173,"birch"}, {4.0,-10.4,1.15,206,"fir"},
    {10.2,-7.3,.86,134,"pine"}, {-8.1,-3.7,1.04,301,"fir"}, {-.8,-2.3,1.20,89,"pine"},
    {5.5,-3.1,.91,28,"birch"}, {11.0,.9,.94,249,"fir"}, {-10.5,3.8,.94,214,"birch"},
    {-3.9,2.5,1.06,322,"pine"}, {3.3,3.3,.89,62,"fir"}, {7.6,7.0,1.08,192,"pine"},
    {-7.6,9.9,.92,275,"fir"}, {-.8,9.2,1.14,5,"birch"}, {3.6,11.1,.83,124,"pine"},
}
-- A short, narrow group can follow a contour through a banked bend. Each
-- entry reuses the species, height and rotation of a tree in the full stand.
local trailRoots = {
    {2,-5.1,-.60}, {6,-1.7,.88}, {3,2.1,-.77}, {13,5.7,.48},
}
local radius = {pine=.32, fir=.29, birch=.22}
local height = {pine=10.95, fir=12.2, birch=9.77}
forest.count = #roots

function forest.options(x, y, z, yaw, scale)
    return {
        pos=vector.create(x,y,z), rot=vector.create(0,yaw,0), size=vector.create(scale,scale,scale),
        solid=false, tag="woodland",
    }
end

function forest.trunks(x, z, yaw, scale, visit)
    local c,s = math.cos(yaw)*scale, math.sin(yaw)*scale
    for _, tree in roots do
        local dx,dz,k,species = tree[1],tree[2],tree[3]*scale,tree[5]
        visit(x+c*dx+s*dz, z-s*dx+c*dz, radius[species]*k, height[species]*k)
    end
end

function forest.trailTrunks(x,z,yaw,scale,visit)
    local c,s=math.cos(yaw)*scale,math.sin(yaw)*scale
    for _,root in trailRoots do
        local tree=roots[root[1]]
        local k=tree[3]*scale
        visit(x+c*root[2]+s*root[3],z-s*root[2]+c*root[3],radius[tree[5]]*k,height[tree[5]]*k)
    end
end

return forest