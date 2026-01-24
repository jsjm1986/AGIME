import { useState, useEffect } from 'react';
import { Link } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { Button } from '../components/ui/button';
import { Input } from '../components/ui/input';
import { Card, CardHeader, CardTitle, CardContent } from '../components/ui/card';
import { LanguageSwitcher } from '../components/LanguageSwitcher';
import { apiClient, ApiKey } from '../api/client';

export function ApiKeysPage() {
  const { t } = useTranslation();
  const [keys, setKeys] = useState<ApiKey[]>([]);
  const [loading, setLoading] = useState(true);
  const [newKeyName, setNewKeyName] = useState('');
  const [creating, setCreating] = useState(false);
  const [newKey, setNewKey] = useState<string | null>(null);

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
    if (!confirm(t('apiKeys.revokeConfirm'))) return;
    try {
      await apiClient.revokeApiKey(keyId);
      loadKeys();
    } catch (err) {
      console.error('Failed to revoke key:', err);
    }
  };

  return (
    <div className="min-h-screen p-4 md:p-8">
      <div className="max-w-4xl mx-auto space-y-6">
        <div className="flex justify-between items-center">
          <h1 className="text-2xl font-bold">{t('apiKeys.title')}</h1>
          <div className="flex items-center gap-2">
            <LanguageSwitcher />
            <Link to="/dashboard">
              <Button variant="outline">{t('apiKeys.backToDashboard')}</Button>
            </Link>
          </div>
        </div>

        {newKey && (
          <Card className="border-green-500">
            <CardHeader>
              <CardTitle className="text-green-600">{t('apiKeys.newKeyCreated')}</CardTitle>
            </CardHeader>
            <CardContent className="space-y-2">
              <div className="p-3 bg-[hsl(var(--muted))] rounded-md font-mono text-sm break-all">
                {newKey}
              </div>
              <p className="text-sm text-[hsl(var(--muted-foreground))]">
                {t('apiKeys.saveKeyWarning')}
              </p>
              <Button variant="outline" onClick={() => setNewKey(null)}>
                {t('common.dismiss')}
              </Button>
            </CardContent>
          </Card>
        )}

        <Card>
          <CardHeader>
            <CardTitle>{t('apiKeys.createNewKey')}</CardTitle>
          </CardHeader>
          <CardContent className="flex gap-2">
            <Input
              placeholder={t('apiKeys.keyNamePlaceholder')}
              value={newKeyName}
              onChange={(e) => setNewKeyName(e.target.value)}
            />
            <Button onClick={handleCreate} disabled={creating}>
              {creating ? t('apiKeys.creating') : t('apiKeys.create')}
            </Button>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>{t('apiKeys.yourKeys')}</CardTitle>
          </CardHeader>
          <CardContent>
            {loading ? (
              <p>{t('common.loading')}</p>
            ) : keys.length === 0 ? (
              <p className="text-[hsl(var(--muted-foreground))]">{t('apiKeys.noKeys')}</p>
            ) : (
              <div className="space-y-3">
                {keys.map((key) => (
                  <div key={key.id} className="flex justify-between items-center p-3 border rounded-md">
                    <div>
                      <p className="font-medium">{key.name || t('apiKeys.unnamedKey')}</p>
                      <p className="text-sm text-[hsl(var(--muted-foreground))]">
                        {t('apiKeys.prefix')}: {key.key_prefix}...
                      </p>
                      <p className="text-xs text-[hsl(var(--muted-foreground))]">
                        {t('common.created')}: {new Date(key.created_at).toLocaleDateString()}
                      </p>
                    </div>
                    <Button variant="destructive" size="sm" onClick={() => handleRevoke(key.id)}>
                      {t('apiKeys.revoke')}
                    </Button>
                  </div>
                ))}
              </div>
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
