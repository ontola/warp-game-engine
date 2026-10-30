import { chromium } from 'playwright';
import { build } from 'esbuild';
import { mkdir, writeFile } from 'node:fs/promises';
import assert from 'node:assert/strict';
const root=new URL('../../../',import.meta.url).pathname;
const out=new URL('../../../build/quest-evidence/',import.meta.url).pathname;
await mkdir(out,{recursive:true});
const url=process.env.QUEST_DEMO_URL??'http://localhost:8787';
const browser=await chromium.launch({channel:process.env.PLAYWRIGHT_CHANNEL??'chrome',headless:true,args:[process.platform==='darwin'?'--use-angle=metal':'--use-angle=swiftshader','--enable-webgl']});
const errors=[], requests=[];
const deadline=setTimeout(()=>{console.error('Browser test deadline');process.exit(1);},120000);
const context=await browser.newContext({viewport:{width:1440,height:1000}});
function watch(page) {
  page.setDefaultTimeout(15000);
  page.on('pageerror',e=>errors.push(e.message));
  page.on('console',m=>{if(m.type()==='error')errors.push(m.text());});
  page.on('response',r=>{if(r.status()>=400)console.error('HTTP',r.status(),r.url());});
  page.on('requestfailed',r=>requests.push(`${r.url()}: ${r.failure()?.errorText}`));
}
try {
  const page=await context.newPage();watch(page);
  await page.goto(url);await page.waitForFunction(()=>window.warpDemo?.ready);
  await page.screenshot({path:out+'landing-desktop.png'});
  assert.ok((await page.evaluate(()=>warpDemo.arenaInstances))>1000);
  console.log('desktop start');await page.click('#desktop');await page.waitForFunction(()=>warpDemo.ticks>20);
  console.log('desktop fire');await page.mouse.down();await page.waitForTimeout(150);await page.mouse.up();
  await page.waitForFunction(()=>warpDemo.state.players[0].ammo<5);
  await page.keyboard.down('KeyW');await page.waitForTimeout(500);await page.keyboard.up('KeyW');
  await page.screenshot({path:out+'desktop-gameplay.png'});
  await page.keyboard.press('Escape');await page.waitForFunction(()=>warpDemo.paused);
  const frozen=await page.evaluate(()=>warpDemo.ticks);await page.waitForTimeout(200);assert.equal(await page.evaluate(()=>warpDemo.ticks),frozen);
  await page.click('#back');
  await page.waitForFunction(()=>navigator.serviceWorker.controller);
  await context.setOffline(true);await page.reload();await page.waitForFunction(()=>warpDemo?.ready);
  assert.equal(await page.evaluate(()=>warpDemo.ready),true);
  await context.setOffline(false);
  const desktopReport=await page.evaluate(()=>warpDemo);
  await context.close();
  const phone=await browser.newContext({viewport:{width:390,height:844},isMobile:true,deviceScaleFactor:1,userAgent:'Mozilla/5.0 (Linux; Android 13) AppleWebKit/537.36 Chrome/140.0.0.0 Mobile Safari/537.36'});
  const mobile=await phone.newPage();watch(mobile);await mobile.goto(url);await mobile.waitForFunction(()=>warpDemo?.ready);
  await mobile.screenshot({path:out+'landing-phone.png',fullPage:true});await phone.close();
  const emulation=await build({entryPoints:[new URL('./emulator.mjs',import.meta.url).pathname],bundle:true,format:'iife',write:false});
  const vrContext=await browser.newContext({viewport:{width:1440,height:900}});
  await vrContext.addInitScript({content:emulation.outputFiles[0].text});
  const vr=await vrContext.newPage();watch(vr);await vr.goto(url);await vr.waitForFunction(()=>warpDemo?.ready);
  await vr.click('#enter');await vr.waitForFunction(()=>warpDemo.views===2&&warpDemo.panelVisible);
  await vr.screenshot({path:out+'vr-start-stereo.png'});
  const button=async(id,value)=>{await vr.evaluate(([id,value])=>xrDevice.controllers.right.updateButtonValue(id,value),[id,value]);await vr.waitForTimeout(100);};
  await button('trigger',1);await button('trigger',0);
  await vr.waitForFunction(()=>!warpDemo.paused&&warpDemo.ticks>10);
  const initialYaw=await vr.evaluate(()=>warpDemo.rigYaw);
  await vr.evaluate(()=>xrDevice.controllers.right.updateAxes('thumbstick',1,0));
  await vr.waitForTimeout(350);
  const turned=await vr.evaluate(()=>warpDemo.rigYaw);assert.ok(Math.abs(turned-initialYaw+Math.PI/6)<.001);
  await vr.waitForTimeout(250);assert.equal(await vr.evaluate(()=>warpDemo.rigYaw),turned);
  await vr.evaluate(()=>xrDevice.controllers.right.updateAxes('thumbstick',0,0));
  await vr.evaluate(()=>xrDevice.controllers.left.updateAxes('thumbstick',0,-1));
  const before=await vr.evaluate(()=>warpDemo.state.players[0].pos);
  await vr.waitForTimeout(500);
  const after=await vr.evaluate(()=>warpDemo.state.players[0].pos);assert.ok(Math.hypot(after[0]-before[0],after[2]-before[2])>.1);
  await vr.evaluate(()=>xrDevice.controllers.left.updateAxes('thumbstick',0,0));
  await button('trigger',1);await vr.waitForFunction(()=>warpDemo.state.players[0].ammo<5);await button('trigger',0);
  await vr.screenshot({path:out+'vr-gameplay-stereo.png'});
  // A tracked trigger must release immediately when its source disappears.
  await vr.evaluate(()=>{xrDevice.controllers.right.connected=false;});
  await vr.waitForFunction(()=>!warpDemo.handVisible);
  const shots=await vr.evaluate(()=>warpDemo.localShots);
  await vr.waitForTimeout(250);assert.equal(await vr.evaluate(()=>warpDemo.localShots),shots);
  await vr.evaluate(()=>{xrDevice.controllers.right.connected=true;});
  await vr.waitForFunction(()=>warpDemo.handVisible);
  // System menu / headset visibility loss pauses, with no wall-time catch-up.
  await vr.evaluate(()=>xrDevice.updateVisibilityState('visible-blurred'));
  await vr.waitForFunction(()=>warpDemo.paused);
  const blurredTick=await vr.evaluate(()=>warpDemo.ticks);
  await vr.waitForTimeout(250);assert.equal(await vr.evaluate(()=>warpDemo.ticks),blurredTick);
  await vr.evaluate(()=>xrDevice.updateVisibilityState('visible'));
  await button('trigger',1);await button('trigger',0);
  await vr.waitForFunction(()=>!warpDemo.paused);
  await button('b-button',1);await button('b-button',0);
  await vr.waitForFunction(()=>warpDemo.paused&&warpDemo.panelVisible);
  const cameraBefore=await vr.evaluate(()=>warpDemo.cameraPosition);
  await vr.evaluate(()=>{xrDevice.position.x+=.1;});await vr.waitForTimeout(150);
  const cameraAfter=await vr.evaluate(()=>warpDemo.cameraPosition);
  assert.ok(Math.abs(Math.hypot(cameraAfter[0]-cameraBefore[0],cameraAfter[2]-cameraBefore[2])-.1)<.015,'paused menus retain physical 6DoF translation');
  const vrFrozen=await vr.evaluate(()=>warpDemo.ticks);await vr.waitForTimeout(250);assert.equal(await vr.evaluate(()=>warpDemo.ticks),vrFrozen);
  const report={desktop:desktopReport,vr:await vr.evaluate(()=>warpDemo),errors,requests,scope:'Chrome on macOS + Meta IWER Quest 2 emulator, not physical headset'};
  // Exit through the real in-world menu, then enter again.
  for(let i=0;i<2;i++) {
    await vr.evaluate(()=>xrDevice.controllers.left.updateAxes('thumbstick',1,0));await vr.waitForTimeout(80);
    await vr.evaluate(()=>xrDevice.controllers.left.updateAxes('thumbstick',0,0));await vr.waitForTimeout(80);
  }
  await button('trigger',1);await button('trigger',0);
  await vr.waitForFunction(()=>warpDemo.mode==='landing');
  await vr.click('#enter');await vr.waitForFunction(()=>warpDemo.views===2&&warpDemo.panelVisible);
  await writeFile(out+'validation.json' ,JSON.stringify(report,null,2));
  assert.deepEqual(errors,[]);assert.deepEqual(requests,[]);
  console.log(JSON.stringify({result:'PASS',views:report.vr.views,triangles:report.vr.triangles,drawCalls:report.vr.drawCalls,artifacts:out}));
} finally {clearTimeout(deadline);await browser.close();}
