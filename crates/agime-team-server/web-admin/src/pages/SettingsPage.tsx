import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { AppShell } from '../components/layout/AppShell';
import { PageHeader } from '../components/layout/PageHeader';
import { Card, CardHeader, CardTitle, CardContent } from '../components/ui/card';
import { Button } from '../components/ui/button';
import { Input } from '../components/ui/input';
import { Textarea } from '../components/ui/textarea';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../components/ui/select';
import { useAuth } from '../contexts/AuthContext';
import { useToast } from '../contexts/ToastContext';
import { apiClient, type ChatPersonaProfile } from '../api/client';

export function SettingsPage() {
  const { t } = useTranslation();
  const { user, updateUserProfile, updateUserPreferences } = useAuth();
  const { addToast } = useToast();
  const [displayName, setDisplayName] = useState(user?.display_name || '');
  const [personaProfile, setPersonaProfile] = useState<ChatPersonaProfile>(user?.preferences?.chat_persona_profile || 'default');
  const [personaNote, setPersonaNote] = useState(user?.preferences?.chat_persona_note || '');
  const [saving, setSaving] = useState(false);
  const [currentPassword, setCurrentPassword] = useState('');
  const [newPassword, setNewPassword] = useState('');
  const [confirmPassword, setConfirmPassword] = useState('');
  const [changingPassword, setChangingPassword] = useState(false);

  useEffect(() => {
    setDisplayName(user?.display_name || '');
    setPersonaProfile(user?.preferences?.chat_persona_profile || 'default');
    setPersonaNote(user?.preferences?.chat_persona_note || '');
  }, [user?.display_name, user?.preferences?.chat_persona_profile, user?.preferences?.chat_persona_note]);

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
    const nextDisplayName = displayName.trim();
    const currentDisplayName = (user?.display_name || '').trim();
    if (!nextDisplayName) {
      addToast('error', t('register.displayNameRequired'));
      return;
    }
    setSaving(true);
    try {
      if (nextDisplayName !== currentDisplayName) {
        await updateUserProfile(nextDisplayName);
      }
      await updateUserPreferences({
        mobile_interaction_mode: user?.preferences?.mobile_interaction_mode || 'classic',
        chat_persona_profile: personaProfile,
        chat_persona_note: personaNote.trim() || null,
      });
      addToast('success', t('common.saved'));
    } catch (err) {
      addToast('error', err instanceof Error ? err.message : t('common.error'));
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

        <Card>
          <CardHeader>
            <CardTitle>对话风格</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <div>
              <label className="text-sm font-medium">默认风格</label>
              <Select value={personaProfile} onValueChange={(value) => setPersonaProfile(value as ChatPersonaProfile)}>
                <SelectTrigger className="mt-1">
                  <SelectValue placeholder="选择风格" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="default">自然默认</SelectItem>
                  <SelectItem value="warm">温暖亲和</SelectItem>
                  <SelectItem value="supportive">支持鼓励</SelectItem>
                  <SelectItem value="playful">轻松活泼</SelectItem>
                  <SelectItem value="direct">直接利落</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div>
              <label className="text-sm font-medium">补充说明</label>
              <Textarea
                value={personaNote}
                onChange={(e) => setPersonaNote(e.target.value)}
                placeholder="例如：说话直接一点，但保持有温度；先理解我，再给建议。"
                className="mt-1 min-h-[96px]"
              />
            </div>
            <p className="text-xs text-muted-foreground">
              这里只影响普通对话窗口，不影响 Agentify、portal 等功能型会话。
            </p>
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
