import { XRDevice, metaQuest2 } from 'iwer';
const device=new XRDevice(metaQuest2, {stereoEnabled:true});
device.installRuntime({forceInstall:true});
device.position.set(0,1.6,0);
device.controllers.left.position.set(-.28,1.15,-.4);
device.controllers.right.position.set(.24,1.25,-.4);
window.xrDevice=device;
