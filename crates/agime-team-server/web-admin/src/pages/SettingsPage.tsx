import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { AppShell } from '../components/layout/AppShell';
import { PageHeader } from '../components/layout/PageHeader';
import { Card, CardHeader, CardTitle, CardContent } from '../components/ui/card';
import { Button } from '../components/ui/button';
import { Input } from '../components/ui/input';
import { useAuth } from '../contexts/AuthContext';
import { useToast } from '../contexts/ToastContext';
import { apiClient } from '../api/client';

export function SettingsPage() {
  const { t } = useTranslation();
  const { user } = useAuth();
  const { addToast } = useToast();
  const [displayName, setDisplayName] = useState(user?.display_name || '');
  const [saving, setSaving] = useState(false);
  const [currentPassword, setCurrentPassword] = useState('');
  const [newPassword, setNewPassword] = useState('');
  const [confirmPassword, setConfirmPassword] = useState('');
  const [changingPassword, setChangingPassword] = useState(false);

  const handleChangePassword = async () => {
    if (newPassword.length < 8) {
      addToast('error', t('register.passwordTooShort'));
      return;
    }
    if (newPassword !== confirmPassword) {
      addToast('error', t('register.passwordMismatch'));
      return;
    }
    setChangingPassword(true);
    try {
      await apiClient.changePassword(currentPassword || null, newPassword);
      addToast('success', t('settings.passwordChanged'));
      setCurrentPassword('');
      setNewPassword('');
      setConfirmPassword('');
    } catch (err) {
      addToast('error', err instanceof Error ? err.message : t('common.error'));
    } finally {
      setChangingPassword(false);
    }
  };

  const handleSave = async () => {
    setSaving(true);
    try {
      // TODO: 实现保存用户资料的 API
      addToast('success', t('common.save'));
    } catch {
      addToast('error', t('common.error'));
    } finally {
      setSaving(false);
    }
  };

  return (
    <AppShell>
      <PageHeader title={t('sidebar.settings')} />
      <div className="p-6 space-y-6">
        {/* 用户资料 */}
        <Card>
          <CardHeader>
            <CardTitle>{t('settings.profile')}</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <div>
              <label className="text-sm font-medium">{t('common.email')}</label>
              <Input value={user?.email || ''} disabled className="mt-1" />
            </div>
            <div>
              <label className="text-sm font-medium">{t('register.displayName')}</label>
              <Input
                value={displayName}
                onChange={(e) => setDisplayName(e.target.value)}
                className="mt-1"
              />
            </div>
            <Button onClick={handleSave} disabled={saving}>
              {saving ? t('common.saving') : t('common.save')}
            </Button>
          </CardContent>
        </Card>

        {/* 修改密码 */}
        <Card>
          <CardHeader>
            <CardTitle>{t('settings.changePassword')}</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <div>
              <label className="text-sm font-medium">{t('settings.currentPassword')}</label>
              <Input
                type="password"
                value={currentPassword}
                onChange={(e) => setCurrentPassword(e.target.value)}
                placeholder={t('settings.currentPasswordPlaceholder')}
                className="mt-1"
              />
            </div>
            <div>
              <label className="text-sm font-medium">{t('settings.newPassword')}</label>
              <Input
                type="password"
                value={newPassword}
                onChange={(e) => setNewPassword(e.target.value)}
                placeholder={t('settings.newPasswordPlaceholder')}
                minLength={8}
                className="mt-1"
              />
            </div>
            <div>
              <label className="text-sm font-medium">{t('register.confirmPassword')}</label>
              <Input
                type="password"
                value={confirmPassword}
                onChange={(e) => setConfirmPassword(e.target.value)}
                placeholder={t('register.confirmPasswordPlaceholder')}
                className="mt-1"
              />
            </div>
            <Button onClick={handleChangePassword} disabled={changingPassword || !newPassword}>
              {changingPassword ? t('common.saving') : t('settings.changePassword')}
            </Button>
          </CardContent>
        </Card>
      </div>
    </AppShell>
  );
}
