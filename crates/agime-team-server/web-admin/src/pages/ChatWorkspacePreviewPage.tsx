import { useMemo, useRef, useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { ArrowLeft, ExternalLink } from "lucide-react";
import { SharedPreviewContent } from "../components/documents/DocumentPreview";
import { Button } from "../components/ui/button";
import { chatApi } from "../api/chat";
import { normalizePreviewMimeType } from "../utils/filePreview";

function normalizeName(path: string, label: string | null): string {
  const trimmedLabel = label?.trim();
  if (trimmedLabel) {
    return trimmedLabel;
  }
  const parts = path.split("/").filter(Boolean);
  return parts[parts.length - 1] || path;
}

function isWorkspaceVisualisationHtml(path: string, contentType: string | null): boolean {
  const normalizedPath = path.replace(/\\/g, "/").toLowerCase();
  const normalizedType = (contentType || "").toLowerCase();
  return (
    normalizedType.startsWith("text/html") &&
    normalizedPath.startsWith("artifacts/visualisations/") &&
    normalizedPath.endsWith(".html")
  );
}

function WorkspaceVisualisationFrame({
  src,
  title,
}: {
  src: string;
  title: string;
}) {
  const iframeRef = useRef<HTMLIFrameElement | null>(null);
  const [height, setHeight] = useState<number | null>(null);

  const syncHeight = () => {
    const doc = iframeRef.current?.contentDocument;
    if (!doc) {
      return;
    }
    const body = doc.body;
    const element = doc.documentElement;
    const measured = Math.max(
      body?.scrollHeight || 0,
      body?.offsetHeight || 0,
      element?.scrollHeight || 0,
      element?.offsetHeight || 0,
    );
    if (measured > 0) {
      setHeight(Math.max(measured + 8, 520));
    }
  };

  const handleLoad = () => {
    syncHeight();
    window.setTimeout(syncHeight, 100);
    window.setTimeout(syncHeight, 500);
  };

  return (
    <iframe
      ref={iframeRef}
      src={src}
      title={title}
      sandbox="allow-scripts allow-same-origin allow-downloads"
      scrolling="no"
      onLoad={handleLoad}
      style={height ? { height } : undefined}
      className="min-h-[70vh] w-full border-0 bg-white"
    />
  );
}

export function ChatWorkspacePreviewPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const sessionId = searchParams.get("session")?.trim() || "";
  const filePath = searchParams.get("path")?.trim() || "";
  const label = searchParams.get("label");
  const contentType = searchParams.get("contentType");

  const fileName = useMemo(
    () => normalizeName(filePath, label),
    [filePath, label],
  );
  const contentUrl = useMemo(() => {
    if (!sessionId || !filePath) {
      return "";
    }
    return chatApi.getSessionWorkspaceFileContentUrl(sessionId, filePath);
  }, [filePath, sessionId]);
  const rawPreviewUrl = useMemo(() => {
    if (!sessionId || !filePath) {
      return "";
    }
    return chatApi.getSessionWorkspacePreviewUrl(sessionId, filePath);
  }, [filePath, sessionId]);
  const downloadUrl = contentUrl;
  const effectiveContentType = normalizePreviewMimeType(
    contentType,
    `${fileName || ""} ${filePath}`.trim(),
  );
  const renderTrustedVisualisation = isWorkspaceVisualisationHtml(
    filePath,
    effectiveContentType,
  );

  if (!sessionId || !filePath) {
    return (
      <div className="min-h-screen bg-background px-6 py-10">
        <div className="mx-auto max-w-3xl rounded-[24px] border border-border/60 bg-card px-6 py-8 shadow-sm">
          <h1 className="text-lg font-semibold text-foreground">
            {t("chat.workspacePreviewInvalid", "无效的工作区预览链接")}
          </h1>
          <p className="mt-2 text-sm text-muted-foreground">
            {t(
              "chat.workspacePreviewInvalidDesc",
              "缺少会话或文件路径参数，无法加载预览。",
            )}
          </p>
          <Button
            className="mt-5"
            variant="outline"
            onClick={() => navigate(-1)}
          >
            <ArrowLeft className="mr-2 h-4 w-4" />
            {t("common.back", "返回")}
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-[hsl(var(--background))]">
      <div className="mx-auto flex min-h-screen max-w-7xl flex-col px-4 py-4 sm:px-6 sm:py-6">
        <div className="mb-4 flex flex-wrap items-center justify-between gap-3 rounded-[24px] border border-border/60 bg-card/95 px-4 py-3 shadow-sm backdrop-blur">
          <div className="min-w-0">
            <div className="truncate text-base font-semibold text-foreground">
              {fileName}
            </div>
            <div className="mt-1 truncate text-xs text-muted-foreground">
              {filePath}
              {effectiveContentType ? ` · ${effectiveContentType}` : ""}
            </div>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <Button variant="outline" onClick={() => navigate(-1)}>
              <ArrowLeft className="mr-2 h-4 w-4" />
              {t("common.back", "返回")}
            </Button>
            <Button variant="outline" asChild>
              <a href={rawPreviewUrl} target="_blank" rel="noreferrer">
                <ExternalLink className="mr-2 h-4 w-4" />
                {t("chat.openRawPreview", "打开原始文件")}
              </a>
            </Button>
            <Button asChild>
              <a href={downloadUrl} target="_blank" rel="noreferrer" download={fileName}>
                {t("common.download", "下载")}
              </a>
            </Button>
          </div>
        </div>

        <div className="min-h-0 flex-1 overflow-hidden rounded-[28px] border border-border/60 bg-card shadow-sm">
          {renderTrustedVisualisation ? (
            <WorkspaceVisualisationFrame
              src={contentUrl}
              title={fileName}
            />
          ) : (
            <SharedPreviewContent
              document={{
                name: fileName,
                mime_type: effectiveContentType,
                file_size: 0,
              }}
              contentUrl={contentUrl}
              onDownload={() => window.open(downloadUrl, "_blank", "noopener,noreferrer")}
            />
          )}
        </div>
      </div>
    </div>
  );
}
