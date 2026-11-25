const assert = require('node:assert/strict');
const {readFileSync} = require('node:fs');
const {join} = require('node:path');
const {test} = require('node:test');
const {runInNewContext} = require('node:vm');
const script = readFileSync(join(__dirname, '../../lobbies/game-room/examples/browser/web-catalog.js'), 'utf8');
const titles = ['ballkickers', 'frog-fighter'].map(id => ({
  id, folder:id, title:id, min_players:1, max_players:4, has_cover:true, web_entry:'godot/' + id + '/index.html',
}));

async function read({extra=titles, origin='https://splitscreen.club', path='/play/', custom=false, base=[]}={}) {
  const requests = [], window = {};
  runInNewContext(script, {
    window, location:{origin,pathname:path}, AbortSignal, document:{documentElement:{dataset:{version:'v'},hasAttribute:()=>custom}},
    fetch:async url => {
      requests.push(url);
      if (url.startsWith('games/library.json')) return {ok:true,json:async()=>[...base]};
      if (extra instanceof Error) throw extra;
      return {ok:extra!==null,json:async()=>extra};
    },
  });
  const pending = window.clubGameLibrary();
  assert.equal(window.clubGameLibrary(), pending);
  return {games:await pending, requests};
}

test('production merges valid same-origin entries and retains packaged games and order',async()=>{
  const existing={id:'spaceracer',folder:'spaceracer'};
  const {games,requests}=await read({base:[existing,titles[0]],extra:[titles[0],{...titles[1],web_entry:'https://elsewhere.invalid/'},titles[1],titles[1]]});
  assert.deepEqual(games.map(g=>g.id), ['spaceracer','ballkickers','frog-fighter']);
  assert.equal(games[1],titles[0]);
  assert.equal(games[2].web_cover,'godot/frog-fighter/cover.png');
  assert.deepEqual(requests,['games/library.json?v=v','/play/godot/library.json']);
});

test('new managed games use one slug for their folder, entry point and cover', async () => {
  const game = {
    ...titles[0], id:'bombing-bugs', folder:'bombing-bugs', title:'Bombing Bugs',
    web_entry:'godot/bombing-bugs/index.html',
  };
  const invalid = [
    ...['', '../other', 'other/game', 'other%2fgame', 'UPPER', 'two--parts', 123].map(id => ({
      ...game, id, folder:id, web_entry:'godot/' + id + '/index.html',
    })),
    {...game, folder:'another-folder'},
    {...game, web_entry:'godot/another-folder/index.html'},
    {...game, web_entry:'https://splitscreen.club/play/godot/bombing-bugs/index.html'},
    {...game, min_players:0},
    {...game, max_players:9},
  ];
  const {games} = await read({extra:[...invalid, game, game]});
  assert.deepEqual(games.map(row => row.id), ['bombing-bugs']);
  assert.deepEqual({...games[0]}, {...game, web_cover:'godot/bombing-bugs/cover.png'});

  const packaged = {id:'another-id', folder:game.folder};
  assert.deepEqual((await read({base:[packaged], extra:[game]})).games, [packaged]);
});

test('other origins, paths and custom shelves make no overlay request',async()=>{
  for (const options of [{origin:'http://localhost:8790'}, {origin:'https://other.example'}, {path:'/archive/'}, {custom:true}]) {
    const {games,requests}=await read(options);
    assert.equal(games.length,0); assert.equal(requests.length,1);
  }
});

test('unavailable, timed-out and malformed manifests leave bundled games intact',async()=>{
  for (const extra of [null,new Error('timeout'),{},[null,{...titles[0],max_players:0}]]) {
    const {games}=await read({base:[titles[0]],extra});
    assert.deepEqual(games,[titles[0]]);
  }
});
