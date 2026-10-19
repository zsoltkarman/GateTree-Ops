export function visibleLauncherCount(widths, available, gap = 4, moreWidth = 30) {
  const total = widths.reduce((sum, width) => sum + width, 0) + Math.max(0, widths.length - 1) * gap;
  if (total <= available) return widths.length;
  let used = moreWidth;
  let count = 0;
  for (const width of widths) {
    if (used + gap + width > available) break;
    used += gap + width;
    count++;
  }
  return count;
}

export function moveLauncherItem(ids, source, target, after = false) {
  if (source === target || !ids.includes(source) || (target && !ids.includes(target))) return [...ids];
  const next = ids.filter((id) => id !== source);
  const index = target ? next.indexOf(target) + Number(after) : next.length;
  next.splice(index, 0, source);
  return next;
}
