import test from 'node:test';
import assert from 'node:assert/strict';

// 测试纯领域逻辑与算法（HistoryManager 与 LaserPointerManager）
class InkHistoryManager {
  constructor(initialStrokes = []) {
    this.undoStack = [...initialStrokes];
    this.redoStack = [];
  }

  get strokes() {
    return this.undoStack;
  }

  push(stroke) {
    this.undoStack.push(stroke);
    this.redoStack = [];
  }

  undo() {
    const stroke = this.undoStack.pop();
    if (stroke) {
      this.redoStack.push(stroke);
      return stroke;
    }
    return null;
  }

  redo() {
    const stroke = this.redoStack.pop();
    if (stroke) {
      this.undoStack.push(stroke);
      return stroke;
    }
    return null;
  }

  clear() {
    this.undoStack = [];
    this.redoStack = [];
  }

  canUndo() {
    return this.undoStack.length > 0;
  }

  canRedo() {
    return this.redoStack.length > 0;
  }
}

class LaserPointerManager {
  constructor() {
    this.points = [];
    this.lifetimeMs = 2000;
  }

  addPoint(x, y, timestamp = Date.now()) {
    this.points.push({ x, y, pressure: 1.0, timestamp });
  }

  prune(now = Date.now()) {
    const cutoff = now - this.lifetimeMs;
    this.points = this.points.filter((p) => p.timestamp >= cutoff);
  }

  clear() {
    this.points = [];
  }

  get activePoints() {
    return this.points;
  }

  hasPoints() {
    return this.points.length > 0;
  }
}

test('InkHistoryManager maintains undo/redo stack properly', () => {
  const manager = new InkHistoryManager();
  assert.equal(manager.canUndo(), false);
  assert.equal(manager.canRedo(), false);
  assert.equal(manager.undo(), null);
  assert.equal(manager.redo(), null);

  const stroke1 = { id: 's1', tool: 'pen', color: '#ff0000', width: 4, points: [{ x: 10, y: 10, pressure: 0.5, timestamp: 100 }] };
  const stroke2 = { id: 's2', tool: 'highlighter', color: '#00ff00', width: 8, points: [{ x: 20, y: 20, pressure: 0.8, timestamp: 200 }] };

  manager.push(stroke1);
  assert.equal(manager.canUndo(), true);
  assert.equal(manager.canRedo(), false);
  assert.equal(manager.strokes.length, 1);

  manager.push(stroke2);
  assert.equal(manager.strokes.length, 2);

  // 撤销 stroke2
  const undone = manager.undo();
  assert.equal(undone.id, 's2');
  assert.equal(manager.strokes.length, 1);
  assert.equal(manager.canRedo(), true);

  // 重做 stroke2
  const redone = manager.redo();
  assert.equal(redone.id, 's2');
  assert.equal(manager.strokes.length, 2);
  assert.equal(manager.canRedo(), false);

  // 撤销后再 push 新笔画应清空重做栈
  manager.undo();
  const stroke3 = { id: 's3', tool: 'rect', color: '#0000ff', width: 2, points: [{ x: 0, y: 0 }, { x: 50, y: 50 }] };
  manager.push(stroke3);
  assert.equal(manager.strokes.length, 2);
  assert.equal(manager.canRedo(), false);
  assert.equal(manager.redo(), null);

  // 清空
  manager.clear();
  assert.equal(manager.strokes.length, 0);
  assert.equal(manager.canUndo(), false);
  assert.equal(manager.canRedo(), false);
});

test('LaserPointerManager automatically prunes points older than 2000ms', () => {
  const laser = new LaserPointerManager();
  const baseTime = 10000;

  laser.addPoint(100, 100, baseTime);
  laser.addPoint(120, 120, baseTime + 500);
  laser.addPoint(150, 150, baseTime + 1500);

  assert.equal(laser.hasPoints(), true);
  assert.equal(laser.activePoints.length, 3);

  // 1000ms 过去（未超过 2000ms 寿命）
  laser.prune(baseTime + 1000);
  assert.equal(laser.activePoints.length, 3);

  // 2100ms 过去（第 1 个点超过 2000ms，被裁切）
  laser.prune(baseTime + 2100);
  assert.equal(laser.activePoints.length, 2);
  assert.equal(laser.activePoints[0].x, 120);

  // 4000ms 过去（所有点均已超过 2000ms 消隐时间）
  laser.prune(baseTime + 4000);
  assert.equal(laser.hasPoints(), false);
  assert.equal(laser.activePoints.length, 0);
});
