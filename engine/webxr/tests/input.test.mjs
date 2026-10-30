import test from 'node:test';
import assert from 'node:assert/strict';
import { FixedStep, Edges, SnapTurn, deadzone } from '../input.mjs';
test('held stick turns once and rearms only after neutral', () => {
  const s = new SnapTurn();
  assert.equal(s.update(0.9), -Math.PI / 6);
  for(let i=0;i<120;i++) assert.equal(s.update(0.9),0);
  s.update(0); assert.equal(s.update(-0.9), Math.PI / 6);
});
test('button edge survives render frames until a tick and consumes once', () => {
  const e = new Edges(); e.poll({jump:true}); e.poll({jump:true});
  assert.equal(e.take('jump'),true); assert.equal(e.take('jump'),false);
  e.poll({jump:false}); e.poll({jump:true}); assert.equal(e.take('jump'),true);
  e.clear(); assert.equal(e.take('jump'),false);
});
test('60Hz simulation at 72, 90 and 120Hz; no resume backlog', () => {
  for (const hz of [72,90,120]) {
    const c = new FixedStep(); let ticks=0;
    for(let i=0;i<=hz*10;i++) c.advance(i*1000/hz,()=>ticks++);
    assert.ok(ticks>=599 && ticks<=600, `${hz}: ${ticks}`);
    c.reset(); c.advance(60000,()=>ticks++); assert.ok(ticks<=600);
    const before=ticks; c.advance(120000,()=>ticks++); assert.ok(ticks-before<=4);
  }
});
test('invalid axes and deadzone never create motion', () => {
  for (const x of [NaN, Infinity, 0, 0.1,-0.18]) assert.equal(deadzone(x),0);
  assert.equal(deadzone(1),1); assert.equal(deadzone(-1),-1);
});
