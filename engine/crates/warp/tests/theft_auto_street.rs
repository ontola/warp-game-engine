//! Street widths, pedestrian routes and the nighttime light pool use the generated city.
use app::{Controls, Game};
use std::path::Path;
use warp::{LuaGame, Overrides};

#[test]
fn street_furniture_stays_at_the_curb_and_lights_are_shared_across_seats() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/warp-theft-auto");
    let mut layouts = Vec::new();
    for seed in [1948, 925, 53154, 1948] {
        let mut game = LuaGame::load(
            &dir,
            &Overrides {
                seed: Some(seed),
                players: Some(4),
                ..Default::default()
            },
            None,
        )
        .unwrap();
        game.echo = false;
        for _ in 0..5 {
            game.tick(&[Controls::default(); 4]);
        }
        assert!(game.errors().is_empty(), "{:?}", game.errors());
        let layout = game.luau(r#"
            local city=state.city
            local lamps,bins=world.find("street-lamp"),world.find("street-bin")
            local layout={}
            local function check(id,offset)
                local e=world.get(id)
                local p=e.pos
                local x=math.abs(p.x-56*math.round(p.x/56))
                local z=math.abs(p.z-56*math.round(p.z/56))
                assert(math.abs(math.min(x,z)-offset)<.001 and math.max(x,z)>12,"curb/crossing")
                for _,b in city.buildings do
                    assert(math.abs(p.x-b.door.x)>=1.8 or p.z<b.door.z-.5 or p.z>=b.door.z+16,"door")
                end
                table.insert(layout,string.format("%.2f,%.2f",p.x,p.z))
                return p,e
            end
            local starterLamp,starterBin=false,false
            for i,id in lamps do
                local p,e=check(id,6.74)
                local d=vector.create(math.sin(e.rot.y),0,math.cos(e.rot.y))
                local head=p+1.87*d+vector.create(0,5.23,0)
                assert(vector.magnitude(head-city.lamps[i])<.002)
                local across=math.min(math.abs(head.x-56*math.round(head.x/56)),math.abs(head.z-56*math.round(head.z/56)))
                assert(across<5,"lamp must overhang the road")
                starterLamp=starterLamp or vector.magnitude(p-vector.create(6.74,0,17.4))<.002
            end
            for _,id in bins do
                local p,e=check(id,6.78)
                assert(e.solid,"bin should have a compact collision footprint")
                starterBin=starterBin or vector.magnitude(p-vector.create(6.78,0,14.1))<.002
                for _,lamp in lamps do assert(vector.magnitude(p-world.get(lamp).pos)>1.75) end
            end
            assert(starterLamp and starterBin and #city.lamps==#lamps)
            assert(#lamps>=45 and #bins>=22)
            return table.concat(layout,";")
        "#).unwrap();
        game.luau("state.clock.hour=23").unwrap();
        for _ in 0..16 {
            game.tick(&[Controls::default(); 4]);
        }
        assert!(
            game.core()
                .lamps
                .iter()
                .any(|lamp| (lamp.pos.x - 4.87).abs() < 0.002
                    && (lamp.pos.y - 5.23).abs() < 0.002
                    && (lamp.pos.z - 17.4).abs() < 0.002
                    && lamp.intensity >= 500.
                    && lamp.range >= 20.),
            "the starter lamp must light the street after dusk"
        );
        let counts=game.luau("return {lamps=#world.find('street-lamp'), bins=#world.find('street-bin'), base=world.count()}").unwrap();
        game.luau(
            r#"
            for i,dest in {{6,14},{-104,75},{120,83},{-104,-93}} do
                world.place(state.fleet.players[i].body,vector.create(dest[1],.85,dest[2]))
            end
        "#,
        )
        .unwrap();
        for _ in 0..16 {
            game.tick(&[Controls::default(); 4]);
        }
        let night = game.core().lamps.len();
        println!("street seed {seed} {counts}, four districts at night {night}");
        assert!((12..=20).contains(&night));
        game.luau("state.clock.hour=12").unwrap();
        for _ in 0..16 {
            game.tick(&[Controls::default(); 4]);
        }
        assert!(game.core().lamps.is_empty());
        assert!(game.errors().is_empty(), "{:?}", game.errors());
        layouts.push(layout);
    }
    assert_eq!(layouts[0], layouts[3]);
    assert_ne!(layouts[0], layouts[1]);
    assert_ne!(layouts[1], layouts[2]);
}
