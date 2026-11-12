import { XRHost, THREE } from '../../../../engine/webxr/host.mjs';
import { FixedStep, Edges, SnapTurn } from '../../../../engine/webxr/input.mjs';
import init, { QuestGame } from './pkg/growing_guns_quest.js';

const $ = id => document.getElementById(id);
const status = $('status');
const clock = new FixedStep(), edges = new Edges(), turn = new SnapTurn(), choice = new SnapTurn();
const up = new THREE.Vector3(0,1,0), temp = new THREE.Object3D();
let game, current, previous, revision = -1, mode = 'landing', paused = true;
let yaw = 0, desktopPitch = 0, lastHead = null, pendingRoom = new THREE.Vector3();
let latestXR = null, selection = 0, panelKey = '', wristKey = '', newRound = 0;
let localShots=0, lastShot=null;
let audio, ticks = 0, lastEvents = [], errorMessage = '', panelShown = false;
let leftHand, rightHand, weapon, wrist, wristCanvas, wristContext, panel, panelCanvas, panelContext;
let arena, bullets, grenades, lava;
const robots = [], effects = [], keys = new Set();
const mouse = {fire:false,special:false};
let desktopPointer = false;
const host = new XRHost($('game'), {frame, session, visibility, recenter: clearTracking});
const {scene, camera, rig, renderer} = host;
scene.background = new THREE.Color('#9fb8c2');
scene.fog = new THREE.Fog('#9fb8c2', 85, 260);
scene.add(new THREE.HemisphereLight('#d9f1ff','#534438',2.4));
const sunlight = new THREE.DirectionalLight('#fff0d5',2.5); sunlight.position.set(-35,70,25); scene.add(sunlight);
const boxGeo = new THREE.BoxGeometry(1,1,1);
const stoneMaterial = new THREE.MeshLambertMaterial({color:0xffffff});
const palette = ['#65564a','#b4997c','#4b5155','#85776d'];
const teamColors = ['#66e5cf','#ff7769','#ffc75c','#a596ff'];
const material = c => new THREE.MeshLambertMaterial({color:c});
function box(parent, size, pos, color) {
  const m = new THREE.Mesh(boxGeo, material(color)); m.scale.set(...size); m.position.set(...pos); parent.add(m); return m;
}
function robot(color) {
  const g = new THREE.Group();
  box(g,[.58,.65,.34],[0,.02,0],color);
  box(g,[.46,.4,.4],[0,.6,0],'#d8d9cf');
  box(g,[.36,.12,.03],[0,.63,-.215],'#153c4b');
  box(g,[.23,.1,.035],[0,.63,-.235],color);
  box(g,[.18,.48,.2],[-.4,0,0],'#394c54');
  box(g,[.18,.48,.2],[.4,0,0],'#394c54');
  const legs = [box(g,[.22,.53,.24],[-.17,-.58,0],'#283b46'),box(g,[.22,.53,.24],[.17,-.58,0],'#283b46')];
  const barrel=box(g,[.13,.13,.65],[.35,.05,-.42],'#23323c');
  const sight=box(g,[.14,.035,.24],[.35,.14,-.46],color);
  scene.add(g); return {group:g,legs,barrel,sight,ground:gun(scene)};
}
function gun(parent) {
  const g = new THREE.Group(); parent.add(g);
  box(g,[.09,.11,.3],[0,-.015,-.12],'#24353f');
  box(g,[.065,.07,.22],[0,.005,-.33],'#15242d');
  box(g,[.055,.14,.09],[0,-.1,-.01],'#725942').rotation.x=-.2;
  box(g,[.095,.017,.12],[0,.05,-.11],'#ffbf59');
  box(g,[.018,.02,.025],[0,.05,-.35],'#67f7ce');
  return g;
}
function canvasPanel(width, height, physicalWidth, physicalHeight) {
  const canvas = document.createElement('canvas'); canvas.width=width; canvas.height=height;
  const texture = new THREE.CanvasTexture(canvas); texture.colorSpace = THREE.SRGBColorSpace;
  const mesh = new THREE.Mesh(new THREE.PlaneGeometry(physicalWidth,physicalHeight),new THREE.MeshBasicMaterial({map:texture,side:THREE.DoubleSide}));
  return [mesh,canvas,canvas.getContext('2d')];
}
function setup() {
  for(let i=0;i<4;i++) robots.push(robot(teamColors[i]));
  bullets = new THREE.InstancedMesh(new THREE.SphereGeometry(1,6,4),new THREE.MeshBasicMaterial({color:'#fff6b9'}),2048);
  grenades = new THREE.InstancedMesh(new THREE.IcosahedronGeometry(1,0),material('#8aff9c'),128);
  bullets.frustumCulled=false; grenades.frustumCulled=false; scene.add(bullets,grenades);
  lava = new THREE.Mesh(new THREE.PlaneGeometry(300,300),new THREE.MeshBasicMaterial({color:'#d65d2e'}));
  lava.rotation.x=-Math.PI/2; lava.position.y=-3; scene.add(lava);
  leftHand=new THREE.Group(); rightHand=new THREE.Group(); rig.add(leftHand,rightHand);
  box(leftHand,[.085,.085,.13],[0,0,0],'#5bbfae'); weapon=gun(rightHand);
  [wrist,wristCanvas,wristContext] = canvasPanel(512,256,.34,.17);
  leftHand.add(wrist); wrist.position.set(0,.17,-.1); wrist.rotation.x=-.65;
  [panel,panelCanvas,panelContext] = canvasPanel(1024,720,1.35,.95);
  panel.visible=false; scene.add(panel);
  buildArena();
}
function buildArena() {
  const geometry = game.geometry();
  if(arena) { scene.remove(arena); arena.dispose(); }
  arena=new THREE.InstancedMesh(boxGeo,stoneMaterial,geometry.length/8);
  const color=new THREE.Color();
  for(let i=0;i<geometry.length;i+=8) {
    temp.position.set(geometry[i],geometry[i+1],geometry[i+2]);
    temp.rotation.set(0,geometry[i+6],0);
    temp.scale.set(...Array.from(geometry.slice(i+3,i+6),v=>Math.max(.02,v-.012)));
    temp.updateMatrix(); arena.setMatrixAt(i/8,temp.matrix);
    color.set(palette[geometry[i+7]]).multiplyScalar(.9+((i*37)%19)/100);
    arena.setColorAt(i/8,color);
  }
  arena.computeBoundingSphere(); scene.add(arena); revision=current.revision;
}
function clearTracking() {
  clock.reset(); edges.clear(); turn.reset(); choice.reset(); pendingRoom.set(0,0,0); lastHead=null;
  if(current) { current.roomApplied=[0,0,0]; previous=current; }
  keys.clear(); mouse.fire=false; mouse.special=false;
}
function session(started) {
  clearTracking(); latestXR=null;
  if(started) {
    mode='xr'; yaw=current.players[0].yaw; paused=true; selection=0; panelShown=false;
    $('landing').hidden=true; $('hud').hidden=true; $('overlay').hidden=true;
  } else {
    mode='landing'; paused=true; leftHand.visible=rightHand.visible=false; panel.visible=false;
    $('landing').hidden=false; panelShown=false; camera.quaternion.identity();
    $('enter').disabled=false; $('enter').textContent='Start VR op je headset';
    status.textContent='VR afgesloten. Je kunt de demo opnieuw starten.';
  }
}
function visibility(state) {
  clearTracking();
  // System menu, headset removal, and blurred tracking never leave a gun firing.
  if(state!=='visible') { paused=true; panelShown=false; }
}
function togglePause(value) {
  const held=edges.held, room=pendingRoom.clone(), head=lastHead;
  paused=value; clearTracking(); edges.held=held; pendingRoom.copy(room); lastHead=head; panelShown=false; panelKey=''; selection=0;
  if(mode==='desktop') {
    $('overlay').hidden=!paused;
    if(paused) document.exitPointerLock(); else $('game').requestPointerLock?.()?.catch(()=>{});
  }
}
function restart() {
  game.restart(); current=JSON.parse(game.frame()); previous=current;
  newRound=current.round; selection=0; clearTracking();
}
function handTransform(group, pose) {
  group.position.fromArray(pose.position); group.quaternion.fromArray(pose.quaternion);
}
function xrControls(xr) {
  const h=new THREE.Vector3().fromArray(xr.head.position);
  if(lastHead) {
    const delta=h.clone().sub(lastHead); delta.y=0;
    if(delta.length()<.5) pendingRoom.add(delta.applyAxisAngle(up,yaw));
  }
  lastHead=h;
  const left=xr.hands.left, right=xr.hands.right;
  leftHand.visible=!!left; rightHand.visible=!!right;
  if(left) handTransform(leftHand,left.grip??left.aim);
  if(right) handTransform(rightHand,right.aim);
  edges.poll({fire:right?.trigger, jump:right?.primary, reload:right?.squeeze||left?.primary,
    pause:right?.secondary, recenter:left?.secondary, special:left?.trigger});
  if(edges.take('pause')) togglePause(!paused);
  if(edges.take('recenter')) { panelShown=false; lastHead=null; }
  const offering=!!current.players[0].offer&&!paused&&current.phase!=='MatchOver';
  if(offering || paused || current.phase==='MatchOver') {
    const delta=choice.update(left?.stick[0]??0);
    const n=offering?current.players[0].offer.length:3;
    if(delta) selection=(selection+(delta<0?1:-1)+n)%n;
  } else {
    const delta=turn.update(right?.stick[0]??0);
    if(delta) { yaw+=delta; panelShown=false; }
  }
  return {left,right,h};
}
function controls(xrParts) {
  if(mode==='xr') {
    const {left,right,h}=xrParts;
    const q=new THREE.Quaternion().fromArray(latestXR.head.quaternion);
    const f=new THREE.Vector3(0,0,-1).applyQuaternion(q).applyAxisAngle(up,yaw);
    let aim=null;
    if(right) {
      const direction=new THREE.Vector3(0,0,-1).applyQuaternion(new THREE.Quaternion().fromArray(right.aim.quaternion));
      const offset=new THREE.Vector3().fromArray(right.aim.position).addScaledVector(direction,.43);
      offset.x-=h.x; offset.z-=h.z; offset.y-=.9;
      offset.applyAxisAngle(up,yaw); direction.applyAxisAngle(up,yaw);
      aim={offset:offset.toArray(),direction:direction.toArray()};
    }
    const roomStep=pendingRoom.clone().clampLength(0,.45);
    const room=roomStep.toArray(); pendingRoom.sub(roomStep);
    const fireEdge=edges.take('fire');
    return {mv:[left?.stick[0]??0,-(left?.stick[1]??0)], yaw:Math.atan2(-f.x,-f.z),pitch:Math.asin(f.y),
      fire:!!right?.trigger&&!current.players[0].offer, jump:edges.take('jump'),reload:edges.take('reload'),
      special:!!left?.trigger, aim, room,
      pick:current.players[0].offer&&fireEdge?selection:null};
  }
  return {mv:[Number(keys.has('KeyD'))-Number(keys.has('KeyA')),Number(keys.has('KeyW'))-Number(keys.has('KeyS'))],
    yaw, pitch:desktopPitch, fire:mouse.fire, special:mouse.special,
    jump:edges.take('jump'),reload:edges.take('reload'),pick:queuedPick};
}
let queuedPick=null;
function frame(time,xr) {
  if(!game) return;
  let parts=null;
  if(mode==='xr') {
    latestXR=xr;
    if(!xr?.tracked) { clearTracking(); leftHand.visible=rightHand.visible=false; return; }
    parts=xrControls(xr);
    if((paused||current.phase==='MatchOver')&&edges.take('fire')) {
      if(selection===2) { host.end(); return; }
      if(selection===1||current.phase==='MatchOver') restart();
      togglePause(false);
    }
  }
  let alpha=1;
  if(mode==='landing'||paused||current.phase==='MatchOver') clock.reset();
  else {
    alpha=clock.advance(time,()=>{
      previous=current;
      lastEvents=JSON.parse(game.tick(JSON.stringify(controls(parts))));
      queuedPick=null; current=JSON.parse(game.frame()); ticks++;
      for(const event of lastEvents) feedback(event);
      if(current.round!==newRound) { previous=current; newRound=current.round; selection=0; lastHead=null; pendingRoom.set(0,0,0); edges.clear(); }
    });
  }
  if(current.revision!==revision) buildArena();
  const p=current.players[0];
  for(let i=0;i<robots.length;i++) {
    const r=robots[i], pl=current.players[i], old=previous.players[i];
    r.barrel.visible=r.sight.visible=pl.armed;
    r.ground.visible=!!pl.groundGun;
    if(pl.groundGun) {
      r.ground.position.fromArray(pl.groundGun);
      r.ground.position.y+=.045;
      r.ground.rotation.z=Math.PI/2;
    }
    r.group.visible=pl.alive&&(i!==0||mode==='landing');
    r.group.position.fromArray(old.pos).lerp(new THREE.Vector3().fromArray(pl.pos),alpha);
    r.group.rotation.y=old.yaw+Math.atan2(Math.sin(pl.yaw-old.yaw),Math.cos(pl.yaw-old.yaw))*alpha;
    const moved=new THREE.Vector3().fromArray(old.pos).distanceTo(new THREE.Vector3().fromArray(pl.pos))>.004;
    r.legs[0].rotation.x=moved?Math.sin(time*.012)*.45:0;
    r.legs[1].rotation.x=-r.legs[0].rotation.x;
  }
  updateInstances(bullets,current.bullets,.075,alpha,4);
  updateInstances(grenades,current.grenades,.15,alpha,3);
  lava.position.y=current.lava;
  if(mode==='landing') {
    rig.position.set(0,0,0); rig.rotation.set(0,0,0);
    camera.position.set(24,24,35); camera.lookAt(-5,3,0);
    leftHand.visible=rightHand.visible=false;
  } else {
    const body=new THREE.Vector3().fromArray(previous.players[0].pos).lerp(new THREE.Vector3().fromArray(p.pos),alpha);
    rig.rotation.set(0,yaw,0);
    if(mode==='xr') {
      // Interpolate virtual locomotion only. Apply physical head translation
      // immediately, including render frames without a sim tick and pause menus.
      body.addScaledVector(new THREE.Vector3().fromArray(current.roomApplied),1-alpha).add(pendingRoom);
      const offset=parts.h.clone(); offset.y=0; offset.applyAxisAngle(up,yaw);
      rig.position.copy(body).sub(offset); rig.position.y=body.y-.9;
    } else {
      rig.position.copy(body); camera.position.set(0,.7,0); camera.rotation.set(desktopPitch,0,0,'YXZ');
      leftHand.visible=false; rightHand.visible=true;
      rightHand.position.set(.25,.45,-.25); rightHand.quaternion.copy(camera.quaternion);
    }
  }
  weapon.visible=p.armed;
  const scale=1+Math.min(p.cards,10)*.06; weapon.scale.setScalar(scale);
  // Preserve muzzle position when the weapon grows; the sim's .43m barrel end
  // and visible barrel stay together. Only width/height grow in this demo.
  weapon.scale.z=1;
  updateUI();
  for(let i=effects.length-1;i>=0;i--) {
    const effect=effects[i]; effect.ttl-=1/72;
    effect.mesh.scale.multiplyScalar(1.035); effect.mesh.material.opacity=Math.max(0,effect.ttl*2);
    if(effect.ttl<=0) {scene.remove(effect.mesh);effect.mesh.geometry.dispose();effect.mesh.material.dispose();effects.splice(i,1);}
  }
}
function updateInstances(mesh, data, radius, alpha, velocityOffset) {
  mesh.count=Math.min(data.length,mesh.instanceMatrix.count);
  temp.rotation.set(0,0,0);temp.scale.setScalar(radius);
  for(let i=0;i<mesh.count;i++) {const dt=(1-alpha)/60;temp.position.set(data[i][0]-data[i][velocityOffset]*dt,data[i][1]-data[i][velocityOffset+1]*dt,data[i][2]-data[i][velocityOffset+2]*dt);temp.updateMatrix();mesh.setMatrixAt(i,temp.matrix);}
  mesh.instanceMatrix.needsUpdate=true;
}
function startAudio() { audio??=new AudioContext(); audio.resume().catch(()=>{}); }
function tone(frequency, duration, volume) {
  if(!audio||audio.state!=='running') return;
  const o=audio.createOscillator(),g=audio.createGain(); o.type='triangle';o.frequency.setValueAtTime(frequency,audio.currentTime);
  o.frequency.exponentialRampToValueAtTime(40,audio.currentTime+duration);
  g.gain.setValueAtTime(volume,audio.currentTime);g.gain.exponentialRampToValueAtTime(.0001,audio.currentTime+duration);
  o.connect(g);g.connect(audio.destination);o.start();o.stop(audio.currentTime+duration);
}
function feedback(e) {
  if(e.Shot) {
    const local=e.Shot.player===0;
    tone(local?180:120,.1,local?.07:.012);
    if(local) { localShots++; lastShot=e.Shot; latestXR?.hands.right?.pulse(.3,35); }
  }
  if(e.Hit?.victim===0) latestXR?.hands.left?.pulse(.65,80);
  if(e.Explosion) {
    tone(70,.3,.09);
    const mesh=new THREE.Mesh(new THREE.IcosahedronGeometry(Math.min(3,e.Explosion.radius)*.5,1),new THREE.MeshBasicMaterial({color:'#ffba54',transparent:true,opacity:.7,depthWrite:false,wireframe:true}));
    mesh.position.fromArray(e.Explosion.pos);scene.add(mesh);effects.push({mesh,ttl:.4});
  }
}
function positionPanel() {
  if(mode!=='xr'||!latestXR?.tracked)return;
  // Place at eye level in the world, not permanently stuck to the face.
  rig.updateMatrixWorld(true);
  const h=new THREE.Vector3().fromArray(latestXR.head.position).applyMatrix4(rig.matrixWorld);
  const forward=new THREE.Vector3(0,0,-1).applyQuaternion(new THREE.Quaternion().fromArray(latestXR.head.quaternion)).applyAxisAngle(up,yaw);
  forward.y=0;forward.normalize();panel.position.copy(h).addScaledVector(forward,1.65);
  panel.lookAt(h);
  panelShown=true;
}
function drawPanel(ctx,title,subtitle,rows,selected) {
  const w=ctx.canvas.width,h=ctx.canvas.height;
  ctx.fillStyle='#112632';ctx.fillRect(0,0,w,h);
  ctx.fillStyle='#ffbe59';ctx.font='bold 24px sans-serif';ctx.fillText('W / WARP     GROWING GUNS',48,60);
  ctx.fillStyle='#fff5e5';ctx.font='bold 48px sans-serif';ctx.fillText(title,48,136);
  ctx.fillStyle='#a5c2cb';ctx.font='25px sans-serif';ctx.fillText(subtitle,48,185);
  rows.forEach((r,i)=>{
    const y=222+i*126;ctx.fillStyle=i===selected?'#ffbe59':'#213d4b';ctx.fillRect(40,y,w-80,110);
    ctx.fillStyle=i===selected?'#142630':'#f4f4e9';ctx.font='bold 30px sans-serif';ctx.fillText(r[0],64,y+43);
    ctx.font='24px sans-serif';ctx.fillText(r[1]??'',64,y+82);
  });
  ctx.fillStyle='#acc3c8';ctx.font='21px sans-serif';ctx.fillText('Linkerstick: kiezen    Rechtertrekker: bevestigen',48,h-32);
}
function updateUI() {
  const p=current.players[0];
  const match=current.phase==='MatchOver';
  const offer=!paused&&!match?p.offer:null;
  const summary=`HP ${Math.max(0,p.hp)} / ${p.maxHp}   •   ${p.reload?'HERLADEN':`${p.ammo} / ${p.mag}`}   •   RONDE ${current.round}`;
  $('stats').textContent=summary;
  let rows, title, sub;
  if(offer) {title='KIES JE UPGRADE';sub='Verloren ronde? Groter geweer.';rows=offer.map(c=>[c.name,c.desc]);}
  else {title=match?'MATCH AFGELOPEN':(ticks?'EVEN PAUZE':'KLAAR VOOR DE ARENA?');sub='A: spring   Grip: herlaad   Linkertrekker: granaat';rows=[[match?'OPNIEUW SPELEN':'VERDER SPELEN','Linkerstick lopen · rechterstick 30° draaien'],['NIEUWE MATCH','Begin opnieuw tegen drie bots'],['VERLAAT VR','Terug naar het startscherm']];}
  selection=Math.min(selection,rows.length-1);
  panel.visible=mode==='xr'&&(paused||!!offer||match);
  if(panel.visible) {
    if(!panelShown) positionPanel();
    const key=JSON.stringify([title,rows,selection]);
    if(key!==panelKey) {drawPanel(panelContext,title,sub,rows,selection);panel.material.map.needsUpdate=true;panelKey=key;}
  } else panelShown=false;
  if(summary!==wristKey) {
    wristContext.fillStyle='#102630';wristContext.fillRect(0,0,512,256);
    wristContext.fillStyle='#ffcf75';wristContext.font='bold 34px sans-serif';wristContext.fillText(`HP ${Math.max(0,p.hp)}    ${p.reload?'...':p.ammo+' / '+p.mag}`,25,60);
    wristContext.fillStyle='#d0e6e9';wristContext.font='26px sans-serif';wristContext.fillText(`RONDE ${current.round}   SCORE ${p.score}`,25,115);
    wristContext.font='24px sans-serif';wristContext.fillText('B: pauze  ·  Y: paneel hier',25,172);
    wristContext.fillText(p.ghost?'Uitgeschakeld · wacht op ronde':'Grip: herladen',25,220);
    wrist.material.map.needsUpdate=true;wristKey=summary;
  }
  if(mode==='desktop') {
    $('overlay').hidden=!(paused||offer||match);$('dialog-title').textContent=title;
    const key=JSON.stringify(offer);
    if($('cards').dataset.key!==key) {
      $('cards').dataset.key=key;$('cards').replaceChildren();
      offer?.forEach((c,i)=>{const b=document.createElement('button');b.textContent=c.name;const small=document.createElement('small');small.textContent=c.desc;b.append(small);b.onclick=()=>{queuedPick=i;};$('cards').append(b);});
      if(offer)document.exitPointerLock();
    }
    $('resume').hidden=!!offer||match;
  }
}
$('enter').onclick=()=>{
  startAudio();$('enter').disabled=true;
  host.enter().catch(e=>{errorMessage=e.message;status.textContent=`VR kon niet starten: ${e.message}. Probeer opnieuw vanuit Quest Browser.`;$('enter').disabled=false;});
};
$('desktop').onclick=()=>{
  startAudio();mode='desktop';yaw=current.players[0].yaw;desktopPitch=0;paused=false;clearTracking();
  $('landing').hidden=true;$('hud').hidden=false;$('game').requestPointerLock?.()?.catch(()=>{});
};
$('menu').onclick=()=>togglePause(true);$('resume').onclick=()=>togglePause(false);
$('restart').onclick=()=>{restart();togglePause(false);};
$('back').onclick=()=>{document.exitPointerLock();session(false);$('hud').hidden=true;$('overlay').hidden=true;};
window.addEventListener('keydown',e=>{
  if(mode!=='desktop')return;
  if(['Space','ArrowUp','ArrowDown'].includes(e.code))e.preventDefault();
  keys.add(e.code);edges.poll({jump:keys.has('Space'),reload:keys.has('KeyR')});
  if(e.code==='Escape')togglePause(true);
});
window.addEventListener('keyup',e=>{keys.delete(e.code);edges.poll({jump:keys.has('Space'),reload:keys.has('KeyR')});});
window.addEventListener('mousemove',e=>{if(desktopPointer&&!paused){yaw-=e.movementX*.002;desktopPitch=THREE.MathUtils.clamp(desktopPitch-e.movementY*.002,-1.3,1.3);}});
$('game').addEventListener('mousedown',e=>{if(mode==='desktop'&&!paused){if(e.button===0)mouse.fire=true;if(e.button===2)mouse.special=true;}});
window.addEventListener('mouseup',()=>{mouse.fire=false;mouse.special=false;});
$('game').addEventListener('contextmenu',e=>e.preventDefault());
document.addEventListener('pointerlockchange',()=>{desktopPointer=document.pointerLockElement===$('game');if(!desktopPointer&&mode==='desktop'&&!current?.players[0].offer)togglePause(true);});
window.addEventListener('blur',()=>{if(mode==='desktop')togglePause(true);});
document.addEventListener('visibilitychange',()=>{if(document.hidden){clearTracking();paused=true;}});

// Read-only diagnostics for on-device remote inspection and browser validation.
Object.defineProperty(window,'warpDemo',{get:()=>({ready:!!game,mode,paused,ticks,localShots,lastShot,views:host.lastViews,drawCalls:renderer.info.render.calls,
  triangles:renderer.info.render.triangles,frameCount:host.frameCount,state:current,lastEvents,error:errorMessage,
  rigYaw:yaw,cameraPosition:camera.getWorldPosition(new THREE.Vector3()).toArray(),handVisible:rightHand?.visible,
  arenaInstances:arena?.count,panelVisible:panel?.visible})});
try {
  await init(); game=new QuestGame(3);current=JSON.parse(game.frame());previous=current;newRound=current.round;setup();
  $('desktop').disabled=false;
  const mobile=/Android|iPhone|iPad/.test(navigator.userAgent);
  if(mobile){$('desktop').textContent='Bekijk de arena';$('desktop').onclick=()=>{status.textContent='Open deze pagina in Quest Browser en kies Start VR. De computervoorvertoning gebruikt muis en toetsenbord.';};}
  if(await host.supported()) {$('enter').textContent='Start VR op je headset';$('enter').disabled=false;status.textContent='Klaar. Zet je headset op en kies Start VR.';}
  else {$('enter').textContent='Open in Quest Browser';status.textContent=isSecureContext?'Deze browser heeft geen VR-headset. Open dezelfde link in Quest Browser.':'VR vereist HTTPS (of localhost). Open de beveiligde demolink in Quest Browser.';}
  if('serviceWorker' in navigator && isSecureContext) {
    navigator.serviceWorker.register('./sw.js').then(async()=>{
      await navigator.serviceWorker.ready;
      if(mode==='landing')status.textContent+=' Offline bestanden opgeslagen.';
    }).catch(()=>{});
  }
} catch(e) {errorMessage=e.stack??e.message;status.textContent=`Laden mislukt: ${e.message}. Herlaad de pagina.`;console.error(e);}
