import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from '../ui/dialog';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { Switch } from '../ui/switch';
import {
  Link2,
  Copy,
  Check,
  Download,
  FileText,
  FileJson,
  FileCode,
  Lock,
  LoaderCircle,
  ExternalLink,
  AlertCircle,
} from 'lucide-react';
import { toast } from 'react-toastify';
import { Session, Message } from '../../api';
import { createLocalShare, CreateShareResponse } from '../../sharedSessions';
import { isElectron } from '../../platform';
import { getApiUrl } from '../../config';
import { QRCodeSVG } from 'qrcode.react';

interface ShareSessionDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  session: Session;
}

type ExpirationOption = 1 | 7 | 30 | null;

interface TunnelStatus {
  state: string;
  url: string;
  hostname: string;
  secret: string;
}

export default function ShareSessionDialog({
  open,
  onOpenChange,
  session,
}: ShareSessionDialogProps) {
  const { t } = useTranslation('sessions');
  const [tunnelStatus, setTunnelStatus] = useState<TunnelStatus | null>(null);
  const [expiration, setExpiration] = useState<ExpirationOption>(7);
  const [enablePassword, setEnablePassword] = useState(false);
  const [password, setPassword] = useState('');
  const [isSharing, setIsSharing] = useState(false);
  const [shareResult, setShareResult] = useState<CreateShareResponse | null>(null);
  const [copied, setCopied] = useState<string | null>(null);

  // Check tunnel status
  useEffect(() => {
    if (open && isElectron) {
      window.electron.cloudflaredStatus().then((status) => {
        setTunnelStatus(status);
      });
    }
  }, [open]);

  // Reset state when dialog closes
  useEffect(() => {
    if (!open) {
      setShareResult(null);
      setPassword('');
      setEnablePassword(false);
      setExpiration(7);
      setCopied(null);
    }
  }, [open]);

  const isTunnelRunning = tunnelStatus?.state === 'running' && tunnelStatus?.url;

  // Generate share link
  const getShareUrl = () => {
    if (!shareResult || !tunnelStatus?.url) return '';
    const baseUrl = tunnelStatus.url.replace(/\/$/, '');
    return `${baseUrl}/web/#/shared/${shareResult.shareToken}`;
  };

  // Handle online share
  const handleCreateShare = async () => {
    setIsSharing(true);
    try {
      // Use getApiUrl to get the local API base URL
      const apiBaseUrl = getApiUrl('').replace(/\/$/, '');
      // Get secret key for authentication
      const secretKey = isElectron ? await window.electron.getSecretKey() : undefined;
      const result = await createLocalShare(
        apiBaseUrl,
        session.id,
        expiration,
        enablePassword ? password : undefined,
        secretKey
      );
      setShareResult(result);
    } catch (error) {
      console.error('Failed to create share:', error);
      toast.error(t('shareModal.createFailed'));
    } finally {
      setIsSharing(false);
    }
  };

  // Copy to clipboard
  const handleCopy = async (text: string, key: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(key);
      setTimeout(() => setCopied(null), 2000);
    } catch {
      toast.error(t('shareModal.copyFailed'));
    }
  };

  // Convert messages to Markdown
  const convertToMarkdown = (messages: Message[]): string => {
    let md = `# ${session.name}\n\n`;
    md += `**Working Directory:** ${session.working_dir}\n`;
    md += `**Messages:** ${session.message_count}\n`;
    if (session.total_tokens) {
      md += `**Tokens:** ${session.total_tokens.toLocaleString()}\n`;
    }
    md += '\n---\n\n';

    for (const message of messages) {
      const role = message.role === 'user' ? 'User' : 'Assistant';
      md += `## ${role}\n\n`;

      for (const content of message.content) {
        if (content.type === 'text') {
          md += `${content.text}\n\n`;
        } else if (content.type === 'toolRequest') {
          const toolCall = content.toolCall || {};
          const toolName = Object.keys(toolCall)[0] || 'unknown';
          md += `**Tool:** ${toolName}\n\`\`\`json\n${JSON.stringify(toolCall, null, 2)}\n\`\`\`\n\n`;
        } else if (content.type === 'toolResponse') {
          const result = content.toolResult || {};
          md += `**Tool Result:**\n\`\`\`\n${JSON.stringify(result, null, 2)}\n\`\`\`\n\n`;
        }
      }
      md += '---\n\n';
    }

    return md;
  };

  // Generate HTML export
  const generateHtml = (messages: Message[]): string => {
    const markdown = convertToMarkdown(messages);
    // Simple HTML wrapper with basic styling
    return `<!DOCTYPE html>
<html lang="zh-CN">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>${session.name}</title>
  <style>
    body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; max-width: 800px; margin: 0 auto; padding: 20px; line-height: 1.6; }
    h1 { color: #1a1a1a; border-bottom: 2px solid #0d9488; padding-bottom: 10px; }
    h2 { color: #374151; margin-top: 30px; }
    pre { background: #1a1a1a; color: #e5e7eb; padding: 16px; border-radius: 8px; overflow-x: auto; }
    code { font-family: 'Fira Code', monospace; }
    hr { border: none; border-top: 1px solid #e5e7eb; margin: 20px 0; }
    .meta { color: #6b7280; font-size: 14px; }
  </style>
</head>
<body>
  <pre>${markdown.replace(/</g, '&lt;').replace(/>/g, '&gt;')}</pre>
</body>
</html>`;
  };

  // Download file
  const downloadFile = (content: string, filename: string, type: string) => {
    const blob = new Blob([content], { type });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  };

  // Export handlers
  const handleExportHtml = () => {
    const messages = session.conversation || [];
    const html = generateHtml(messages);
    downloadFile(html, `${session.name || 'session'}.html`, 'text/html');
  };

  const handleExportMarkdown = () => {
    const messages = session.conversation || [];
    const md = convertToMarkdown(messages);
    downloadFile(md, `${session.name || 'session'}.md`, 'text/markdown');
  };

  const handleExportJson = () => {
    const json = JSON.stringify(session, null, 2);
    downloadFile(json, `${session.name || 'session'}.json`, 'application/json');
  };

  const handleCopyAsMarkdown = () => {
    const messages = session.conversation || [];
    const md = convertToMarkdown(messages);
    handleCopy(md, 'markdown');
  };

  const shareUrl = getShareUrl();

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg p-0">
        <DialogHeader className="px-6 py-5 border-b border-border-subtle">
          <DialogTitle className="text-base font-medium">
            {t('shareModal.title')}
          </DialogTitle>
        </DialogHeader>

        <div className="px-6 py-5 space-y-6">
          {/* Online Sharing Section - Only show when tunnel is running */}
          {isTunnelRunning && !shareResult && (
            <div className="space-y-4">
              <div className="flex items-center gap-2 text-sm font-medium text-text-default">
                <Link2 className="w-4 h-4 text-teal-500" />
                {t('shareModal.onlineShare')}
              </div>

              {/* Expiration Options */}
              <div className="space-y-2">
                <label className="text-sm text-text-muted">{t('shareModal.expiration')}</label>
                <div className="flex gap-2">
                  {([1, 7, 30, null] as ExpirationOption[]).map((days) => (
                    <button
                      key={days ?? 'never'}
                      onClick={() => setExpiration(days)}
                      className={`px-3 py-1.5 text-sm rounded-full border transition-colors ${
                        expiration === days
                          ? 'bg-teal-500 text-white border-teal-500'
                          : 'border-border-subtle text-text-muted hover:border-teal-500'
                      }`}
                    >
                      {days === null
                        ? t('shareModal.never')
                        : t('shareModal.days', { count: days })}
                    </button>
                  ))}
                </div>
              </div>

              {/* Password Protection */}
              <div className="space-y-2">
                <div className="flex items-center justify-between">
                  <label className="text-sm text-text-muted flex items-center gap-2">
                    <Lock className="w-4 h-4" />
                    {t('shareModal.passwordProtection')}
                  </label>
                  <Switch
                    checked={enablePassword}
                    onCheckedChange={setEnablePassword}
                    variant="mono"
                  />
                </div>
                {enablePassword && (
                  <Input
                    type="password"
                    placeholder={t('shareModal.passwordPlaceholder')}
                    value={password}
                    onChange={(e) => setPassword(e.target.value)}
                    className="mt-2"
                  />
                )}
              </div>

              {/* Create Share Button */}
              <Button
                onClick={handleCreateShare}
                disabled={isSharing || (enablePassword && !password)}
                className="w-full"
              >
                {isSharing ? (
                  <>
                    <LoaderCircle className="w-4 h-4 mr-2 animate-spin" />
                    {t('shareModal.creating')}
                  </>
                ) : (
                  t('shareModal.createLink')
                )}
              </Button>
            </div>
          )}

          {/* Share Result */}
          {shareResult && isTunnelRunning && (
            <div className="space-y-4">
              <div className="flex items-center gap-2 text-sm font-medium text-teal-500">
                <Check className="w-4 h-4" />
                {t('shareModal.linkCreated')}
              </div>

              {/* Share URL */}
              <div className="space-y-2">
                <div className="relative rounded-lg border border-border-subtle bg-background-muted p-3">
                  <code className="text-sm break-all pr-10">{shareUrl}</code>
                  <Button
                    size="sm"
                    variant="ghost"
                    className="absolute right-2 top-1/2 -translate-y-1/2"
                    onClick={() => handleCopy(shareUrl, 'url')}
                  >
                    {copied === 'url' ? (
                      <Check className="w-4 h-4 text-teal-500" />
                    ) : (
                      <Copy className="w-4 h-4" />
                    )}
                  </Button>
                </div>

                {/* QR Code */}
                <div className="flex justify-center p-4 bg-white rounded-lg">
                  <QRCodeSVG value={shareUrl} size={160} />
                </div>

                {shareResult.hasPassword && (
                  <p className="text-xs text-text-muted flex items-center gap-1">
                    <Lock className="w-3 h-3" />
                    {t('shareModal.passwordProtected')}
                  </p>
                )}

                {shareResult.expiresAt && (
                  <p className="text-xs text-text-muted">
                    {t('shareModal.expiresAt', {
                      date: new Date(shareResult.expiresAt).toLocaleDateString(),
                    })}
                  </p>
                )}
              </div>

              {/* Open in Browser */}
              <Button
                variant="outline"
                className="w-full"
                onClick={() => window.open(shareUrl, '_blank')}
              >
                <ExternalLink className="w-4 h-4 mr-2" />
                {t('shareModal.openInBrowser')}
              </Button>
            </div>
          )}

          {/* Tunnel Not Running Notice */}
          {!isTunnelRunning && isElectron && (
            <div className="rounded-lg border border-amber-200 dark:border-amber-800 bg-amber-50 dark:bg-amber-950 p-4">
              <div className="flex items-start gap-3">
                <AlertCircle className="w-5 h-5 text-amber-600 dark:text-amber-400 flex-shrink-0 mt-0.5" />
                <div>
                  <p className="text-sm font-medium text-amber-800 dark:text-amber-200">
                    {t('shareModal.tunnelRequired')}
                  </p>
                  <p className="text-xs text-amber-700 dark:text-amber-300 mt-1">
                    {t('shareModal.tunnelHint')}
                  </p>
                </div>
              </div>
            </div>
          )}

          {/* Divider */}
          <div className="border-t border-border-subtle" />

          {/* Export Section - Always visible */}
          <div className="space-y-4">
            <div className="flex items-center gap-2 text-sm font-medium text-text-default">
              <Download className="w-4 h-4 text-teal-500" />
              {t('shareModal.exportAsFile')}
            </div>

            <div className="grid grid-cols-3 gap-2">
              <Button variant="outline" onClick={handleExportHtml} className="flex-col h-auto py-3">
                <FileCode className="w-5 h-5 mb-1" />
                <span className="text-xs">HTML</span>
              </Button>
              <Button variant="outline" onClick={handleExportMarkdown} className="flex-col h-auto py-3">
                <FileText className="w-5 h-5 mb-1" />
                <span className="text-xs">Markdown</span>
              </Button>
              <Button variant="outline" onClick={handleExportJson} className="flex-col h-auto py-3">
                <FileJson className="w-5 h-5 mb-1" />
                <span className="text-xs">JSON</span>
              </Button>
            </div>

            {/* Copy as Markdown */}
            <Button variant="outline" onClick={handleCopyAsMarkdown} className="w-full">
              {copied === 'markdown' ? (
                <>
                  <Check className="w-4 h-4 mr-2 text-teal-500" />
                  {t('shareModal.copied')}
                </>
              ) : (
                <>
                  <Copy className="w-4 h-4 mr-2" />
                  {t('shareModal.copyAsMarkdown')}
                </>
              )}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
