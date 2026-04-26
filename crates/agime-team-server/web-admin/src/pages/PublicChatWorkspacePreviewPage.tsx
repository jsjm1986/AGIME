import { useMemo, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { ExternalLink } from "lucide-react";
import { SharedPreviewContent } from "../components/documents/DocumentPreview";
import { Button } from "../components/ui/button";

function resolvePublicUrls(shareId: string) {
  const base = `/api/team/agent/chat/public/workspace-shares/${encodeURIComponent(shareId)}`;
  return {
    downloadUrl: `${base}/download`,
    contentUrl: `${base}/content`,
  };
}

function guessNameFromShare(shareId: string, contentType: string) {
  const extMap: Record<string, string> = {
    "application/pdf": ".pdf",
    "application/json": ".json",
    "text/markdown": ".md",
    "text/plain": ".txt",
    "text/html": ".html",
    "text/csv": ".csv",
    "image/svg+xml": ".svg",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document":
      ".docx",
    "application/msword": ".doc",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet":
      ".xlsx",
    "application/vnd.ms-excel": ".xls",
    "application/vnd.openxmlformats-officedocument.presentationml.presentation":
      ".pptx",
    "application/vnd.ms-powerpoint": ".ppt",
  };
  const suffix = extMap[contentType] || "";
  return shareId ? `shared-file-${shareId}${suffix}` : `shared-file${suffix}`;
}

function PublicVisualisationFrame({
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

export function PublicChatWorkspacePreviewPage() {
  const { t } = useTranslation();
  const [searchParams] = useSearchParams();
  const shareId = searchParams.get("share")?.trim() || "";
  const contentType = searchParams.get("contentType")?.trim() || "";

  const urls = useMemo(() => resolvePublicUrls(shareId), [shareId]);

  if (!shareId) {
    return (
      <div className="min-h-screen bg-background px-6 py-10">
        <div className="mx-auto max-w-3xl rounded-[24px] border border-border/60 bg-card px-6 py-8 shadow-sm">
          <h1 className="text-lg font-semibold text-foreground">
            {t("chat.workspacePreviewInvalid", "无效的工作区预览链接")}
          </h1>
          <p className="mt-2 text-sm text-muted-foreground">
            {t(
              "chat.publicWorkspacePreviewInvalidDesc",
              "缺少分享标识，无法加载公开预览。",
            )}
          </p>
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
              {t("chat.sharedWorkspacePreview", "共享文件预览")}
            </div>
            <div className="mt-1 truncate text-xs text-muted-foreground">
              {shareId}
              {contentType ? ` · ${contentType}` : ""}
            </div>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <Button variant="outline" asChild>
              <a href={urls.contentUrl} target="_blank" rel="noreferrer">
                <ExternalLink className="mr-2 h-4 w-4" />
                {t("chat.openRawPreview", "打开原始文件")}
              </a>
            </Button>
            <Button asChild>
              <a href={urls.downloadUrl} target="_blank" rel="noreferrer">
                {t("common.download", "下载")}
              </a>
            </Button>
          </div>
        </div>

        <div className="min-h-0 flex-1 overflow-hidden rounded-[28px] border border-border/60 bg-card shadow-sm">
          {contentType.toLowerCase().startsWith("text/html") ? (
            <PublicVisualisationFrame
              src={urls.contentUrl}
              title={guessNameFromShare(shareId, contentType)}
            />
          ) : (
            <SharedPreviewContent
              document={{
                name: guessNameFromShare(shareId, contentType),
                mime_type: contentType,
                file_size: 0,
              }}
              contentUrl={urls.contentUrl}
              onDownload={() => window.open(urls.downloadUrl, "_blank", "noopener,noreferrer")}
            />
          )}
        </div>
      </div>
    </div>
  );
}
