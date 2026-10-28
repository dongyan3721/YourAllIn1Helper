import test from 'node:test';
import assert from 'node:assert/strict';
import { startWindowDrag } from '../../src/utils/windowDrag.ts';

test('window drag handles title descendants but preserves controls and non-left clicks', async () => {
  let drags = 0;
  let prevented = 0;
  globalThis.window = { __TAURI__: { window: { getCurrentWindow: () => ({ startDragging: async () => { drags++; } }) } } };
  const event = (button, interactive) => ({
    button, target: { closest: (selector) => {
      assert.match(selector, /input/); assert.match(selector, /button/); assert.match(selector, /select/);
      return interactive ? {} : null;
    } }, preventDefault: () => { prevented++; },
  });
  try {
    await startWindowDrag(event(0, false));
    assert.equal(drags, 1); assert.equal(prevented, 1);
    await startWindowDrag(event(0, true));
    await startWindowDrag(event(1, false));
    await startWindowDrag(event(2, false));
    assert.equal(drags, 1); assert.equal(prevented, 1);
    globalThis.window = {};
    await startWindowDrag(event(0, false));
    assert.equal(prevented, 1, 'browser preview retains normal mouse behavior');
  } finally { delete globalThis.window; }
});
