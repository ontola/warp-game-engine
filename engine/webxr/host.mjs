import * as THREE from 'three';
import { deadzone } from './input.mjs';
export { THREE };

/** Reusable standalone WebXR shell. Games supply a scene and fixed-step sim.
 * Three's WebXRManager uses the runtime's predicted stereo poses/projections,
 * manages XR swapchain rendering and handles session shutdown. No streaming.
 */
export class XRHost {
  constructor(canvas, callbacks) {
    this.callbacks = callbacks;
    this.renderer = new THREE.WebGLRenderer({canvas, antialias: true, alpha: false, powerPreference: 'high-performance'});
    this.renderer.setPixelRatio(Math.min(devicePixelRatio, 1.5));
    this.renderer.outputColorSpace = THREE.SRGBColorSpace;
    this.renderer.xr.enabled = true;
    this.renderer.xr.setReferenceSpaceType('local-floor');
    this.renderer.xr.setFramebufferScaleFactor(0.8);
    this.renderer.xr.setFoveation(1);
    this.scene = new THREE.Scene();
    this.camera = new THREE.PerspectiveCamera(75, 1, 0.06, 350);
    this.rig = new THREE.Group();
    this.rig.add(this.camera);
    this.scene.add(this.rig);
    this.session = null;
    this.lastViews = 0;
    this.frameCount = 0;
    this.resize = () => {
      if (!this.session) {
        this.renderer.setSize(innerWidth, innerHeight, false);
        this.camera.aspect = innerWidth / innerHeight;
        this.camera.updateProjectionMatrix();
      }
    };
    window.addEventListener('resize', this.resize);
    this.resize();
    this.renderer.setAnimationLoop((time, frame) => {
      this.frameCount++;
      const input = frame ? this.input(frame) : null;
      callbacks.frame(time, input);
      this.renderer.render(this.scene, this.camera);
    });
  }
  async supported() {
    return !!(isSecureContext && navigator.xr && await navigator.xr.isSessionSupported('immersive-vr'));
  }
  async enter() {
    if (this.session) return;
    // Call directly inside the user's click handler; no await before requestSession.
    const session = await navigator.xr.requestSession('immersive-vr', {requiredFeatures: ['local-floor']});
    this.session = session;
    session.addEventListener('end', () => {
      this.session = null; this.lastViews = 0;
      this.resize(); this.callbacks.session(false);
    }, {once: true});
    session.addEventListener('visibilitychange', () => this.callbacks.visibility(session.visibilityState));
    try {
      await this.renderer.xr.setSession(session);
      this.renderer.xr.getReferenceSpace().addEventListener('reset', () => this.callbacks.recenter());
      if (session.supportedFrameRates?.includes(72)) await session.updateTargetFrameRate(72).catch(() => {});
      this.callbacks.session(true);
    } catch (error) {
      await session.end();
      throw error;
    }
  }
  input(frame) {
    const space = this.renderer.xr.getReferenceSpace();
    const pose = frame.getViewerPose(space);
    this.lastViews = pose?.views.length ?? 0;
    if (!pose || this.session.visibilityState !== 'visible') return {tracked: false};
    const transform = t => ({position: [t.position.x,t.position.y,t.position.z], quaternion: [t.orientation.x,t.orientation.y,t.orientation.z,t.orientation.w]});
    const hands = {};
    for (const source of this.session.inputSources) {
      if (source.handedness !== 'left' && source.handedness !== 'right') continue;
      const aim = frame.getPose(source.targetRaySpace, space);
      const grip = source.gripSpace && frame.getPose(source.gripSpace, space);
      const pad = source.gamepad;
      if (!aim || !pad || pad.mapping !== 'xr-standard') continue;
      hands[source.handedness] = {
        aim: transform(aim.transform), grip: grip ? transform(grip.transform) : null,
        // xr-standard: trigger, squeeze, touchpad, stick, A/X, B/Y.
        trigger: (pad.buttons[0]?.value ?? 0) > 0.75,
        squeeze: (pad.buttons[1]?.value ?? 0) > 0.75,
        primary: !!pad.buttons[4]?.pressed, secondary: !!pad.buttons[5]?.pressed,
        stick: [deadzone(pad.axes[2] ?? 0), deadzone(pad.axes[3] ?? 0)],
        pulse: (strength, ms) => pad.hapticActuators?.[0]?.pulse(strength, ms)?.catch(() => {}),
      };
    }
    return {tracked: true, head: transform(pose.transform), hands};
  }
  end() { return this.session?.end(); }
}
