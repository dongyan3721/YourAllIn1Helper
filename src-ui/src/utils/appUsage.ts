/**
 * 应用使用时长面板的日期范围逻辑（纯函数）。
 * 最近 N 天的结束日期必须在用户操作时按当前本地日期计算，避免跨午夜后沿用挂载时的日期。
 */

export interface UsageDateRange {
  startDate: string;
  endDate: string;
}

export function toDateInput(date: Date): string {
  const year = date.getFullYear();
  const month = `${date.getMonth() + 1}`.padStart(2, '0');
  const day = `${date.getDate()}`.padStart(2, '0');
  return `${year}-${month}-${day}`;
}

export function addDays(date: string, days: number): string {
  const parsed = new Date(`${date}T12:00:00`);
  parsed.setDate(parsed.getDate() + days);
  return toDateInput(parsed);
}

/** 返回含 `now` 当天在内的最近 N 个本地日期。 */
export function lastDaysRange(days: number, now: Date = new Date()): UsageDateRange {
  const span = Number.isFinite(days) ? Math.max(1, Math.floor(days)) : 1;
  const endDate = toDateInput(now);
  return {
    startDate: addDays(endDate, -(span - 1)),
    endDate,
  };
}
