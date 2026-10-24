/**
 * 增强截图前端纯逻辑单元测试：直接导入业务源码（Node 类型剥离），不复制实现。
 * 覆盖选区几何（八手柄、键盘微调、边界与翻转）、标注历史、马赛克取色、取色格式与贴图视图。
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  RESIZE_HANDLES,
  clampRectInside,
  computeFrameScale,
  containsPoint,
  cssToFrame,
  frameRectToCss,
  intersectRect,
  isDrag,
  nudgeRect,
  placeToolbar,
  rectFromPoints,
  resizeWithHandle,
  translateRect,
} from '../../src/utils/screenshot/geometry.ts';
import {
  EditHistory,
  isMeaningful,
  mosaicBlockSize,
  nextNumberValue,
  toFrameSize,
} from '../../src/utils/screenshot/annotations.ts';
import { pixelateBlocks } from '../../src/utils/screenshot/mosaic.ts';
import { readPixel, toHex, toRgbText } from '../../src/utils/screenshot/color.ts';
import {
  COLLAPSED_SIZE,
  MAX_ZOOM,
  MIN_ZOOM,
  formatPercent,
  pinWindowSize,
  stepOpacity,
  stepZoom,
  wheelDelta,
} from '../../src/utils/screenshot/pinView.ts';

const FRAME = { x: 0, y: 0, width: 200, height: 100 };

test('drag points normalize in any direction and selections serialize like SelectionBox', () => {
  const r = rectFromPoints({ x: 50.4, y: 80 }, { x: 10, y: 20.6 });
  assert.deepEqual(r, { x: 10, y: 21, width: 40, height: 59 });
  // 与后端 app_core::SelectionBox 字段完全一致，可直接作为导出参数序列化。
  assert.deepEqual(Object.keys(JSON.parse(JSON.stringify(r))).sort(), ['height', 'width', 'x', 'y']);
});

test('intersect and clamp keep selections inside the frozen frame', () => {
  assert.deepEqual(intersectRect({ x: -20, y: 90, width: 50, height: 50 }, FRAME), { x: 0, y: 90, width: 30, height: 10 });
  assert.equal(intersectRect({ x: 300, y: 0, width: 5, height: 5 }, FRAME), null);
  assert.deepEqual(clampRectInside({ x: 190, y: -5, width: 30, height: 300 }, FRAME), { x: 170, y: 0, width: 30, height: 100 });
  assert.deepEqual(translateRect({ x: 5, y: 5, width: 10, height: 10 }, -50, 500, FRAME), { x: 0, y: 90, width: 10, height: 10 });
});

test('eight handles resize the expected edges', () => {
  const base = { x: 50, y: 30, width: 40, height: 20 };
  const expected = {
    top_left: { x: 45, y: 25, width: 45, height: 25 },
    top: { x: 50, y: 25, width: 40, height: 25 },
    top_right: { x: 50, y: 25, width: 45, height: 25 },
    right: { x: 50, y: 30, width: 45, height: 20 },
    bottom_right: { x: 50, y: 30, width: 45, height: 25 },
    bottom: { x: 50, y: 30, width: 40, height: 25 },
    bottom_left: { x: 45, y: 30, width: 45, height: 25 },
    left: { x: 45, y: 30, width: 45, height: 20 },
  };
  assert.equal(RESIZE_HANDLES.length, 8);
  for (const handle of RESIZE_HANDLES) {
    const outward = {
      dx: handle.includes('left') ? -5 : handle.includes('right') ? 5 : 0,
      dy: handle.startsWith('top') ? -5 : handle.startsWith('bottom') ? 5 : 0,
    };
    const { rect } = resizeWithHandle(base, handle, outward.dx, outward.dy, FRAME);
    assert.deepEqual(rect, expected[handle], handle);
  }
});

test('dragging a handle past the opposite edge flips it and stays at least 1px', () => {
  const base = { x: 50, y: 30, width: 40, height: 20 };
  const flipped = resizeWithHandle(base, 'right', -60, 0, FRAME);
  assert.deepEqual(flipped.rect, { x: 30, y: 30, width: 20, height: 20 });
  assert.equal(flipped.handle, 'left');
  const corner = resizeWithHandle(base, 'bottom_right', -40, -20, FRAME);
  assert.equal(corner.rect.width, 1);
  assert.equal(corner.rect.height, 1);
  const outside = resizeWithHandle(base, 'top_left', -500, -500, FRAME);
  assert.deepEqual(outside.rect, { x: 0, y: 0, width: 90, height: 50 });
});

test('keyboard nudges move 1px and Shift resizes width/height within bounds', () => {
  const sel = { x: 10, y: 10, width: 20, height: 20 };
  assert.deepEqual(nudgeRect(sel, 'ArrowLeft', false, FRAME), { x: 9, y: 10, width: 20, height: 20 });
  assert.deepEqual(nudgeRect(sel, 'ArrowDown', false, FRAME), { x: 10, y: 11, width: 20, height: 20 });
  assert.deepEqual(nudgeRect(sel, 'ArrowRight', true, FRAME), { x: 10, y: 10, width: 21, height: 20 });
  assert.deepEqual(nudgeRect(sel, 'ArrowUp', true, FRAME), { x: 10, y: 10, width: 20, height: 19 });
  assert.deepEqual(nudgeRect({ x: 0, y: 0, width: 1, height: 1 }, 'ArrowLeft', true, FRAME).width, 1);
  assert.deepEqual(nudgeRect({ x: 0, y: 0, width: 5, height: 5 }, 'ArrowLeft', false, FRAME).x, 0);
  assert.deepEqual(nudgeRect({ x: 190, y: 0, width: 10, height: 5 }, 'ArrowRight', true, FRAME).width, 10);
});

test('css to frame mapping handles Retina and non-uniform window sizes', () => {
  const retina = computeFrameScale(3024, 1964, 1512, 982);
  assert.deepEqual(retina, { sx: 2, sy: 2 });
  assert.deepEqual(cssToFrame({ x: 100.3, y: 50.6 }, retina), { x: 201, y: 101 });
  assert.deepEqual(frameRectToCss({ x: 20, y: 40, width: 200, height: 100 }, retina), { x: 10, y: 20, width: 100, height: 50 });
  const odd = computeFrameScale(1000, 600, 500, 200);
  assert.deepEqual(odd, { sx: 2, sy: 3 });
  assert.deepEqual(computeFrameScale(10, 10, 0, 0), { sx: 1, sy: 1 });
});

test('hit test, drag threshold and toolbar placement', () => {
  const r = { x: 0, y: 0, width: 10, height: 10 };
  assert.ok(containsPoint(r, { x: 9, y: 9 }));
  assert.ok(!containsPoint(r, { x: 10, y: 5 }));
  assert.ok(!isDrag({ x: 0, y: 0 }, { x: 3, y: 3 }, 4));
  assert.ok(isDrag({ x: 0, y: 0 }, { x: 0, y: 4 }, 4));
  const viewport = { width: 800, height: 600 };
  const bar = { width: 300, height: 40 };
  assert.deepEqual(placeToolbar({ x: 100, y: 100, width: 400, height: 100 }, bar, viewport), { x: 200, y: 208 });
  assert.deepEqual(placeToolbar({ x: 100, y: 500, width: 400, height: 90 }, bar, viewport), { x: 200, y: 452 });
  const full = placeToolbar({ x: 0, y: 0, width: 800, height: 600 }, bar, viewport);
  assert.deepEqual(full, { x: 500, y: 552 });
  assert.equal(placeToolbar({ x: 0, y: 0, width: 50, height: 50 }, bar, viewport).x, 0);
});

test('edit history supports undo, redo, update, remove and drops redo after new edits', () => {
  const h = new EditHistory(3);
  const a = { id: 'a', kind: 'rect', color: '#ff0000', size: 2, rect: { x: 0, y: 0, width: 10, height: 10 } };
  const b = { id: 'b', kind: 'number', color: '#ff0000', size: 2, at: { x: 1, y: 1 }, value: 1 };
  assert.equal(h.canUndo, false);
  h.add(a);
  h.add(b);
  assert.equal(h.items.length, 2);
  assert.ok(h.undo());
  assert.deepEqual(h.items.map((i) => i.id), ['a']);
  assert.ok(h.canRedo);
  assert.ok(h.redo());
  assert.deepEqual(h.items.map((i) => i.id), ['a', 'b']);
  assert.ok(h.update({ ...a, color: '#00ff00' }));
  assert.equal(h.items[0].color, '#00ff00');
  assert.equal(h.update({ ...a, id: 'missing' }), false);
  h.undo();
  h.add({ ...b, id: 'c', value: 2 });
  assert.equal(h.canRedo, false, 'new edit clears redo stack');
  assert.ok(h.remove('a'));
  assert.equal(h.remove('a'), false);
  h.clear();
  assert.equal(h.items.length, 0);
  // 历史上限 3：最多回退三步。
  let steps = 0;
  while (h.undo()) steps++;
  assert.equal(steps, 3);
});

test('counter numbers follow existing values, tiny drags are discarded, sizes scale with DPR', () => {
  const n = (value) => ({ id: `n${value}`, kind: 'number', color: '#fff', size: 4, at: { x: 0, y: 0 }, value });
  assert.equal(nextNumberValue([]), 1);
  assert.equal(nextNumberValue([n(1), n(4), n(2)]), 5);
  assert.ok(!isMeaningful({ id: 'r', kind: 'rect', color: '#fff', size: 2, rect: { x: 0, y: 0, width: 2, height: 30 } }));
  assert.ok(isMeaningful({ id: 'r', kind: 'mosaic', color: '#000', size: 2, block: 6, rect: { x: 0, y: 0, width: 8, height: 8 } }));
  assert.ok(!isMeaningful({ id: 't', kind: 'text', color: '#fff', size: 2, at: { x: 0, y: 0 }, text: '   ' }));
  assert.ok(isMeaningful({ id: 'arrow', kind: 'arrow', color: '#fff', size: 2, from: { x: 0, y: 0 }, to: { x: 3, y: 4 } }));
  assert.equal(toFrameSize(4, 2), 8);
  assert.equal(toFrameSize(0.1, 1), 1);
  assert.equal(mosaicBlockSize(1), 6);
  assert.equal(mosaicBlockSize(8), 24);
});

test('mosaic blocks are true averages of the frozen frame and clip at frame edges', () => {
  // 4x2 帧：左 2x2 为 (0,0,0) 与 (100,100,100) 交替，右 2x2 全为 (200,0,0)。
  const w = 4;
  const h = 2;
  const frame = new Uint8ClampedArray(w * h * 4);
  const set = (x, y, r, g, b) => frame.set([r, g, b, 255], (y * w + x) * 4);
  set(0, 0, 0, 0, 0);
  set(1, 0, 100, 100, 100);
  set(0, 1, 100, 100, 100);
  set(1, 1, 0, 0, 0);
  for (const [x, y] of [[2, 0], [3, 0], [2, 1], [3, 1]]) set(x, y, 200, 0, 0);
  const blocks = pixelateBlocks(frame, w, h, { x: 0, y: 0, width: 4, height: 2 }, 2);
  assert.equal(blocks.cols, 2);
  assert.equal(blocks.rows, 1);
  assert.deepEqual(Array.from(blocks.rgba), [50, 50, 50, 255, 200, 0, 0, 255]);
  const clipped = pixelateBlocks(frame, w, h, { x: 3, y: 1, width: 10, height: 10 }, 4);
  assert.equal(clipped.cols, 1);
  assert.equal(clipped.rows, 1);
  assert.deepEqual(Array.from(clipped.rgba), [200, 0, 0, 255]);
  const empty = pixelateBlocks(frame, w, h, { x: 10, y: 10, width: 5, height: 5 }, 2);
  assert.equal(empty.rgba.length, 0);
});

test('color picker reads raw pixels and formats HEX/RGB like the backend expects', () => {
  const px = new Uint8ClampedArray([1, 2, 3, 255, 250, 128, 15, 255]);
  const c = readPixel(px, 2, 1, 1, 0);
  assert.deepEqual(c, { r: 250, g: 128, b: 15, a: 255 });
  assert.equal(toHex(c), '#FA800F');
  assert.equal(toRgbText(c), 'rgb(250, 128, 15)');
  assert.equal(readPixel(px, 2, 1, 2, 0), null);
  assert.equal(readPixel(px, 2, 1, -1, 0), null);
  assert.equal(readPixel(px, 2, 1, 0.5, 0), null);
});

test('pin zoom stays within 10%-500%, opacity within 10%-100%, collapse uses mini block', () => {
  let z = 1;
  for (let i = 0; i < 100; i++) z = stepZoom(z, -1);
  assert.equal(z, MAX_ZOOM);
  for (let i = 0; i < 100; i++) z = stepZoom(z, 1);
  assert.equal(z, MIN_ZOOM);
  assert.equal(stepZoom(1, 0), 1);
  assert.equal(stepZoom(1, -120), 1.1);
  let o = 1;
  o = stepOpacity(o, 1);
  assert.equal(o, 0.9);
  for (let i = 0; i < 20; i++) o = stepOpacity(o, 1);
  assert.equal(o, 0.1);
  assert.equal(stepOpacity(1, -1), 1);
  // macOS 按住 Shift 时滚轮增量出现在 deltaX。
  assert.equal(wheelDelta({ deltaX: -3, deltaY: 0 }), -3);
  assert.equal(wheelDelta({ deltaX: 5, deltaY: 2 }), 2);
  assert.deepEqual(pinWindowSize(200, 100, 1.5, false), { width: 300, height: 150 });
  assert.deepEqual(pinWindowSize(100, 50, 0.1, false), { width: 16, height: 16 });
  assert.deepEqual(pinWindowSize(200, 100, 3, true), { width: COLLAPSED_SIZE, height: COLLAPSED_SIZE });
  assert.equal(formatPercent(0.1), '10%');
  assert.equal(formatPercent(5), '500%');
});
