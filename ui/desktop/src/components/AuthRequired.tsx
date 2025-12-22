/**
 * Authentication Required Component
 *
 * Shown when users access the web UI without a valid authentication secret.
 * Provides instructions on how to get the access URL from the desktop app.
 */

import React, { useState } from 'react';
import { setTunnelSecret } from '../platform';
import { Button } from './ui/button';
import { Input } from './ui/input';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from './ui/card';
import { QrCode, Smartphone, Key, AlertCircle, RefreshCw } from 'lucide-react';

interface AuthRequiredProps {
  error?: string | null;
}

export default function AuthRequired({ error }: AuthRequiredProps) {
  const [manualSecret, setManualSecret] = useState('');
  const [showManualInput, setShowManualInput] = useState(false);
  const [isSubmitting, setIsSubmitting] = useState(false);

  const handleManualSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!manualSecret.trim()) return;

    setIsSubmitting(true);

    try {
      // Store the secret
      setTunnelSecret(manualSecret.trim());

      // Reload the page to trigger authentication
      window.location.reload();
    } catch (err) {
      console.error('Manual authentication failed:', err);
      setIsSubmitting(false);
    }
  };

  return (
    <div className="min-h-screen bg-background-default flex items-center justify-center p-4">
      <div className="w-full max-w-md space-y-6">
        {/* Logo */}
        <div className="text-center">
          <div className="w-16 h-16 mx-auto mb-4 rounded-2xl bg-gradient-to-br from-block-teal to-block-orange flex items-center justify-center">
            <span className="text-3xl font-bold text-white">A</span>
          </div>
          <h1 className="text-2xl font-semibold text-text-default">AGIME Web</h1>
          <p className="text-text-muted mt-1">访问需要认证 / Authentication Required</p>
        </div>

        {/* Error message if any */}
        {error && (
          <Card className="border-red-500/30 bg-red-500/5">
            <CardContent className="p-4 flex items-start gap-3">
              <AlertCircle className="w-5 h-5 text-red-500 flex-shrink-0 mt-0.5" />
              <div>
                <p className="text-sm font-medium text-red-500">连接错误 / Connection Error</p>
                <p className="text-xs text-red-400 mt-1">{error}</p>
              </div>
            </CardContent>
          </Card>
        )}

        {/* Instructions */}
        <Card>
          <CardHeader className="pb-4">
            <CardTitle className="text-lg">如何访问 / How to Access</CardTitle>
            <CardDescription>
              从桌面端 AGIME 获取访问链接
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            {/* Step 1 */}
            <div className="flex gap-3">
              <div className="w-8 h-8 rounded-full bg-block-teal/20 flex items-center justify-center flex-shrink-0">
                <span className="text-sm font-medium text-block-teal">1</span>
              </div>
              <div>
                <p className="text-sm font-medium text-text-default">打开桌面端 AGIME</p>
                <p className="text-xs text-text-muted">Open AGIME desktop app</p>
              </div>
            </div>

            {/* Step 2 */}
            <div className="flex gap-3">
              <div className="w-8 h-8 rounded-full bg-block-teal/20 flex items-center justify-center flex-shrink-0">
                <span className="text-sm font-medium text-block-teal">2</span>
              </div>
              <div>
                <p className="text-sm font-medium text-text-default">进入设置 → 应用 → 远程访问</p>
                <p className="text-xs text-text-muted">Go to Settings → App → Remote Access</p>
              </div>
            </div>

            {/* Step 3 */}
            <div className="flex gap-3">
              <div className="w-8 h-8 rounded-full bg-block-teal/20 flex items-center justify-center flex-shrink-0">
                <span className="text-sm font-medium text-block-teal">3</span>
              </div>
              <div>
                <p className="text-sm font-medium text-text-default">启动隧道并扫描二维码</p>
                <p className="text-xs text-text-muted">Start tunnel and scan QR code</p>
              </div>
            </div>

            {/* Visual guide */}
            <div className="mt-4 p-4 bg-background-muted rounded-lg flex items-center justify-center gap-6">
              <div className="flex flex-col items-center">
                <Smartphone className="w-8 h-8 text-text-muted" />
                <span className="text-xs text-text-muted mt-1">桌面端</span>
              </div>
              <div className="flex items-center text-text-muted">
                <div className="w-8 border-t border-dashed border-text-muted"></div>
                <QrCode className="w-6 h-6 mx-2 text-block-teal" />
                <div className="w-8 border-t border-dashed border-text-muted"></div>
              </div>
              <div className="flex flex-col items-center">
                <div className="w-8 h-8 rounded bg-text-muted/20 flex items-center justify-center">
                  <span className="text-xs">🌐</span>
                </div>
                <span className="text-xs text-text-muted mt-1">网页端</span>
              </div>
            </div>
          </CardContent>
        </Card>

        {/* Manual input option */}
        <Card>
          <CardHeader className="pb-2">
            <button
              onClick={() => setShowManualInput(!showManualInput)}
              className="flex items-center gap-2 text-sm text-text-muted hover:text-text-default transition-colors"
            >
              <Key className="w-4 h-4" />
              手动输入密钥 / Manual Secret Input
              <span className={`transition-transform ${showManualInput ? 'rotate-180' : ''}`}>▼</span>
            </button>
          </CardHeader>

          {showManualInput && (
            <CardContent>
              <form onSubmit={handleManualSubmit} className="space-y-3">
                <Input
                  type="password"
                  placeholder="输入访问密钥 / Enter secret key"
                  value={manualSecret}
                  onChange={(e) => setManualSecret(e.target.value)}
                  className="font-mono text-sm"
                />
                <Button
                  type="submit"
                  className="w-full"
                  disabled={!manualSecret.trim() || isSubmitting}
                >
                  {isSubmitting ? (
                    <>
                      <RefreshCw className="w-4 h-4 mr-2 animate-spin" />
                      验证中...
                    </>
                  ) : (
                    '连接 / Connect'
                  )}
                </Button>
              </form>
            </CardContent>
          )}
        </Card>

        {/* Footer */}
        <p className="text-center text-xs text-text-muted">
          AGIME Web - 通过隧道安全访问您的 AI 助手
          <br />
          Securely access your AI assistant via tunnel
        </p>
      </div>
    </div>
  );
}
