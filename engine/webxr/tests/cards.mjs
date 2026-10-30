// Longer, actual-gameplay check: let the bots eliminate the human, then use
// the real XR card UI. No injected offers or privileged debug mutations.
import { chromium } from 'playwright';
import { build } from 'esbuild';
const browser=await chromium.launch({channel:process.env.PLAYWRIGHT_CHANNEL??'chrome',headless:true,args:[process.platform==='darwin'?'--use-angle=metal':'--use-angle=swiftshader']});
try {
 const context=await browser.newContext({viewport:{width:1440,height:900}});
 const emu=await build({entryPoints:[new URL('./emulator.mjs',import.meta.url).pathname],bundle:true,format:'iife',write:false});
 await context.addInitScript({content:emu.outputFiles[0].text});
 const page=await context.newPage();page.setDefaultTimeout(45000);
 await page.goto(process.env.QUEST_DEMO_URL??'http://localhost:8787');await page.waitForFunction(()=>warpDemo?.ready);
 await page.click('#enter');await page.waitForFunction(()=>warpDemo.views===2);
 await page.evaluate(()=>xrDevice.controllers.right.updateButtonValue('trigger',1));
 await page.waitForTimeout(150);await page.evaluate(()=>xrDevice.controllers.right.updateButtonValue('trigger',0));
 await page.waitForFunction(()=>warpDemo.state.players[0].offer?.length>0,{},{timeout:45000});
 await page.screenshot({path:new URL('../../../build/quest-evidence/vr-card-choice-stereo.png',import.meta.url).pathname});
 const before=await page.evaluate(()=>warpDemo.state.players[0].cards);
 await page.evaluate(()=>xrDevice.controllers.right.updateButtonValue('trigger',1));
 await page.waitForFunction(n=>warpDemo.state.players[0].cards===n+1,before);
 console.log('PASS: bots eliminated human; in-world upgrade selection changed the real weapon');
} finally {await browser.close();}
