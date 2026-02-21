import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { missionApi, MissionArtifact } from '../../api/mission';

interface ArtifactListProps {
  missionId: string;
}

export function ArtifactList({ missionId }: ArtifactListProps) {
  const { t } = useTranslation();
  const [artifacts, setArtifacts] = useState<MissionArtifact[]>([]);
  const [loading, setLoading] = useState(true);
  const [expanded, setExpanded] = useState<string | null>(null);

  const formatSize = (size: number) => {
    if (size < 1024) return `${size} B`;
    if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`;
    return `${(size / (1024 * 1024)).toFixed(1)} MB`;
  };

  const copyText = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      // no-op
    }
  };

  const downloadContent = (name: string, content: string) => {
    const blob = new Blob([content], { type: 'text/plain;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = name || 'artifact.txt';
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  };

  useEffect(() => {
    let cancelled = false;
    missionApi.listArtifacts(missionId).then(items => {
      if (!cancelled) {
        setArtifacts(items || []);
        setLoading(false);
      }
    }).catch(() => {
      if (!cancelled) setLoading(false);
    });
    return () => { cancelled = true; };
  }, [missionId]);

  if (loading) {
    return <p className="text-sm text-muted-foreground p-3">Loading...</p>;
  }

  if (artifacts.length === 0) {
    return (
      <p className="text-sm text-muted-foreground p-3 text-center">
        {t('mission.noArtifacts', 'No artifacts')}
      </p>
    );
  }

  return (
    <div className="space-y-2 p-3">
      {artifacts.map(a => (
        <div key={a.artifact_id} className="border rounded-md">
          <button
            onClick={() => setExpanded(
              expanded === a.artifact_id ? null : a.artifact_id
            )}
            className="w-full flex items-center justify-between p-2 text-sm hover:bg-accent rounded-md"
          >
            <div className="flex items-center gap-2">
              <span className="text-xs px-1.5 py-0.5 rounded bg-muted font-mono">
                {a.artifact_type}
              </span>
              <span className="font-medium truncate">{a.name}</span>
            </div>
            <div className="text-xs text-muted-foreground flex items-center gap-2">
              <span>{formatSize(a.size)}</span>
              <span>·</span>
              <span>Step {a.step_index + 1}</span>
            </div>
          </button>

          {expanded === a.artifact_id && (
            <div className="border-t p-2">
              {a.file_path && (
                <div className="text-xs text-muted-foreground mb-2 break-all">
                  <span className="font-medium mr-1">{t('mission.filePath', 'Path')}:</span>
                  {a.file_path}
                </div>
              )}

              <div className="flex items-center gap-2 mb-2">
                {a.content && (
                  <>
                    <button
                      onClick={() => copyText(a.content || '')}
                      className="text-xs px-2 py-1 rounded border border-border hover:bg-accent transition-colors"
                    >
                      {t('mission.copyContent', 'Copy content')}
                    </button>
                    <button
                      onClick={() => downloadContent(a.name, a.content || '')}
                      className="text-xs px-2 py-1 rounded border border-border hover:bg-accent transition-colors"
                    >
                      {t('mission.downloadText', 'Download text')}
                    </button>
                  </>
                )}
                {a.file_path && (
                  <button
                    onClick={() => copyText(a.file_path || '')}
                    className="text-xs px-2 py-1 rounded border border-border hover:bg-accent transition-colors"
                  >
                    {t('mission.copyPath', 'Copy path')}
                  </button>
                )}
              </div>

              {a.content ? (
                <pre className="text-xs font-mono bg-muted rounded p-2 overflow-x-auto max-h-64">
                  {a.content}
                </pre>
              ) : (
                <p className="text-xs text-muted-foreground">
                  {t('mission.noInlineContent', 'No inline content for this artifact')}
                </p>
              )}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}
