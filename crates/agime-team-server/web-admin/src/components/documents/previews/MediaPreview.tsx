interface MediaPreviewProps {
  contentUrl: string;
  mimeType: string;
}

export function MediaPreview({ contentUrl, mimeType }: MediaPreviewProps) {
  const isVideo = mimeType.startsWith('video/');

  if (isVideo) {
    return (
      <div className="flex items-center justify-center h-full p-4">
        <video controls className="max-w-full max-h-full rounded">
          <source src={contentUrl} type={mimeType} />
        </video>
      </div>
    );
  }

  return (
    <div className="flex items-center justify-center h-full p-4">
      <audio controls className="w-full max-w-md">
        <source src={contentUrl} type={mimeType} />
      </audio>
    </div>
  );
}
