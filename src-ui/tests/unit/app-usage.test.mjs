/**
 * 应用使用时长面板日期范围单元测试：最近 7/30 天必须以操作时的本地日期为准，
 * 不能沿用组件挂载时的日期，否则跨午夜后范围会少一天。
 */
import test from 'node:test';
import assert from 'node:assert/strict';
import { addDays, lastDaysRange, toDateInput } from '../../src/utils/appUsage.ts';

test('recent range is recalculated from the current local date across midnight', () => {
  const beforeMidnight = lastDaysRange(7, new Date('2026-10-28T23:59:00'));
  const afterMidnight = lastDaysRange(7, new Date('2026-10-29T00:01:00'));

  assert.deepEqual(beforeMidnight, { startDate: '2026-10-22', endDate: '2026-10-28' });
  assert.deepEqual(afterMidnight, { startDate: '2026-10-23', endDate: '2026-10-29' });
  assert.notEqual(beforeMidnight.endDate, afterMidnight.endDate);
});

test('seven and thirty day ranges are inclusive of the current day', () => {
  const now = new Date('2026-10-29T00:01:00');
  assert.deepEqual(lastDaysRange(7, now), { startDate: '2026-10-23', endDate: '2026-10-29' });
  assert.deepEqual(lastDaysRange(30, now), { startDate: '2026-09-30', endDate: '2026-10-29' });
  assert.deepEqual(lastDaysRange(1, now), { startDate: '2026-10-29', endDate: '2026-10-29' });
});

test('date helpers keep leap days and year boundaries local', () => {
  const leap = new Date('2028-03-01T08:00:00');
  assert.equal(addDays(toDateInput(leap), -1), '2028-02-29');
  assert.equal(addDays('2026-01-01', -1), '2025-12-31');
  assert.equal(addDays('2026-12-31', 1), '2027-01-01');
});
