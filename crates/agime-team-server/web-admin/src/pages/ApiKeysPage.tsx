import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../components/ui/button';
import { Input } from '../components/ui/input';
import { Card, CardHeader, CardTitle, CardContent } from '../components/ui/card';
import { AppShell } from '../components/layout/AppShell';
import { PageHeader } from '../components/layout/PageHeader';
import { Skeleton } from '../components/ui/skeleton';
import { ConfirmDialog } from '../components/ui/confirm-dialog';
import { apiClient, ApiKey } from '../api/client';
import { formatDate } from '../utils/format';

export function ApiKeysPage() {
  const { t } = useTranslation();
  const [keys, setKeys] = useState<ApiKey[]>([]);
  const [loading, setLoading] = useState(true);
  const [newKeyName, setNewKeyName] = useState('');
  const [creating, setCreating] = useState(false);
  const [newKey, setNewKey] = useState<string | null>(null);
  const [revokeTarget, setRevokeTarget] = useState<string | null>(null);

  const loadKeys = async () => {
    try {
      const res = await apiClient.getApiKeys();
      setKeys(res.keys);
    } catch (err) {
      console.error('Failed to load keys:', err);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadKeys();
  }, []);

  const handleCreate = async () => {
    setCreating(true);
    try {
      const res = await apiClient.createApiKey(newKeyName || undefined);
      setNewKey(res.key.api_key);
      setNewKeyName('');
      loadKeys();
    } catch (err) {
      console.error('Failed to create key:', err);
    } finally {
      setCreating(false);
    }
  };

  const handleRevoke = async (keyId: string) => {
    setRevokeTarget(keyId);
  };

  const confirmRevoke = async () => {
    if (!revokeTarget) return;
    try {
      await apiClient.revokeApiKey(revokeTarget);
      loadKeys();
    } catch (err) {
      console.error('Failed to revoke key:', err);
    } finally {
      setRevokeTarget(null);
    }
  };

  return (
    <AppShell>
      <PageHeader title={t('apiKeys.title')} />

      {newKey && (
        <Card className="ui-section-panel mb-6 border-[hsl(var(--status-success-text))/0.18] bg-[hsl(var(--status-success-bg))/0.48]">
          <CardHeader>
            <div className="ui-kicker">{t('apiKeys.newKeyCreated')}</div>
            <CardTitle className="text-[hsl(var(--status-success-text))]">{t('apiKeys.newKeyCreated')}</CardTitle>
          </CardHeader>
          <CardContent className="space-y-2">
            <div className="ui-copy-block break-all p-3 font-mono text-sm">
              {newKey}
            </div>
            <p className="ui-secondary-text text-sm">
              {t('apiKeys.saveKeyWarning')}
            </p>
            <Button variant="outline" onClick={() => setNewKey(null)}>
              {t('common.dismiss')}
            </Button>
          </CardContent>
        </Card>
      )}

      <Card className="ui-section-panel mb-6">
        <CardHeader>
          <CardTitle className="ui-heading text-[22px]">{t('apiKeys.createNewKey')}</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-3 sm:flex-row">
          <Input
            placeholder={t('apiKeys.keyNamePlaceholder')}
            value={newKeyName}
            onChange={(e) => setNewKeyName(e.target.value)}
            className="sm:flex-1"
          />
          <Button onClick={handleCreate} disabled={creating}>
            {creating ? t('apiKeys.creating') : t('apiKeys.create')}
          </Button>
        </CardContent>
      </Card>

      <Card className="ui-section-panel">
        <CardHeader>
          <CardTitle className="ui-heading text-[22px]">{t('apiKeys.yourKeys')}</CardTitle>
        </CardHeader>
        <CardContent>
          {loading ? (
            <div className="space-y-3">
              <Skeleton className="h-16 w-full" />
              <Skeleton className="h-16 w-full" />
            </div>
          ) : keys.length === 0 ? (
            <div className="ui-empty-panel p-5 text-sm ui-secondary-text">{t('apiKeys.noKeys')}</div>
          ) : (
            <div className="space-y-3">
              {keys.map((key) => (
                <div key={key.id} className="ui-subtle-panel flex flex-col gap-3 p-4 sm:flex-row sm:items-center sm:justify-between">
                  <div className="space-y-1">
                    <p className="text-sm font-semibold text-[hsl(var(--foreground))]">{key.name || t('apiKeys.unnamedKey')}</p>
                    <p className="ui-secondary-text text-sm">
                      {t('apiKeys.prefix')}: {key.key_prefix}...
                    </p>
                    <p className="ui-tertiary-text text-xs">
                      {t('common.created')}: {formatDate(key.created_at)}
                    </p>
                  </div>
                  <Button className="sm:self-start" variant="destructive" size="sm" onClick={() => handleRevoke(key.id)}>
                    {t('apiKeys.revoke')}
                  </Button>
                </div>
              ))}
            </div>
          )}
        </CardContent>
      </Card>

      <ConfirmDialog
        open={!!revokeTarget}
        onOpenChange={(open) => { if (!open) setRevokeTarget(null); }}
        title={t('apiKeys.revoke')}
        description={t('apiKeys.revokeConfirm')}
        variant="destructive"
        onConfirm={confirmRevoke}
      />
    </AppShell>
  );
}
