// SyncTeX uses physical PDF points from the unrotated page's top left.
// PDF.js uses PDF user space, with a bottom-left origin, before applying its
// viewport transform (zoom, page rotation, crop origin, and device layout).
export function toPdfPoint(viewport, { x, y }) {
  const [left, , , top] = viewport.viewBox;
  return [left + x / viewport.userUnit, top - y / viewport.userUnit];
}

export function toViewportPoint(viewport, point) {
  return viewport.convertToViewportPoint(...toPdfPoint(viewport, point));
}

export function fromViewportPoint(viewport, x, y) {
  const [pdfX, pdfY] = viewport.convertToPdfPoint(x, y);
  const [left, , , top] = viewport.viewBox;
  return { x: (pdfX - left) * viewport.userUnit, y: (top - pdfY) * viewport.userUnit };
}

export function highlightRectangle(viewport, location) {
  const box = [location.h, location.v, location.width, location.height].every(Number.isFinite)
    && location.width > 0 && location.height > 0;
  const first = toViewportPoint(viewport, box ? { x: location.h, y: location.v - location.height } : location);
  const last = box ? toViewportPoint(viewport, { x: location.h + location.width, y: location.v }) : first;
  return { left: Math.min(first[0], last[0]) - 3, top: Math.min(first[1], last[1]) - 3,
    width: Math.max(10, Math.abs(last[0] - first[0]) + 6), height: Math.max(10, Math.abs(last[1] - first[1]) + 6) };
}
