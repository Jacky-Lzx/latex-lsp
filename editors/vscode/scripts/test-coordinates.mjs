import assert from 'node:assert/strict';
import test from 'node:test';
import { getDocument } from 'pdfjs-dist/legacy/build/pdf.mjs';
import { fromViewportPoint, highlightRectangle, toViewportPoint } from '../media/preview/coordinates.mjs';

function fixture() {
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Count 1 /Kids [3 0 R] >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /CropBox [10 20 190 280] /UserUnit 2 /Contents 4 0 R >>',
    '<< /Length 0 >>\nstream\n\nendstream',
  ];
  let pdf = '%PDF-1.7\n';
  const offsets = objects.map((object, index) => {
    const offset = pdf.length;
    pdf += `${index + 1} 0 obj\n${object}\nendobj\n`;
    return offset;
  });
  const xref = pdf.length;
  pdf += `xref\n0 5\n0000000000 65535 f \n${offsets.map(offset => `${String(offset).padStart(10, '0')} 00000 n \n`).join('')}`;
  pdf += `trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return new Uint8Array(Buffer.from(pdf));
}

test('SyncTeX coordinates round trip through real PDF.js zoom, rotation, crop, and user units', async () => {
  const task = getDocument({ data: fixture() });
  try {
    const pdf = await task.promise;
    const page = await pdf.getPage(1);
    const point = { x: 40, y: 80 };
    // The crop is 180 x 260 PDF units, or 360 x 520 physical points.
    const expected = new Map([[0, [40, 80]], [90, [440, 40]], [180, [320, 440]], [270, [80, 320]]]);
    for (const scale of [.25, 1, 1.5, 4]) {
      for (const [rotation, coordinates] of expected) {
        const viewport = page.getViewport({ scale, rotation });
        const actual = toViewportPoint(viewport, point);
        actual.forEach((value, index) => assert.ok(Math.abs(value - coordinates[index] * scale) < 1e-8));
        const reverse = fromViewportPoint(viewport, ...actual);
        assert.ok(Math.abs(reverse.x - point.x) < 1e-8);
        assert.ok(Math.abs(reverse.y - point.y) < 1e-8);
        const box = highlightRectangle(viewport, { ...point, h: 40, v: 80, width: 100, height: 12 });
        assert.ok(actual[0] >= box.left && actual[0] <= box.left + box.width);
        assert.ok(actual[1] >= box.top && actual[1] <= box.top + box.height);
      }
    }
  } finally {
    await task.destroy();
  }
});
