//! The edited ribbon and terrain must share the contact grid, including the
//! diagonal through a cell. Also exercise validation without throwing a Luau
//! exception, as live browser editors can recover only ordinary error returns.
use std::path::Path;
use warp::{LuaGame, Overrides};

#[test]
fn edited_cape_mesh_follows_the_heightfield_and_rejects_bad_triangles_atomically() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let mut game = LuaGame::load(&dir, &Overrides {seed:Some(1948), ..Default::default()}, None).unwrap();
    let code=r#"
        local d=table.clone(require("west-cape-data"))
        d.widths=d.widths or {asphalt=9,gravel=5.2}
        d.roads=table.clone(d.roads)
        d.roads[3]=table.clone(d.roads[3])
        local i=math.floor(#d.roads[3]/2)
        d.roads[3][i]={d.roads[3][i][1]-6,d.roads[3][i][2]+2}
        local function height(x,z)
            local t=d.terrain
            local u,v=(x-t.x)/t.spacing,(z-t.z)/t.spacing
            local ix,iz=math.floor(u),math.floor(v)
            if ix<0 or iz<0 or ix>=t.columns-1 or iz>=t.rows-1 then return 0 end
            local k=iz*t.columns+ix+1
            local a,b,c,e=t.heights[k],t.heights[k+1],t.heights[k+t.columns],t.heights[k+t.columns+1]
            local fx,fz=u-ix,v-iz
            if fx+fz<=1 then return a+(b-a)*fx+(c-a)*fz end
            return e+(c-e)*(1-fx)+(b-e)*(1-fz)
        end
        local mesh=require("cape_mesh")
        local parts=mesh.prepare(d)
        local checked=0
        for _,part in parts do
            if part.surface==40 or part.surface==41 then
                local above=if part.surface==40 then 0 else .025
                for j=1,#part.indices,3 do
                    local a,b,c=part.positions[part.indices[j]],part.positions[part.indices[j+1]],part.positions[part.indices[j+2]]
                    local mid=(a+b+c)/3
                    assert(math.abs(mid.y-height(mid.x,mid.z)-above)<.005,"triangle crosses the physical terrain")
                    checked+=1
                end
            end
        end
        assert(checked>500)
        local initial=world.count()
        local id,err=mesh.spawnPrepared(parts)
        assert(id,err)
        assert(world.get(id).shape=="mesh" and world.count()==initial+1)
        local last=parts[#parts]
        last.indices[#last.indices]=#last.positions+1
        local invalid,message=mesh.spawnPrepared(parts)
        assert(invalid==nil and type(message)=="string" and world.count()==initial+1)
        assert(world.remove(id) and world.count()==initial)
    "#;
    // The interactive console intentionally cannot import files. Evaluate the
    // three shipped modules with their normal dependencies in the same Luau VM.
    let mut session=String::from("local modules={}\nlocal function require(k) return assert(modules[k],k) end\n");
    for name in ["west_cape_layout","west-cape-data","cape_mesh"] {
        let module=std::fs::read_to_string(dir.join(format!("{name}.module.luau"))).unwrap();
        session.push_str(&format!("modules['{name}']=(function()\n{module}\nend)()\n"));
    }
    session.push_str(code);
    game.luau(&session).unwrap();
}
