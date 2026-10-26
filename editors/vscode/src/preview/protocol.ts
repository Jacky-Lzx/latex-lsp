export type PdfLocation = {
  page: number;
  x: number;
  y: number;
  h?: number;
  v?: number;
  width?: number;
  height?: number;
};

export type PreviewSource = { uri: string; column?: number };
export type InverseSearch = { pdfUri: string; revision: number; page: number; x: number; y: number };

export function isPdfPoint(value: PdfLocation): boolean {
  return Number.isSafeInteger(value?.page) && value.page > 0
    && Number.isFinite(value.x) && value.x >= 0 && Number.isFinite(value.y) && value.y >= 0;
}
