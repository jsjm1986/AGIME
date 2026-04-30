const EXTENSION_MIME_TYPES: Record<string, string> = {
  avif: "image/avif",
  bmp: "image/bmp",
  c: "text/x-c",
  cpp: "text/x-c++",
  css: "text/css",
  csv: "text/csv",
  doc: "application/msword",
  docm: "application/vnd.ms-word.document.macroenabled.12",
  docx: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
  gif: "image/gif",
  go: "text/x-go",
  h: "text/x-c",
  hpp: "text/x-c++",
  htm: "text/html",
  html: "text/html",
  ico: "image/x-icon",
  java: "text/x-java",
  jpeg: "image/jpeg",
  jpg: "image/jpeg",
  js: "text/javascript",
  jsx: "text/javascript",
  json: "application/json",
  log: "text/plain",
  m4a: "audio/mp4",
  markdown: "text/markdown",
  md: "text/markdown",
  mov: "video/quicktime",
  mp3: "audio/mpeg",
  mp4: "video/mp4",
  ogg: "audio/ogg",
  pdf: "application/pdf",
  png: "image/png",
  pot: "application/vnd.ms-powerpoint",
  potx: "application/vnd.openxmlformats-officedocument.presentationml.template",
  pps: "application/vnd.ms-powerpoint",
  ppsx: "application/vnd.openxmlformats-officedocument.presentationml.slideshow",
  ppt: "application/vnd.ms-powerpoint",
  pptm: "application/vnd.ms-powerpoint.presentation.macroenabled.12",
  pptx: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
  py: "text/x-python",
  rs: "text/x-rust",
  rtf: "application/rtf",
  scss: "text/x-scss",
  svg: "image/svg+xml",
  ts: "text/x-typescript",
  tsx: "text/x-typescript",
  txt: "text/plain",
  wav: "audio/wav",
  webm: "video/webm",
  webp: "image/webp",
  xls: "application/vnd.ms-excel",
  xlsb: "application/vnd.ms-excel.sheet.binary.macroenabled.12",
  xlsm: "application/vnd.ms-excel.sheet.macroenabled.12",
  xlsx: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
  xml: "application/xml",
  yaml: "application/x-yaml",
  yml: "application/x-yaml",
};

const GENERIC_BINARY_MIME_TYPES = new Set([
  "application/octet-stream",
  "binary/octet-stream",
]);

export function inferMimeTypeFromName(name: string): string {
  const clean = name.split(/[?#]/, 1)[0]?.trim().toLowerCase() || "";
  const match = /\.([a-z0-9]+)$/.exec(clean);
  if (!match) {
    return "";
  }
  return EXTENSION_MIME_TYPES[match[1]] || "";
}

export function normalizePreviewMimeType(
  mimeType: string | null | undefined,
  fileNameOrPath: string,
): string {
  const normalized = (mimeType || "").split(";", 1)[0].trim().toLowerCase();
  if (normalized && !GENERIC_BINARY_MIME_TYPES.has(normalized)) {
    return normalized;
  }
  return inferMimeTypeFromName(fileNameOrPath) || normalized;
}

export function isBrowserPreviewableFile(
  fileNameOrPath: string,
  mimeType?: string | null,
): boolean {
  const mime = normalizePreviewMimeType(mimeType, fileNameOrPath);
  if (
    mime.startsWith("text/") ||
    mime.startsWith("image/") ||
    mime.startsWith("audio/") ||
    mime.startsWith("video/")
  ) {
    return true;
  }
  return (
    mime === "application/json" ||
    mime === "application/pdf" ||
    mime === "application/msword" ||
    mime === "application/rtf" ||
    mime === "application/xml" ||
    mime === "application/x-yaml" ||
    mime === "application/vnd.ms-excel" ||
    mime === "application/vnd.ms-powerpoint" ||
    mime === "application/vnd.openxmlformats-officedocument.presentationml.presentation" ||
    mime === "application/vnd.openxmlformats-officedocument.presentationml.slideshow" ||
    mime === "application/vnd.openxmlformats-officedocument.presentationml.template" ||
    mime === "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" ||
    mime === "application/vnd.openxmlformats-officedocument.wordprocessingml.document" ||
    mime.startsWith("application/vnd.ms-excel.") ||
    mime.startsWith("application/vnd.ms-powerpoint.") ||
    mime.startsWith("application/vnd.ms-word.")
  );
}
